use anyhow::{format_err, Result};

#[derive(
    Debug, Clone, Default, Ord, PartialOrd, Eq, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct Core {
    #[serde(rename = "api-key")]
    pub api_key: String,
    #[serde(rename = "api-base", default)]
    pub api_base: String,
    /// Private client identifier shared only with the Thirdpass server.
    #[serde(rename = "client-id", default)]
    pub client_id: String,
    #[serde(rename = "reviewer-uuid")]
    pub reviewer_uuid: String,
}

fn get_regex() -> Result<regex::Regex> {
    Ok(regex::Regex::new(r"core\.(.*)")?)
}

pub fn is_match(name: &str) -> Result<bool> {
    Ok(get_regex()?.is_match(name))
}

pub fn set(core: &mut Core, name: &str, value: &str) -> Result<()> {
    let name_error_message = format!("Unknown setting field name: {}", name);

    let captures = get_regex()?
        .captures(name)
        .ok_or(format_err!(name_error_message.clone()))?;
    let field = captures
        .get(1)
        .ok_or(format_err!(name_error_message.clone()))?
        .as_str();

    match field {
        "api-key" => {
            core.api_key = value.to_string();
            Ok(())
        }
        "api-base" => {
            core.api_base = value.to_string();
            Ok(())
        }
        "client-id" => {
            core.client_id = value.to_string();
            Ok(())
        }
        "reviewer-uuid" => {
            core.reviewer_uuid = value.to_string();
            Ok(())
        }
        _ => Err(format_err!(name_error_message.clone())),
    }
}

pub fn get(core: &Core, name: &str) -> Result<String> {
    let name_error_message = format!("Unknown setting field name: {}", name);

    let captures = get_regex()?
        .captures(name)
        .ok_or(format_err!(name_error_message.clone()))?;
    let field = captures
        .get(1)
        .ok_or(format_err!(name_error_message.clone()))?
        .as_str();

    match field {
        "api-key" => Ok(core.api_key.clone()),
        "api-base" => Ok(core.api_base.clone()),
        "client-id" => Ok(core.client_id.clone()),
        "reviewer-uuid" => Ok(core.reviewer_uuid.clone()),
        _ => Err(format_err!(name_error_message.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_can_be_set_and_read() {
        let mut core = Core::default();

        set(&mut core, "core.client-id", "client-1").expect("failed to set client id");

        assert_eq!(
            get(&core, "core.client-id").expect("failed to get client id"),
            "client-1"
        );
    }

    #[test]
    fn missing_client_id_defaults_to_empty_for_existing_configs() {
        let core: Core = serde_yaml::from_str(
            r#"
api-key: tmp_api_key
api-base: https://thirdpass.dev/api
reviewer-uuid: reviewer-1
"#,
        )
        .expect("failed to deserialize legacy core config");

        assert_eq!(core.client_id, "");
    }
}
