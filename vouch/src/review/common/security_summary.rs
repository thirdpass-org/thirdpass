#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySummary {
    Critical,
    Medium,
    Low,
    None,
}

impl std::str::FromStr for SecuritySummary {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> Result<SecuritySummary, Self::Err> {
        match input {
            "critical" => Ok(SecuritySummary::Critical),
            "medium" => Ok(SecuritySummary::Medium),
            "low" => Ok(SecuritySummary::Low),
            "none" => Ok(SecuritySummary::None),
            _ => Err(anyhow::format_err!(
                "Failed to parse security summary from string: {}",
                input
            )),
        }
    }
}

impl std::fmt::Display for SecuritySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}
