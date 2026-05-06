use anyhow::{format_err, Result};

#[derive(
    Debug, Clone, Default, Ord, PartialOrd, Eq, PartialEq, serde::Serialize, serde::Deserialize,
)]
pub struct ReviewTool {
    pub name: String,

    #[serde(rename = "install-check")]
    pub install_check: bool,

    #[serde(rename = "agent-model", skip_serializing_if = "Option::is_none")]
    pub agent_model: Option<String>,

    #[serde(
        rename = "agent-reasoning-effort",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_reasoning_effort: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

fn get_regex() -> Result<regex::Regex> {
    Ok(regex::Regex::new(r"review-tool\.(.*)")?)
}

pub fn is_match(name: &str) -> Result<bool> {
    Ok(get_regex()?.is_match(name))
}

pub fn set(review_tool: &mut ReviewTool, name: &str, value: &str) -> Result<()> {
    let name_error_message = format!("Unknown setting field name: {}", name);

    let captures = get_regex()?
        .captures(name)
        .ok_or(format_err!(name_error_message.clone()))?;
    let field = captures
        .get(1)
        .ok_or(format_err!(name_error_message.clone()))?
        .as_str();

    match field {
        "name" => {
            review_tool.name = value.to_string();
            Ok(())
        }
        "install-check" => {
            review_tool.install_check = value == "true";
            Ok(())
        }
        "agent-model" => {
            let value = value.trim();
            review_tool.agent_model = if value.is_empty()
                || value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("null")
            {
                None
            } else {
                Some(value.to_string())
            };
            Ok(())
        }
        "agent-reasoning-effort" => {
            let value = value.trim();
            review_tool.agent_reasoning_effort = if value.is_empty()
                || value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("null")
            {
                None
            } else {
                Some(value.to_string())
            };
            Ok(())
        }
        "agent" => {
            let value = value.trim();
            review_tool.agent = if value.is_empty()
                || value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("null")
            {
                None
            } else {
                Some(value.to_string())
            };
            Ok(())
        }
        _ => Err(format_err!(name_error_message.clone())),
    }
}

pub fn get(review_tool: &ReviewTool, name: &str) -> Result<String> {
    let name_error_message = format!("Unknown setting field name: {}", name);

    let captures = get_regex()?
        .captures(name)
        .ok_or(format_err!(name_error_message.clone()))?;
    let field = captures
        .get(1)
        .ok_or(format_err!(name_error_message.clone()))?
        .as_str();

    match field {
        "name" => Ok(review_tool.name.to_string()),
        "install-check" => Ok(review_tool.install_check.to_string()),
        "agent-model" => Ok(review_tool.agent_model.clone().unwrap_or_default()),
        "agent-reasoning-effort" => Ok(review_tool
            .agent_reasoning_effort
            .clone()
            .unwrap_or_default()),
        "agent" => Ok(review_tool.agent.clone().unwrap_or_default()),
        _ => Err(format_err!(name_error_message.clone())),
    }
}
