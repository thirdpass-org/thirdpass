use crate::common;
use crate::extension;
use crate::package;
use crate::peer;
use crate::registry;
use crate::review;
use crate::review::comment::Comment;
use anyhow::{format_err, Result};
use reqwest::StatusCode;
use thirdpass_core::schema as api;

pub type ReviewCandidate = api::ReviewCandidate;
pub type ReviewQuery = api::ReviewQuery;

#[derive(Debug, serde::Deserialize)]
struct ReviewSubmitResponse {
    id: String,
    public_user_id: String,
}

/// Server response metadata for an accepted review submission.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReviewSubmitResult {
    /// Server-assigned review id.
    pub id: String,
    /// Server-derived public user ID.
    pub public_user_id: String,
}

const API_KEY_CONFIG_COMMAND: &str = "thirdpass config set core.api-key <key>";

#[derive(Debug)]
struct AuthenticationRequiredError {
    status: StatusCode,
    body: String,
}

impl AuthenticationRequiredError {
    fn new(status: StatusCode, body: String) -> Self {
        Self { status, body }
    }
}

impl std::fmt::Display for AuthenticationRequiredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Login required by Thirdpass API ({}). Set your API key with: {}",
            self.status, API_KEY_CONFIG_COMMAND
        )?;
        let body = self.body.trim();
        if !body.is_empty() {
            write!(f, ". Server response: {}", body)?;
        }
        Ok(())
    }
}

impl std::error::Error for AuthenticationRequiredError {}

/// Return true when an API error means the user needs to authenticate.
pub(crate) fn is_authentication_required_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AuthenticationRequiredError>().is_some()
}

pub fn submit(
    review: &review::Review,
    package_manifest: &api::PackageManifest,
    config: &common::config::Config,
) -> Result<ReviewSubmitResult> {
    let registry = get_primary_registry(&review.package)?;
    let target = api::ReviewTarget {
        registry_host: registry.host_name.clone(),
        package_name: review.package.name.clone(),
        package_version: review.package.version.clone(),
        package_hash: review.package.package_hash.clone(),
    };
    let files = to_api_review_files(&review.targets);

    let payload = api::ReviewSubmission {
        target,
        files,
        package_manifest: Some(package_manifest.clone()),
        reviewer_details: review.reviewer_details.clone(),
        agent_summary: if review.agent_summary.trim().is_empty() {
            None
        } else {
            Some(review.agent_summary.clone())
        },
        overall_security_summary: None,
        overall_security_confidence: None,
    };

    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/reviews")?;
    let request = common::api::with_client_headers(client.post(url), config);
    let response = request.json(&payload).send()?;
    let response = require_success(response, "Failed to submit review")?;
    let response = response.json::<ReviewSubmitResponse>()?;
    Ok(ReviewSubmitResult {
        id: response.id,
        public_user_id: response.public_user_id.trim().to_string(),
    })
}

pub fn fetch(
    query: &api::ReviewQuery,
    config: &common::config::Config,
) -> Result<Vec<api::ReviewRecord>> {
    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/reviews")?;
    let request = common::api::with_client_headers(client.get(url), config);
    let response = request.query(&query).send()?;
    let response = require_success(response, "Failed to fetch reviews")?;
    let reviews = response.json::<Vec<api::ReviewRecord>>()?;
    Ok(reviews)
}

pub fn request_target(
    candidates: Vec<api::ReviewCandidate>,
    config: &common::config::Config,
) -> Result<Option<api::ReviewCandidate>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let supported_registry_hosts = supported_registry_hosts(config);
    let payload = review_request(candidates, config, supported_registry_hosts)?;
    let assignment = match post_review_request(&payload, config) {
        Ok(assignment) => assignment,
        Err(err) => {
            if is_authentication_required_error(&err) {
                return Err(err);
            }
            log::warn!("Failed to request target from API: {}", err);
            return Ok(None);
        }
    };
    Ok(assignment.target)
}

pub fn request_global_target(
    config: &common::config::Config,
    supported_registry_hosts: &[String],
) -> Result<Option<api::ReviewCandidate>> {
    let payload = review_request(Vec::new(), config, supported_registry_hosts.to_vec())?;
    Ok(post_review_request(&payload, config)?.target)
}

fn supported_registry_hosts(config: &common::config::Config) -> Vec<String> {
    config
        .extensions
        .registries
        .iter()
        .filter(|&(_registry_host, extension_name)| {
            config
                .extensions
                .enabled
                .get(extension_name)
                .copied()
                .unwrap_or(false)
        })
        .map(|(registry_host, _extension_name)| registry_host.clone())
        .collect()
}

pub(crate) fn supported_registry_hosts_for_filter(
    config: &common::config::Config,
    requested_registry_hosts: &[String],
) -> Result<Vec<String>> {
    if requested_registry_hosts.is_empty() {
        return Ok(supported_registry_hosts(config));
    }

    let requested_registry_hosts = normalized_registry_hosts(requested_registry_hosts)?;
    let mut supported = Vec::new();
    let mut unknown = Vec::new();
    let mut disabled = Vec::new();

    for registry_host in requested_registry_hosts {
        match config.extensions.registries.get(&registry_host) {
            Some(extension_name)
                if config
                    .extensions
                    .enabled
                    .get(extension_name)
                    .copied()
                    .unwrap_or(false) =>
            {
                supported.push(registry_host)
            }
            Some(extension_name) => {
                disabled.push(format!("{} ({})", registry_host, extension_name));
            }
            None => unknown.push(registry_host),
        }
    }

    if !unknown.is_empty() {
        return Err(format_err!(
            "Unknown registry requested: {}. Known registries: {}",
            unknown.join(", "),
            known_registry_hosts(config)
        ));
    }

    if !disabled.is_empty() {
        return Err(format_err!(
            "Requested registry is configured for a disabled extension: {}. Enable the extension with `thirdpass extension enable <name>`.",
            disabled.join(", ")
        ));
    }

    Ok(supported)
}

fn normalized_registry_hosts(registry_hosts: &[String]) -> Result<Vec<String>> {
    let mut normalized = std::collections::BTreeSet::new();
    for registry_host in registry_hosts {
        let registry_host = registry_host.trim().to_ascii_lowercase();
        if registry_host.is_empty() {
            return Err(format_err!("Registry cannot be empty."));
        }
        normalized.insert(registry_host);
    }
    Ok(normalized.into_iter().collect())
}

fn known_registry_hosts(config: &common::config::Config) -> String {
    if config.extensions.registries.is_empty() {
        return "none".to_string();
    }
    config
        .extensions
        .registries
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn review_target_policies(
    config: &common::config::Config,
) -> Result<std::collections::BTreeMap<String, thirdpass_core::extension::ReviewTargetPolicy>> {
    let extension_names = enabled_extension_names_for_registries(config);
    if extension_names.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }

    let mut policies = std::collections::BTreeMap::new();
    for extension in extension::manage::get_enabled(&extension_names, config)? {
        let extension_name = extension.name();
        let policy = extension.review_target_policy();
        for registry_host in extension.registries() {
            if config.extensions.registries.get(&registry_host) == Some(&extension_name)
                && config
                    .extensions
                    .enabled
                    .get(&extension_name)
                    .copied()
                    .unwrap_or(false)
            {
                policies.insert(registry_host, policy.clone());
            }
        }
    }
    Ok(policies)
}

fn enabled_extension_names_for_registries(
    config: &common::config::Config,
) -> std::collections::BTreeSet<String> {
    config
        .extensions
        .registries
        .values()
        .filter(|extension_name| {
            config
                .extensions
                .enabled
                .get(*extension_name)
                .copied()
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

fn review_request(
    candidates: Vec<api::ReviewCandidate>,
    config: &common::config::Config,
    supported_registry_hosts: Vec<String>,
) -> Result<api::ReviewRequest> {
    let mut review_target_policies = review_target_policies(config)?;
    let supported_registry_hosts_set: std::collections::BTreeSet<_> =
        supported_registry_hosts.iter().cloned().collect();
    review_target_policies
        .retain(|registry_host, _policy| supported_registry_hosts_set.contains(registry_host));

    Ok(api::ReviewRequest {
        candidates,
        supported_registry_hosts,
        review_target_policies,
    })
}

fn post_review_request(
    payload: &api::ReviewRequest,
    config: &common::config::Config,
) -> Result<api::ReviewAssignment> {
    let client = reqwest::blocking::Client::new();
    let base = crate::common::api::normalize_base(&config.core.api_base)?;
    let url = crate::common::api::join(&base, "v1/review-requests")?;
    let request = common::api::with_client_headers(client.post(url), config);
    let response = request.json(&payload).send()?;
    let response = require_success(response, "Review request failed")?;
    Ok(response.json::<api::ReviewAssignment>()?)
}

fn require_success(
    response: reqwest::blocking::Response,
    failure_message: &'static str,
) -> Result<reqwest::blocking::Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if is_authentication_required_status(status) {
        return Err(AuthenticationRequiredError::new(status, body).into());
    }

    Err(format_err!("{} ({}): {}", failure_message, status, body))
}

fn is_authentication_required_status(status: StatusCode) -> bool {
    matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
}

pub fn store_records(
    records: Vec<api::ReviewRecord>,
    config: &common::config::Config,
) -> Result<usize> {
    let mut stored = 0;
    for record in records {
        if record.reviewer_details.public_user_id == config.core.public_user_id {
            continue;
        }
        store_record(record, config)?;
        stored += 1;
    }
    Ok(stored)
}

fn store_record(record: api::ReviewRecord, config: &common::config::Config) -> Result<()> {
    let api::ReviewRecord {
        target,
        reviewer_details,
        files,
        overall_security_summary,
        overall_security_confidence,
        agent_summary,
        ..
    } = record;
    let registry = build_registry(&target)?;
    let package = build_package(&target, &registry);
    let peer = peer::public_user_peer(&reviewer_details.public_user_id, &config.core.api_base)?;
    let targets = files
        .into_iter()
        .map(|file| {
            let api::ReviewFile {
                file_path,
                file_hash,
                summary,
                security_summary,
                confidence,
                comments,
            } = file;
            let comments = comments
                .into_iter()
                .map(|comment| from_remote_comment(comment, &file_path))
                .collect::<std::collections::BTreeSet<_>>();
            review::ReviewTarget {
                file_path: std::path::PathBuf::from(file_path),
                file_hash,
                agent_summary: summary,
                security_summary,
                confidence,
                comments,
            }
        })
        .collect::<Vec<_>>();

    let review = review::Review {
        id: 0,
        peer,
        package,
        targets,
        reviewer_details,
        agent_summary: agent_summary.unwrap_or_default(),
        overall_security_summary,
        overall_security_confidence,
    };

    review::store_submitted(&review)?;
    Ok(())
}

fn to_api_review_files(targets: &[review::ReviewTarget]) -> Vec<api::ReviewFile> {
    targets
        .iter()
        .map(|target| api::ReviewFile {
            file_path: target.file_path.display().to_string(),
            file_hash: target.file_hash.clone(),
            summary: target.agent_summary.clone(),
            security_summary: target.security_summary,
            confidence: target.confidence,
            comments: target
                .comments
                .iter()
                .cloned()
                .map(to_remote_comment)
                .collect(),
        })
        .collect()
}

fn to_remote_comment(comment: Comment) -> api::ReviewComment {
    api::ReviewComment {
        comment: comment.message,
        security: comment.security,
        complexity: comment.complexity,
        selection: comment.selection,
    }
}

fn from_remote_comment(comment: api::ReviewComment, file_path: &str) -> Comment {
    Comment {
        id: 0,
        security: comment.security,
        complexity: comment.complexity,
        path: std::path::PathBuf::from(file_path),
        message: comment.comment,
        selection: comment.selection,
    }
}

fn get_primary_registry(package: &package::Package) -> Result<&registry::Registry> {
    let registry = package
        .registries
        .iter()
        .next()
        .ok_or(format_err!("Package does not have associated registries."))?;
    Ok(registry)
}

fn build_registry(target: &api::ReviewTarget) -> Result<registry::Registry> {
    let host = target.registry_host.as_str();
    let human_url = url::Url::parse(&format!("https://{}/", host))?;
    let artifact_url = url::Url::parse(&format!("https://{}/artifact", host))?;
    Ok(registry::Registry {
        id: 0,
        host_name: target.registry_host.clone(),
        human_url,
        artifact_url,
    })
}

fn build_package(target: &api::ReviewTarget, registry: &registry::Registry) -> package::Package {
    package::Package {
        id: 0,
        name: target.package_name.clone(),
        version: target.package_version.clone(),
        registries: maplit::btreeset! { registry.clone() },
        package_hash: target.package_hash.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_api_review_files_preserves_file_hash() {
        let file_hash = api::FileHash::blake3("abc123");
        let targets = vec![review::ReviewTarget {
            file_path: std::path::PathBuf::from("index.js"),
            file_hash: Some(file_hash.clone()),
            agent_summary: Some("Reviewed the file.".to_string()),
            security_summary: Some(api::SecuritySummary::Low),
            confidence: Some(api::ReviewConfidence::High),
            comments: std::collections::BTreeSet::new(),
        }];

        let files = to_api_review_files(&targets);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "index.js");
        assert_eq!(files[0].file_hash, Some(file_hash));
        assert_eq!(files[0].summary.as_deref(), Some("Reviewed the file."));
        assert_eq!(files[0].security_summary, Some(api::SecuritySummary::Low));
        assert_eq!(files[0].confidence, Some(api::ReviewConfidence::High));
    }

    #[test]
    fn review_submit_response_reads_public_user_id() {
        let response: ReviewSubmitResponse =
            serde_json::from_str(r#"{"id":"rev_1","public_user_id":"user-1"}"#)
                .expect("failed to parse response");

        assert_eq!(response.id, "rev_1");
        assert_eq!(response.public_user_id, "user-1");
    }

    #[test]
    fn authentication_required_error_tells_user_how_to_configure_api_key() {
        let err: anyhow::Error = AuthenticationRequiredError::new(
            StatusCode::UNAUTHORIZED,
            "missing bearer token".to_string(),
        )
        .into();

        assert!(is_authentication_required_error(&err));
        assert!(err
            .to_string()
            .contains("thirdpass config set core.api-key <key>"));
        assert!(err.to_string().contains("missing bearer token"));
    }

    #[test]
    fn forbidden_status_is_treated_as_authentication_required() {
        assert!(is_authentication_required_status(StatusCode::FORBIDDEN));
    }

    #[test]
    fn supported_registry_hosts_uses_enabled_extensions() {
        let mut config = common::config::Config::default();
        config.extensions.enabled.insert("js".to_string(), true);
        config.extensions.enabled.insert("rs".to_string(), false);
        config
            .extensions
            .registries
            .insert("npmjs.com".to_string(), "js".to_string());
        config
            .extensions
            .registries
            .insert("crates.io".to_string(), "rs".to_string());

        assert_eq!(supported_registry_hosts(&config), vec!["npmjs.com"]);
    }

    #[test]
    fn supported_registry_hosts_for_filter_limits_to_requested_enabled_registries() -> Result<()> {
        let mut config = common::config::Config::default();
        config.extensions.enabled.insert("js".to_string(), true);
        config.extensions.enabled.insert("rs".to_string(), true);
        config
            .extensions
            .registries
            .insert("npmjs.com".to_string(), "js".to_string());
        config
            .extensions
            .registries
            .insert("crates.io".to_string(), "rs".to_string());

        let requested = vec!["NPMJS.COM".to_string(), "npmjs.com".to_string()];

        assert_eq!(
            supported_registry_hosts_for_filter(&config, &requested)?,
            vec!["npmjs.com"]
        );
        Ok(())
    }

    #[test]
    fn supported_registry_hosts_for_filter_rejects_unknown_registries() {
        let mut config = common::config::Config::default();
        config.extensions.enabled.insert("js".to_string(), true);
        config
            .extensions
            .registries
            .insert("npmjs.com".to_string(), "js".to_string());

        let requested = vec!["crates.io".to_string()];
        let err = supported_registry_hosts_for_filter(&config, &requested).unwrap_err();

        assert!(err.to_string().contains("Unknown registry requested"));
        assert!(err.to_string().contains("npmjs.com"));
    }

    #[test]
    fn supported_registry_hosts_for_filter_rejects_disabled_registries() {
        let mut config = common::config::Config::default();
        config.extensions.enabled.insert("rs".to_string(), false);
        config
            .extensions
            .registries
            .insert("crates.io".to_string(), "rs".to_string());

        let requested = vec!["crates.io".to_string()];
        let err = supported_registry_hosts_for_filter(&config, &requested).unwrap_err();

        assert!(err
            .to_string()
            .contains("configured for a disabled extension"));
        assert!(err.to_string().contains("crates.io (rs)"));
    }
}
