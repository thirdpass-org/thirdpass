use anyhow::{format_err, Context, Result};
use std::io::Write;
use std::path::Path;

mod common;
mod core;
mod extensions;
mod review_tool;

#[derive(
    Debug, Clone, Default, Ord, PartialOrd, Eq, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct Config {
    pub core: core::Core,

    #[serde(rename = "review-tool")]
    pub review_tool: review_tool::ReviewTool,

    pub extensions: extensions::Extensions,
}

impl Config {
    pub fn load() -> Result<Self> {
        log::debug!("Loading config.");
        let paths = super::fs::ConfigPaths::new()?;
        log::debug!("Config paths: {:?}", paths);

        let file = std::fs::File::open(paths.config_file)?;
        let reader = std::io::BufReader::new(file);
        Ok(serde_yaml::from_reader(reader)?)
    }

    /// Persist this configuration to disk without exposing a partial file.
    pub fn dump(&self) -> Result<()> {
        let paths = super::fs::ConfigPaths::new()?;
        write_yaml_atomically(&paths.config_file, self)
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        let name_error_message = format!("Unknown settings field: {}", name);

        if core::is_match(name)? {
            Ok(core::set(&mut self.core, name, value)?)
        } else if extensions::is_match(name)? {
            Ok(extensions::set(&mut self.extensions, name, value)?)
        } else if review_tool::is_match(name)? {
            Ok(review_tool::set(&mut self.review_tool, name, value)?)
        } else {
            Err(format_err!(name_error_message.clone()))
        }
    }

    pub fn get(&self, name: &str) -> Result<String> {
        let name_error_message = format!("Unknown settings field: {}", name);

        if core::is_match(name)? {
            Ok(core::get(&self.core, name)?)
        } else if extensions::is_match(name)? {
            Ok(extensions::get(&self.extensions, name)?)
        } else if review_tool::is_match(name)? {
            Ok(review_tool::get(&self.review_tool, name)?)
        } else {
            Err(format_err!(name_error_message.clone()))
        }
    }
}

fn write_yaml_atomically<T>(path: &Path, value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    let parent = path.parent().ok_or(format_err!(
        "Can't find parent directory for config file: {}",
        path.display()
    ))?;
    std::fs::create_dir_all(parent).context(format!(
        "Can't create config directory: {}",
        parent.display()
    ))?;

    let mut temp_file = tempfile::NamedTempFile::new_in(parent).context(format!(
        "Can't create temporary config file in directory: {}",
        parent.display()
    ))?;
    {
        let mut writer = std::io::BufWriter::new(temp_file.as_file_mut());
        serde_yaml::to_writer(&mut writer, value).context(format!(
            "Can't serialize config for file: {}",
            path.display()
        ))?;
        writer.flush().context(format!(
            "Can't flush temporary config file for: {}",
            path.display()
        ))?;
    }
    temp_file.as_file().sync_all().context(format!(
        "Can't sync temporary config file for: {}",
        path.display()
    ))?;

    temp_file
        .persist(path)
        .map_err(|err| err.error)
        .context(format!(
            "Can't replace config file atomically: {}",
            path.display()
        ))?;
    sync_parent_directory(parent)?;

    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(directory: &Path) -> Result<()> {
    std::fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .context(format!(
            "Can't sync config directory: {}",
            directory.display()
        ))
}

#[cfg(not(unix))]
fn sync_parent_directory(_directory: &Path) -> Result<()> {
    Ok(())
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut display_config = self.clone();
        if !display_config.core.api_key.is_empty() {
            display_config.core.api_key = "<redacted>".to_string();
        }
        write!(
            f,
            "{}",
            serde_yaml::to_string(&display_config).map_err(|_| std::fmt::Error)?
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::{Error as _, Serializer};

    struct FailingSerialize;

    impl serde::Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(S::Error::custom("serialization failed"))
        }
    }

    #[test]
    fn atomic_yaml_write_replaces_existing_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.yaml");
        std::fs::write(&path, "old: value\n")?;

        write_yaml_atomically(&path, &vec!["new"])?;

        assert_eq!(std::fs::read_to_string(&path)?, "---\n- new\n");
        Ok(())
    }

    #[test]
    fn atomic_yaml_write_preserves_existing_file_after_serialization_failure() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.yaml");
        std::fs::write(&path, "old: value\n")?;

        let err = write_yaml_atomically(&path, &FailingSerialize).unwrap_err();

        assert!(err.to_string().contains("Can't serialize config"));
        assert_eq!(std::fs::read_to_string(&path)?, "old: value\n");
        Ok(())
    }

    #[test]
    fn display_redacts_api_key() {
        let mut config = Config::default();
        config.core.api_key = "secret-key".to_string();

        let output = config.to_string();

        assert!(output.contains("api-key:"));
        assert!(output.contains("<redacted>"));
        assert!(!output.contains("secret-key"));
    }
}
