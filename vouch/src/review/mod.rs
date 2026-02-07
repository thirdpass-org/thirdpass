use anyhow::Result;

pub mod active;
pub mod comment;
mod common;
pub mod fs;
pub mod remote;
pub mod tool;
pub mod workspace;

pub use crate::review::common::{
    Priority, Review, ReviewConfidence, ReviewScope, ReviewTarget, ReviewerDetails, SecuritySummary,
};

pub fn overall_security_summary(review: &Review) -> Result<SecuritySummary> {
    let mut summary = SecuritySummary::Low;
    let mut saw_comment = false;
    for target in &review.targets {
        for comment in &target.comments {
            saw_comment = true;
            match comment.security {
                Priority::Critical => return Ok(SecuritySummary::Critical),
                Priority::Medium => summary = SecuritySummary::Medium,
                Priority::Low => {}
            }
        }
    }
    if !saw_comment {
        return Ok(SecuritySummary::None);
    }
    Ok(summary)
}

pub fn store_pending(review: &Review) -> Result<std::path::PathBuf> {
    fs::add(&review, fs::ReviewStorageStatus::Pending)
}

pub fn store_submitted(review: &Review) -> Result<std::path::PathBuf> {
    fs::add(&review, fs::ReviewStorageStatus::Submitted)
}

pub fn promote_pending(
    review: &Review,
    pending_path: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    fs::promote(&review, pending_path)
}
