use crate::common::StoreTransaction;
use anyhow::Result;

pub mod active;
pub mod comment;
mod common;
pub mod fs;
pub mod index;
pub mod tool;
pub mod workspace;

pub use crate::review::common::{Priority, Review, ReviewMetadata, SecuritySummary};

pub fn overall_security_summary(review: &Review) -> Result<SecuritySummary> {
    if review.comments.is_empty() {
        return Ok(SecuritySummary::None);
    }

    let mut summary = SecuritySummary::Low;
    for comment in &review.comments {
        match comment.security {
            Priority::Critical => return Ok(SecuritySummary::Critical),
            Priority::Medium => summary = SecuritySummary::Medium,
            Priority::Low => {}
        }
    }
    Ok(summary)
}

pub fn store(review: &Review, tx: &StoreTransaction) -> Result<()> {
    index::update(&review, &tx)?;
    fs::add(&review)?;
    Ok(())
}
