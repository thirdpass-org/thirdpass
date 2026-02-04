use crate::common::StoreTransaction;
use anyhow::Result;

pub mod active;
pub mod comment;
mod common;
pub mod fs;
pub mod index;
pub mod tool;
pub mod workspace;

pub use crate::review::common::{Review, Summary};

pub struct ReviewAnalysis {
    pub count_fail_comments: i32,
    pub count_warn_comments: i32,
}

pub fn analyse(review: &Review) -> Result<ReviewAnalysis> {
    let count_warn_comments = review.comments.iter().fold(0, |sum, comment| {
        if comment.summary == Summary::Warn {
            sum + 1
        } else {
            sum
        }
    });
    let count_fail_comments = review.comments.iter().fold(0, |sum, comment| {
        if comment.summary == Summary::Fail {
            sum + 1
        } else {
            sum
        }
    });
    Ok(ReviewAnalysis {
        count_fail_comments,
        count_warn_comments,
    })
}

pub fn store(review: &Review, tx: &StoreTransaction) -> Result<()> {
    index::update(&review, &tx)?;
    fs::add(&review)?;
    Ok(())
}
