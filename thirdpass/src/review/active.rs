use crate::review;
use anyhow::{Context, Result};
use std::io::Write;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct ActiveReview {
    pub title: String,
    pub description: String,
    #[serde(rename = "isPrimary")]
    pub is_primary: Option<bool>,
    pub comments: std::collections::BTreeSet<review::comment::Comment>,
}

/// Ensure active review file is in place.
pub fn ensure(
    review: &review::Review,
    reviews_directory: &std::path::Path,
) -> Result<std::path::PathBuf> {
    let review_file_path = reviews_directory.join("local.review");
    if review_file_path.exists() {
        return Ok(review_file_path);
    }

    let active_review = ActiveReview {
        title: "local".to_string(),
        description: format!(
            "Package name-version: {}-{}",
            review.package.name, review.package.version
        ),
        is_primary: Some(true),
        comments: flatten_comments(review),
    };

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .append(false)
        .create(true)
        .truncate(true)
        .open(&review_file_path)
        .context(format!(
            "Can't open/create file for writing: {}",
            review_file_path.display()
        ))?;
    file.write_all(serde_json::to_string_pretty(&active_review)?.as_bytes())?;
    Ok(review_file_path)
}

pub fn parse(
    path: &std::path::PathBuf,
) -> Result<std::collections::BTreeSet<review::comment::Comment>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let active_review: review::active::ActiveReview = serde_json::from_reader(reader)?;
    Ok(active_review.comments)
}

fn flatten_comments(
    review: &review::Review,
) -> std::collections::BTreeSet<review::comment::Comment> {
    let mut comments = std::collections::BTreeSet::new();
    for target in &review.targets {
        for comment in &target.comments {
            comments.insert(comment.clone());
        }
    }
    comments
}
