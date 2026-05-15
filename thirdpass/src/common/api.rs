use anyhow::{format_err, Result};
use reqwest::header::AUTHORIZATION;

/// HTTP header used for the private Thirdpass client identifier.
pub const CLIENT_ID_HEADER: &str = "X-Thirdpass-Client-Id";

/// Add standard Thirdpass client headers to an outbound API request.
pub fn with_client_headers(
    request: reqwest::blocking::RequestBuilder,
    config: &crate::common::config::Config,
) -> reqwest::blocking::RequestBuilder {
    let mut request = request.header("User-Agent", super::HTTP_USER_AGENT);
    let api_key = config.core.api_key.trim();
    if !api_key.is_empty() {
        request = request.header(AUTHORIZATION, format!("Bearer {}", api_key));
    }
    if !config.core.client_id.is_empty() {
        request = request.header(CLIENT_ID_HEADER, config.core.client_id.as_str());
    }
    request
}

/// Normalize a configured API base URL while preserving any path prefix.
pub fn normalize_base(raw: &str) -> Result<url::Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format_err!("API base URL is empty."));
    }

    let mut value = raw.to_string();
    if !value.starts_with("http://") && !value.starts_with("https://") {
        value = format!("https://{}", value);
    }

    let mut base = url::Url::parse(&value)?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    Ok(base)
}

/// Join a relative API path to a normalized base URL.
pub fn join(base: &url::Url, path: &str) -> Result<url::Url> {
    Ok(base.join(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_client_headers_adds_private_client_id() {
        let mut config = crate::common::config::Config::default();
        config.core.client_id = "client-id-1".to_string();
        let client = reqwest::blocking::Client::new();

        let request = with_client_headers(client.get("https://example.test"), &config)
            .build()
            .expect("failed to build request");

        assert_eq!(
            request.headers().get(CLIENT_ID_HEADER).unwrap(),
            "client-id-1"
        );
    }

    #[test]
    fn with_client_headers_adds_bearer_api_key() {
        let mut config = crate::common::config::Config::default();
        config.core.api_key = " api-key-1 ".to_string();
        let client = reqwest::blocking::Client::new();

        let request = with_client_headers(client.get("https://example.test"), &config)
            .build()
            .expect("failed to build request");

        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer api-key-1"
        );
    }

    #[test]
    fn with_client_headers_omits_empty_client_id() {
        let config = crate::common::config::Config::default();
        let client = reqwest::blocking::Client::new();

        let request = with_client_headers(client.get("https://example.test"), &config)
            .build()
            .expect("failed to build request");

        assert!(request.headers().get(CLIENT_ID_HEADER).is_none());
    }

    #[test]
    fn normalize_base_preserves_api_path_prefix() {
        let base = normalize_base("https://thirdpass.dev/api").expect("failed to normalize base");
        let url = join(&base, "v1/reviews").expect("failed to join API path");

        assert_eq!(url.as_str(), "https://thirdpass.dev/api/v1/reviews");
    }
}
