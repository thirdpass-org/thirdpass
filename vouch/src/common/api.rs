use anyhow::{format_err, Result};

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
    if !base.as_str().ends_with('/') {
        base = base.join("/")?;
    }
    Ok(base)
}

pub fn join(base: &url::Url, path: &str) -> Result<url::Url> {
    Ok(base.join(path)?)
}
