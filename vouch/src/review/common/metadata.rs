#[derive(
    Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize, Default,
)]
pub struct ReviewMetadata {
    pub reviewer_uuid: String,
    pub agent_name: String,
    pub agent_model: String,
    pub prompt_version: String,
    pub created_at: String,
    pub tool_version: String,
}
