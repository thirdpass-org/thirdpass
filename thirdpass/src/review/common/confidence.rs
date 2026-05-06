#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ReviewConfidence {
    High,
    Medium,
    Low,
}

impl std::str::FromStr for ReviewConfidence {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> Result<ReviewConfidence, Self::Err> {
        match input.to_lowercase().as_str() {
            "high" => Ok(ReviewConfidence::High),
            "medium" => Ok(ReviewConfidence::Medium),
            "low" => Ok(ReviewConfidence::Low),
            _ => Err(anyhow::format_err!(
                "Failed to parse confidence type from string: {}",
                input
            )),
        }
    }
}

impl std::fmt::Display for ReviewConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}
