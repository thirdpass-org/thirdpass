#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewScope {
    TargetFileFull,
    TargetFilePartial,
}

impl Default for ReviewScope {
    fn default() -> Self {
        ReviewScope::TargetFilePartial
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct ReviewerDetails {
    pub reviewer_uuid: String,
    pub agent_name: String,
    pub agent_model: String,
    pub agent_reasoning_effort: String,
    pub prompt_version: String,
    pub review_scope: ReviewScope,
    pub created_at: String,
    pub vouch_version: String,
}
