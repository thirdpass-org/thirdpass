use anyhow::{format_err, Result};

#[derive(
    Debug, Clone, Default, Ord, PartialOrd, Eq, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct Core {
    #[serde(rename = "api-key")]
    pub api_key: String,
    #[serde(rename = "api-base", default)]
    pub api_base: String,
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
        "reviewer-uuid" => Ok(core.reviewer_uuid.clone()),
        _ => Err(format_err!(name_error_message.clone())),
    }
}
