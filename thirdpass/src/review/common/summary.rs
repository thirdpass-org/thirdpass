#[derive(
    Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Summary {
    Fail,
    Warn,
    Pass,
    Todo,
}

impl std::str::FromStr for Summary {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> Result<Summary, Self::Err> {
        match input {
            "todo" => Ok(Summary::Todo),
            "pass" => Ok(Summary::Pass),
            "warn" => Ok(Summary::Warn),
            "fail" => Ok(Summary::Fail),
            _ => Err(anyhow::format_err!(
                "Failed to parse summary type from string: {}",
                input
            )),
        }
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", format!("{:?}", self).to_lowercase())
    }
}
