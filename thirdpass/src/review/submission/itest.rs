use super::*;

static ITEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let server_review = harness.assert_submitted_review_matches_server(&review)?;
    assert_eq!(
        common::config::Config::load()?.core.public_user_id,
        server_review.reviewer_details.public_user_id
    );
    Ok(())
}

#[test]
fn disk_pending_review_is_submitted_by_worker_scan() -> Result<()> {
    let _lock = ITEST_ENV_LOCK.lock().expect("itest env lock poisoned");
    let harness = RealServerHarness::new()?;
    let _env = harness.enter_client_environment()?;
    harness.client_config().dump()?;
    assert_eq!(common::config::Config::load()?.core.public_user_id, "");

    let review = harness.fixture_review()?;
    let pending_path = review::store_pending(&review)?;

    scan_pending_reviews_once_for_test(Duration::from_secs(0));

    assert!(!pending_path.exists());
    harness.assert_submitted_review_matches_server(&review)?;
    assert_eq!(common::config::Config::load()?.core.public_user_id, "");
    Ok(())
}

struct RealServerHarness {
    root: tempfile::TempDir,
    server: RealServer,
    package_name: String,
}

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
            agent_run_metrics: None,
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

    fn assert_submitted_review_matches_server(
        &self,
        review: &Review,
    ) -> Result<thirdpass_core::schema::ReviewRecord> {
        let submitted = local_submitted_review(&review.package.name)?;
        let server_review = self.server_pending_review(&review.package.name)?;
        assert_eq!(
            submitted.reviewer_details.public_user_id,
            server_review.reviewer_details.public_user_id
        );
        Ok(server_review)
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
}

struct RealServer {
    child: std::process::Child,
    api_base: String,
    log_path: PathBuf,
}

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

impl Drop for RealServer {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct ScopedEnv {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

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

fn local_submitted_review(package_name: &str) -> Result<Review> {
    review::fs::list_with_status()?
        .into_iter()
        .find(|stored| {
            stored.status == review::fs::ReviewStorageStatus::Submitted
                && stored.review.package.name == package_name
        })
        .map(|stored| stored.review)
        .context("expected submitted review in local storage")
}

fn fixture_package_manifest() -> thirdpass_core::schema::PackageManifest {
    thirdpass_core::schema::PackageManifest {
        files: vec![thirdpass_core::schema::PackageManifestFile {
            path: "src/index.js".to_string(),
            size_bytes: 120,
        }],
    }
}

fn pick_open_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

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

fn read_to_string_lossy(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| String::new())
}
