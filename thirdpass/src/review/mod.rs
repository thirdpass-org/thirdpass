use anyhow::Result;

pub mod active;
pub mod comment;
mod common;
pub(crate) mod dependency_queue;
pub mod fs;
pub mod remote;
pub mod tool;
pub mod workspace;

pub use crate::review::common::{
    Priority, Review, ReviewScope, ReviewTarget, ReviewerDetails, SecuritySummary,
};

pub fn overall_security_summary(review: &Review) -> Result<SecuritySummary> {
    Ok(review
        .targets
        .iter()
        .map(|target| {
            target
                .security_summary
                .unwrap_or_else(|| security_summary_for_comments(&target.comments))
        })
        .fold(SecuritySummary::None, highest_security_summary))
}

pub fn security_summary_for_comments<'a>(
    comments: impl IntoIterator<Item = &'a comment::Comment>,
) -> SecuritySummary {
    let mut summary = SecuritySummary::Low;
    let mut saw_comment = false;
    for comment in comments {
        saw_comment = true;
        match comment.security {
            Priority::Critical => return SecuritySummary::Critical,
            Priority::Medium => summary = SecuritySummary::Medium,
            Priority::Low => {}
        }
    }
    if !saw_comment {
        return SecuritySummary::None;
    }
    summary
}

fn highest_security_summary(left: SecuritySummary, right: SecuritySummary) -> SecuritySummary {
    if security_summary_rank(&right) > security_summary_rank(&left) {
        right
    } else {
        left
    }
}

fn security_summary_rank(summary: &SecuritySummary) -> u8 {
    match summary {
        SecuritySummary::Critical => 3,
        SecuritySummary::Medium => 2,
        SecuritySummary::Low => 1,
        SecuritySummary::None => 0,
    }
}

pub fn store_pending(review: &Review) -> Result<std::path::PathBuf> {
    fs::add(review, fs::ReviewStorageStatus::Pending)
}

pub fn store_submitted(review: &Review) -> Result<std::path::PathBuf> {
    fs::add(review, fs::ReviewStorageStatus::Submitted)
}

pub fn promote_pending(
    review: &Review,
    pending_path: &std::path::PathBuf,
) -> Result<std::path::PathBuf> {
    fs::promote(review, pending_path)
}
