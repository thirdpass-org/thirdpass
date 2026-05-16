use anyhow::Result;

pub fn get_config_path(extension_name: &str) -> Result<std::path::PathBuf> {
    let config_paths = crate::common::fs::ConfigPaths::new()?;
    Ok(config_paths.extensions_directory.join(format!(
        "{extension_name}.yaml",
        extension_name = extension_name
    )))
}

pub fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }

    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }

    "unknown panic payload".to_string()
}
