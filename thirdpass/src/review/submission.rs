use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::common;
use crate::peer;
use crate::review::{self, Review};

const PENDING_REVIEW_SCAN_INTERVAL: Duration = Duration::from_secs(30);
const PENDING_REVIEW_SCAN_MIN_AGE: Duration = Duration::from_secs(30);
const PENDING_REVIEW_RETRY_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Worker handle used to submit saved reviews without blocking review work.
#[derive(Clone)]
pub(crate) struct Submitter {
    sender: mpsc::Sender<Job>,
}

/// Completion ticket for one queued review submission.
pub(crate) struct Ticket {
    package_label: String,
    receiver: mpsc::Receiver<Status>,
}

struct Job {
    pending_path: PathBuf,
    review: Review,
    package_manifest: thirdpass_core::schema::PackageManifest,
    config: common::config::Config,
    result_tx: Option<mpsc::Sender<Status>>,
}

enum Status {
    Submitted {
        package_label: String,
        public_user_id: String,
    },
    Failed {
        package_label: String,
        error: String,
    },
}

/// Summary from waiting for queued review submissions.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct WaitSummary {
    /// Number of queued submissions accepted by the API.
    pub(crate) submitted: usize,
    /// Number of queued submissions that failed and remain pending.
    pub(crate) failed: usize,
}

struct Worker {
    receiver: mpsc::Receiver<Job>,
    next_retry_at: BTreeMap<PathBuf, Instant>,
}

impl Worker {
    fn run(mut self) {
        loop {
            match self.receiver.recv_timeout(PENDING_REVIEW_SCAN_INTERVAL) {
                Ok(job) => self.run_job(job),
                Err(mpsc::RecvTimeoutError::Timeout) => self.scan_pending_reviews(),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn run_job(&mut self, job: Job) {
        let path = job.pending_path.clone();
        let success = run_submission_job(job);
        if success {
            self.next_retry_at.remove(&path);
        } else {
            self.next_retry_at
                .insert(path, Instant::now() + PENDING_REVIEW_RETRY_COOLDOWN);
        }
    }

    fn scan_pending_reviews(&mut self) {
        self.scan_pending_reviews_older_than(PENDING_REVIEW_SCAN_MIN_AGE);
    }

    fn scan_pending_reviews_older_than(&mut self, min_age: Duration) {
        let paths = match pending_review_paths_older_than(min_age) {
            Ok(paths) => paths,
            Err(error) => {
                log::warn!("Failed to scan pending reviews for retry: {error}");
                return;
            }
        };
        let now = Instant::now();

        for path in paths {
            if path_in_cooldown(&self.next_retry_at, &path, now) {
                continue;
            }

            match build_scanned_submission_job(&path) {
                Ok(job) => self.run_job(job),
                Err(error) => {
                    log::debug!(
                        "Skipping pending review retry for {}: {}",
                        path.display(),
                        error
                    );
                }
            }
        }
    }
}

impl Submitter {
    /// Start a background submission worker for this command invocation.
    pub(crate) fn start() -> Result<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("thirdpass-review-submitter".to_string())
            .spawn(move || {
                Worker {
                    receiver,
                    next_retry_at: BTreeMap::new(),
                }
                .run();
            })
            .context("Failed to start review submission worker.")?;
        Ok(Self { sender })
    }

    /// Queue a pending review for background submission.
    pub(crate) fn submit(
        &self,
        pending_path: PathBuf,
        review: Review,
        package_manifest: thirdpass_core::schema::PackageManifest,
        config: common::config::Config,
    ) -> Ticket {
        let package_label = package_target_label(&review);
        let (result_tx, receiver) = mpsc::channel();
        let job = Job {
            pending_path,
            review,
            package_manifest,
            config,
            result_tx: Some(result_tx.clone()),
        };

        if let Err(error) = self.sender.send(job) {
            let _ = result_tx.send(Status::Failed {
                package_label: package_label.clone(),
                error: format!("submission worker is unavailable: {error}"),
            });
        }

        Ticket {
            package_label,
            receiver,
        }
    }
}

impl Ticket {
    fn wait(self) -> Status {
        match self.receiver.recv() {
            Ok(status) => status,
            Err(error) => Status::Failed {
                package_label: self.package_label,
                error: format!("submission worker stopped before reporting a result: {error}"),
            },
        }
    }
}

fn run_submission_job(job: Job) -> bool {
    let package_label = package_target_label(&job.review);
    let result_tx = job.result_tx.clone();
    let result = submit_review_job(job);
    match (result, result_tx) {
        (Ok(public_user_id), Some(result_tx)) => {
            let _ = result_tx.send(Status::Submitted {
                package_label,
                public_user_id,
            });
            true
        }
        (Ok(_public_user_id), None) => {
            log::info!("Submitted pending review for {package_label}.");
            true
        }
        (Err(error), Some(result_tx)) => {
            let _ = result_tx.send(Status::Failed {
                package_label,
                error: error.to_string(),
            });
            false
        }
        (Err(error), None) => {
            log::warn!("Failed to submit pending review for {package_label}: {error}");
            false
        }
    }
}

fn submit_review_job(job: Job) -> Result<String> {
    let submit_result = review::remote::submit(&job.review, &job.package_manifest, &job.config)?;

    let mut submitted_review = job.review;
    let public_user_id_changed = apply_public_user_id_to_review(
        &mut submitted_review,
        &submit_result.public_user_id,
        &job.config.core.api_base,
    )?;
    finish_submitted_review(&submitted_review, &job.pending_path, public_user_id_changed)?;

    Ok(submit_result.public_user_id)
}

fn build_scanned_submission_job(path: &Path) -> Result<Job> {
    let mut review = read_pending_review(path)?;
    review.overall_security_summary = review::overall_security_summary(&review)?;
    Ok(Job {
        pending_path: path.to_path_buf(),
        review,
        package_manifest: scanned_retry_package_manifest(),
        config: common::config::Config::load()?,
        result_tx: None,
    })
}

/// Package manifest used when retrying old pending reviews found by scanning.
fn scanned_retry_package_manifest() -> thirdpass_core::schema::PackageManifest {
    // Scanner retries run without an active package workspace. The review JSON
    // carries the findings; file inventory is omitted for this retry path.
    thirdpass_core::schema::PackageManifest { files: Vec::new() }
}

fn read_pending_review(path: &Path) -> Result<Review> {
    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    Ok(serde_json::from_reader(reader)?)
}

fn pending_review_paths_older_than(min_age: Duration) -> Result<Vec<PathBuf>> {
    let paths = common::fs::DataPaths::new()?;
    pending_review_paths_older_than_in(&paths.pending_reviews_directory, min_age)
}

fn pending_review_paths_older_than_in(directory: &Path, min_age: Duration) -> Result<Vec<PathBuf>> {
    let mut pending = Vec::new();
    collect_old_pending_review_paths(directory, min_age, &mut pending)?;
    pending.sort();
    Ok(pending)
}

fn collect_old_pending_review_paths(
    directory: &Path,
    min_age: Duration,
    pending: &mut Vec<PathBuf>,
) -> Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_old_pending_review_paths(&path, min_age, pending)?;
            continue;
        }
        if !is_review_json_path(&path) {
            continue;
        }
        if file_age(&path).map(|age| age >= min_age).unwrap_or(false) {
            pending.push(path);
        }
    }
    Ok(())
}

fn is_review_json_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("review-"))
        .unwrap_or(false)
        && path.extension().and_then(|extension| extension.to_str()) == Some("json")
}

fn file_age(path: &Path) -> Option<Duration> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
}

fn path_in_cooldown(next_retry_at: &BTreeMap<PathBuf, Instant>, path: &Path, now: Instant) -> bool {
    next_retry_at
        .get(path)
        .map(|retry_at| *retry_at > now)
        .unwrap_or(false)
}

/// Wait for one queued submission and persist server identity updates.
pub(crate) fn wait_for_submission(ticket: Ticket) -> Result<bool> {
    let summary = wait_for_submissions(vec![ticket])?;
    Ok(summary.submitted == 1)
}

/// Wait for queued submissions and persist server identity updates.
pub(crate) fn wait_for_submissions(tickets: Vec<Ticket>) -> Result<WaitSummary> {
    let mut summary = WaitSummary::default();
    if tickets.len() > 1 {
        println!(
            "Waiting for {} review submission{}.",
            tickets.len(),
            plural_suffix(tickets.len())
        );
    }

    for ticket in tickets {
        match ticket.wait() {
            Status::Submitted {
                package_label,
                public_user_id,
            } => {
                persist_public_user_id(&public_user_id)?;
                println!("Review submitted: {}.", package_label);
                summary.submitted += 1;
            }
            Status::Failed {
                package_label,
                error,
            } => {
                report_submission_failure_message(&package_label, &error);
                summary.failed += 1;
            }
        }
    }

    if summary.submitted + summary.failed > 1 {
        println!(
            "Review submission summary: {} submitted, {} pending.",
            summary.submitted, summary.failed
        );
    }

    Ok(summary)
}

fn persist_public_user_id(public_user_id: &str) -> Result<()> {
    let public_user_id = public_user_id.trim();
    if public_user_id.is_empty() {
        return Ok(());
    }

    let mut config = common::config::Config::load()?;
    if config.core.public_user_id != public_user_id {
        config.core.public_user_id = public_user_id.to_string();
        config.dump()?;
    }
    Ok(())
}

fn apply_public_user_id_to_review(
    review: &mut Review,
    public_user_id: &str,
    api_base: &str,
) -> Result<bool> {
    let public_user_id = public_user_id.trim();
    if public_user_id.is_empty() {
        return Ok(false);
    }

    let changed = review.reviewer_details.public_user_id != public_user_id;
    review.reviewer_details.public_user_id = public_user_id.to_string();
    review.peer = peer::public_user_peer(public_user_id, api_base)?;
    Ok(changed)
}

fn finish_submitted_review(
    review: &Review,
    pending_path: &PathBuf,
    rewrite_contents: bool,
) -> Result<()> {
    if rewrite_contents {
        review::store_submitted(review)?;
        remove_pending_review_if_present(pending_path)?;
    } else {
        promote_pending_review_if_present(review, pending_path)?;
    }
    Ok(())
}

fn remove_pending_review_if_present(pending_path: &Path) -> Result<()> {
    match std::fs::remove_file(pending_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn promote_pending_review_if_present(review: &Review, pending_path: &PathBuf) -> Result<()> {
    match review::promote_pending(review, pending_path) {
        Ok(_) => Ok(()),
        Err(error) if is_not_found_error(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn is_not_found_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .map(|error| error.kind() == std::io::ErrorKind::NotFound)
        .unwrap_or(false)
}

fn report_submission_failure_message(package_label: &str, message: &str) {
    eprintln!(
        "Review submission failed for {}; review remains saved locally for retry: {}",
        package_label, message
    );
    log::warn!(
        "Failed to submit review for {}; review remains saved locally for retry: {}",
        package_label,
        message
    );
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn package_target_label(review: &Review) -> String {
    format!("{}@{}", review.package.name, review.package.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "itest")]
    static ITEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn is_review_json_path_accepts_only_review_json_files() {
        assert!(is_review_json_path(Path::new("review-abc.json")));
        assert!(is_review_json_path(Path::new("nested/review-abc.json")));
        assert!(!is_review_json_path(Path::new("review-abc.tmp")));
        assert!(!is_review_json_path(Path::new("other.json")));
        assert!(!is_review_json_path(Path::new(".gitkeep")));
    }

    #[test]
    fn pending_review_paths_filter_by_name_and_age() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let nested = temp.path().join("package");
        std::fs::create_dir(&nested)?;
        let review_path = nested.join("review-ready.json");
        let other_json_path = nested.join("other.json");
        let temp_path = nested.join("review-ready.tmp");
        std::fs::write(&review_path, b"{}")?;
        std::fs::write(&other_json_path, b"{}")?;
        std::fs::write(&temp_path, b"{}")?;

        let pending = pending_review_paths_older_than_in(temp.path(), Duration::from_secs(0))?;
        assert_eq!(pending, vec![review_path]);

        let too_young =
            pending_review_paths_older_than_in(temp.path(), Duration::from_secs(24 * 60 * 60))?;
        assert!(too_young.is_empty());
        Ok(())
    }

    #[test]
    fn scanned_submission_job_rejects_invalid_json_before_config_load() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let review_path = temp.path().join("review-invalid.json");
        std::fs::write(&review_path, b"{")?;

        let error = match build_scanned_submission_job(&review_path) {
            Ok(_) => panic!("expected invalid pending JSON to be skipped"),
            Err(error) => error,
        };

        assert!(error.downcast_ref::<serde_json::Error>().is_some());
        Ok(())
    }

    #[test]
    fn path_in_cooldown_only_blocks_future_retry_times() {
        let mut next_retry_at = BTreeMap::new();
        let path = PathBuf::from("review-abc.json");
        let now = Instant::now();

        assert!(!path_in_cooldown(&next_retry_at, &path, now));

        next_retry_at.insert(path.clone(), now + Duration::from_secs(60));
        assert!(path_in_cooldown(&next_retry_at, &path, now));
        assert!(!path_in_cooldown(
            &next_retry_at,
            &path,
            now + Duration::from_secs(61)
        ));
    }

    #[cfg(feature = "itest")]
    #[test]
    fn queued_review_submission_saves_server_result_locally() -> Result<()> {
        let _lock = ITEST_ENV_LOCK.lock().expect("itest env lock poisoned");
        let harness = RealServerHarness::new()?;
        let _env = harness.enter_client_environment()?;
        let config = harness.client_config();
        config.dump()?;

        let review = harness.fixture_review()?;
        let pending_path = review::store_pending(&review)?;
        let submitter = Submitter::start()?;
        let ticket = submitter.submit(
            pending_path.clone(),
            review.clone(),
            fixture_package_manifest(),
            config,
        );

        assert!(wait_for_submission(ticket)?);
        drop(submitter);

        assert!(!pending_path.exists());
        let stored = review::fs::list_with_status()?;
        let submitted = stored
            .iter()
            .find(|stored| {
                stored.status == review::fs::ReviewStorageStatus::Submitted
                    && stored.review.package.name == review.package.name
            })
            .expect("expected submitted review in local storage");
        let public_user_id = common::config::Config::load()?.core.public_user_id;
        assert!(!public_user_id.is_empty());
        assert_eq!(
            submitted.review.reviewer_details.public_user_id,
            public_user_id
        );
        harness.assert_server_has_pending_review(&review.package.name, &public_user_id)?;
        Ok(())
    }

    #[cfg(feature = "itest")]
    #[test]
    fn disk_pending_review_is_submitted_by_worker_scan() -> Result<()> {
        let _lock = ITEST_ENV_LOCK.lock().expect("itest env lock poisoned");
        let harness = RealServerHarness::new()?;
        let _env = harness.enter_client_environment()?;
        harness.client_config().dump()?;

        let review = harness.fixture_review()?;
        let pending_path = review::store_pending(&review)?;
        let (_sender, receiver) = mpsc::channel();
        let mut worker = Worker {
            receiver,
            next_retry_at: BTreeMap::new(),
        };

        worker.scan_pending_reviews_older_than(Duration::from_secs(0));

        assert!(!pending_path.exists());
        let stored = review::fs::list_with_status()?;
        let submitted = stored
            .iter()
            .find(|stored| {
                stored.status == review::fs::ReviewStorageStatus::Submitted
                    && stored.review.package.name == review.package.name
            })
            .expect("expected scanned review to be submitted locally");
        let server_review = harness.server_pending_review(&review.package.name)?;
        assert_eq!(
            submitted.review.reviewer_details.public_user_id,
            server_review.reviewer_details.public_user_id
        );
        Ok(())
    }

    #[cfg(feature = "itest")]
    struct RealServerHarness {
        root: tempfile::TempDir,
        server: RealServer,
        package_name: String,
    }

    #[cfg(feature = "itest")]
    impl RealServerHarness {
        fn new() -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix("thirdpass-submission-itest-")
                .tempdir()
                .context("failed to create submission itest temp dir")?;
            let api_base = format!("http://127.0.0.1:{}", pick_open_port()?);
            let server = RealServer::start(root.path(), &api_base)?;
            let package_name = format!("submission-itest-{}", uuid::Uuid::new_v4().to_simple());

            Ok(Self {
                root,
                server,
                package_name,
            })
        }

        fn enter_client_environment(&self) -> Result<ScopedEnv> {
            let client_root = self.root.path().join("client");
            std::fs::create_dir_all(&client_root)?;
            Ok(ScopedEnv::set(&[
                ("HOME", client_root.join("home")),
                ("XDG_CONFIG_HOME", client_root.join("xdg-config")),
                ("XDG_DATA_HOME", client_root.join("xdg-data")),
            ]))
        }

        fn client_config(&self) -> common::config::Config {
            let mut config = common::config::Config::default();
            config.core.api_base = self.server.api_base.clone();
            config.core.client_id = "8f4e77e8-65b9-4d2a-a0bd-441f97a9350f".to_string();
            config
        }

        fn fixture_review(&self) -> Result<Review> {
            let registry = crate::registry::Registry {
                id: 0,
                host_name: "fixture.registry".to_string(),
                human_url: url::Url::parse("https://fixture.registry/package")?,
                artifact_url: url::Url::parse("https://fixture.registry/package.tgz")?,
            };
            let mut registries = std::collections::BTreeSet::new();
            registries.insert(registry);
            let package = crate::package::Package {
                id: 0,
                name: self.package_name.clone(),
                version: "1.0.0".to_string(),
                registries,
                package_hash: "sha256:submission-itest".to_string(),
            };
            let mut comments = std::collections::BTreeSet::new();
            comments.insert(crate::review::comment::Comment {
                id: 0,
                security: review::Priority::Medium,
                complexity: review::Priority::Low,
                path: PathBuf::from("src/index.js"),
                message: "Fixture finding for the submission worker.".to_string(),
                selection: None,
            });
            let targets = vec![review::ReviewTarget {
                file_path: PathBuf::from("src/index.js"),
                file_hash: None,
                agent_summary: Some("Fixture target summary.".to_string()),
                security_summary: Some(review::SecuritySummary::Medium),
                confidence: None,
                comments,
            }];

            Ok(Review {
                id: 0,
                peer: crate::peer::Peer::default(),
                package,
                targets,
                reviewer_details: review::ReviewerDetails {
                    public_user_id: "local-user-before-submit".to_string(),
                    agent_name: "itest-agent".to_string(),
                    agent_model: "itest-model".to_string(),
                    agent_reasoning_effort: "default".to_string(),
                    review_strategy: "package-release/v1".to_string(),
                    review_scope: review::ReviewScope::TargetFileFull,
                    created_at: "2026-06-11T00:00:00Z".to_string(),
                    thirdpass_version: "itest".to_string(),
                },
                agent_summary: "Fixture review summary.".to_string(),
                overall_security_summary: review::SecuritySummary::Medium,
                overall_security_confidence: None,
            })
        }

        fn server_pending_review(
            &self,
            package_name: &str,
        ) -> Result<thirdpass_core::schema::ReviewRecord> {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .context("failed to build pending review client")?;
            let response = client
                .get(format!("{}/v1/admin/reviews/pending", self.server.api_base))
                .bearer_auth(RealServer::ADMIN_KEY)
                .send()
                .context("failed to fetch server pending reviews")?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                anyhow::bail!("pending review query failed ({status}): {body}");
            }
            let reviews = response
                .json::<Vec<thirdpass_core::schema::ReviewRecord>>()
                .context("failed to parse pending review response")?;
            reviews
                .into_iter()
                .find(|review| review.target.package_name == package_name)
                .context("expected server pending reviews to include fixture")
        }

        fn assert_server_has_pending_review(
            &self,
            package_name: &str,
            public_user_id: &str,
        ) -> Result<()> {
            let review = self.server_pending_review(package_name)?;
            assert_eq!(review.reviewer_details.public_user_id, public_user_id);
            Ok(())
        }
    }

    #[cfg(feature = "itest")]
    struct RealServer {
        child: std::process::Child,
        api_base: String,
        log_path: PathBuf,
    }

    #[cfg(feature = "itest")]
    impl RealServer {
        const ADMIN_KEY: &'static str = "thirdpass-submission-itest-admin-key";

        fn start(root: &Path, api_base: &str) -> Result<Self> {
            let server_binary = ensure_server_binary()?;
            let bind_addr = api_base
                .strip_prefix("http://")
                .context("itest server api base must use http")?;
            let log_path = root.join("server.log");
            let stdout = std::fs::File::create(&log_path).context("failed to create server log")?;
            let stderr = std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .context("failed to open server log")?;
            let child = std::process::Command::new(server_binary)
                .env("BIND_ADDR", bind_addr)
                .env("THIRDPASS_SERVER_DATA_DIR", root.join("server-data"))
                .env("THIRDPASS_ADMIN_KEY", Self::ADMIN_KEY)
                .env("THIRDPASS_PUBLIC_USER_ID_SECRET", "submission-itest-secret")
                .stdout(std::process::Stdio::from(stdout))
                .stderr(std::process::Stdio::from(stderr))
                .spawn()
                .context("failed to start thirdpass-server")?;
            let mut server = Self {
                child,
                api_base: api_base.to_string(),
                log_path,
            };
            server.wait_until_ready()?;
            Ok(server)
        }

        fn wait_until_ready(&mut self) -> Result<()> {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .context("failed to build server health client")?;
            let deadline = Instant::now() + Duration::from_secs(30);

            while Instant::now() < deadline {
                if let Some(status) = self.child.try_wait()? {
                    anyhow::bail!(
                        "thirdpass-server exited early with {status}\n{}",
                        read_to_string_lossy(&self.log_path)
                    );
                }

                match client.get(format!("{}/healthz", self.api_base)).send() {
                    Ok(response) if response.status().is_success() => return Ok(()),
                    _ => std::thread::sleep(Duration::from_millis(100)),
                }
            }

            anyhow::bail!(
                "timed out waiting for thirdpass-server\n{}",
                read_to_string_lossy(&self.log_path)
            )
        }
    }

    #[cfg(feature = "itest")]
    impl Drop for RealServer {
        fn drop(&mut self) {
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    #[cfg(feature = "itest")]
    struct ScopedEnv {
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    #[cfg(feature = "itest")]
    impl ScopedEnv {
        fn set(values: &[(&'static str, PathBuf)]) -> Self {
            let previous = values
                .iter()
                .map(|(key, _value)| (*key, std::env::var_os(key)))
                .collect::<Vec<_>>();
            for (key, value) in values {
                std::env::set_var(key, value);
            }
            Self { previous }
        }
    }

    #[cfg(feature = "itest")]
    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (key, value) in self.previous.iter().rev() {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[cfg(feature = "itest")]
    fn fixture_package_manifest() -> thirdpass_core::schema::PackageManifest {
        thirdpass_core::schema::PackageManifest {
            files: vec![thirdpass_core::schema::PackageManifestFile {
                path: "src/index.js".to_string(),
                size_bytes: 120,
            }],
        }
    }

    #[cfg(feature = "itest")]
    fn pick_open_port() -> Result<u16> {
        Ok(std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port())
    }

    #[cfg(feature = "itest")]
    fn ensure_server_binary() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("THIRDPASS_SERVER_BIN") {
            return Ok(path.into());
        }

        let manifest = server_manifest_path()?;
        let status = std::process::Command::new("cargo")
            .arg("build")
            .arg("--manifest-path")
            .arg(&manifest)
            .arg("--bin")
            .arg("thirdpass-server")
            .status()
            .context("failed to build thirdpass-server")?;
        if !status.success() {
            anyhow::bail!("cargo build for thirdpass-server failed with {status}");
        }

        Ok(server_target_dir(&manifest)?
            .join("debug")
            .join(format!("thirdpass-server{}", std::env::consts::EXE_SUFFIX)))
    }

    #[cfg(feature = "itest")]
    fn server_manifest_path() -> Result<PathBuf> {
        let client_crate = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dev_root = client_crate
            .parent()
            .and_then(Path::parent)
            .context("failed to resolve thirdpass-dev root")?;
        let manifest = dev_root.join("thirdpass-server").join("Cargo.toml");
        if !manifest.is_file() {
            anyhow::bail!(
                "expected thirdpass-server manifest at {}",
                manifest.display()
            );
        }
        Ok(manifest)
    }

    #[cfg(feature = "itest")]
    fn server_target_dir(manifest: &Path) -> Result<PathBuf> {
        let output = std::process::Command::new("cargo")
            .arg("metadata")
            .arg("--no-deps")
            .arg("--format-version")
            .arg("1")
            .arg("--manifest-path")
            .arg(manifest)
            .output()
            .context("failed to read thirdpass-server cargo metadata")?;
        if !output.status.success() {
            anyhow::bail!(
                "cargo metadata for thirdpass-server failed\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let target_directory = metadata
            .get("target_directory")
            .and_then(|value| value.as_str())
            .context("cargo metadata did not include target_directory")?;
        Ok(PathBuf::from(target_directory))
    }

    #[cfg(feature = "itest")]
    fn read_to_string_lossy(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|_| String::new())
    }
}
