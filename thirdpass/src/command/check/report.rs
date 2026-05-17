use anyhow::Result;

use crate::review;

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
pub struct DependencyReport {
    pub summary: review::SecuritySummary,
    pub name: String,
    pub version: Option<String>,
    pub review_count: Option<usize>,
    pub note: Option<String>,
}

/// Given a local project dependency, create a corresponding review report from known reviews.
pub fn get_dependency_report(
    dependency: &thirdpass_core::extension::Dependency,
    registry_host_name: &str,
    config: &crate::common::config::Config,
) -> Result<DependencyReport> {
    let package_version = match &dependency.version {
        Ok(version) => version.clone(),
        Err(error) => {
            return Ok(DependencyReport {
                summary: review::SecuritySummary::Medium,
                name: dependency.name.clone(),
                version: None,
                review_count: None,
                note: Some(error.message()),
            });
        }
    };

    let sync_note = match pull_latest_reviews(
        registry_host_name,
        &dependency.name,
        &package_version,
        config,
    ) {
        Ok(_) => None,
        Err(err) => {
            log::warn!(
                "Failed to sync latest reviews for {name}@{version} ({registry}): {error}",
                name = dependency.name,
                version = package_version,
                registry = registry_host_name,
                error = err
            );
            Some("sync failed; using local cache".to_string())
        }
    };

    let reviews = filter_reviews(
        &review::fs::list()?,
        registry_host_name,
        &dependency.name,
        &package_version,
    );

    if reviews.is_empty() {
        // Report no reviews found for dependency.
        return Ok(DependencyReport {
            summary: review::SecuritySummary::None,
            name: dependency.name.clone(),
            version: Some(package_version.clone()),
            review_count: Some(0),
            note: sync_note,
        });
    }

    let stats = get_dependency_stats(&reviews)?;
    let status = get_dependency_status(&stats)?;
    let note = merge_notes(get_dependency_note(&stats), sync_note);

    Ok(DependencyReport {
        summary: status,
        name: dependency.name.clone(),
        version: Some(package_version.clone()),
        review_count: Some(reviews.len()),
        note,
    })
}

fn pull_latest_reviews(
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
    config: &crate::common::config::Config,
) -> Result<()> {
    let query = review::remote::ReviewQuery {
        registry_host: Some(registry_host_name.to_string()),
        package_name: Some(package_name.to_string()),
        package_version: Some(package_version.to_string()),
        file_path: None,
    };
    let records = review::remote::fetch(&query, config)?;
    review::remote::store_records(records, config)?;
    Ok(())
}

fn filter_reviews(
    reviews: &[review::Review],
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
) -> Vec<review::Review> {
    reviews
        .iter()
        .filter(|review| {
            review.package.name == package_name
                && review.package.version == package_version
                && review
                    .package
                    .registries
                    .iter()
                    .any(|registry| registry.host_name == registry_host_name)
        })
        .cloned()
        .collect()
}

#[derive(Debug, Default, Clone)]
struct DependencyStats {
    pub total_review_count: usize,
    pub count_critical_comments: i32,
    pub count_medium_comments: i32,
}

fn get_dependency_stats(reviews: &[review::Review]) -> Result<DependencyStats> {
    let mut stats = DependencyStats {
        total_review_count: reviews.len(),
        ..DependencyStats::default()
    };

    for review in reviews {
        match review::overall_security_summary(review)? {
            review::SecuritySummary::Critical => stats.count_critical_comments += 1,
            review::SecuritySummary::Medium => stats.count_medium_comments += 1,
            review::SecuritySummary::Low => {}
            review::SecuritySummary::None => {}
        }
    }
    Ok(stats)
}

fn get_dependency_status(stats: &DependencyStats) -> Result<review::SecuritySummary> {
    if stats.count_critical_comments > 0 {
        return Ok(review::SecuritySummary::Critical);
    }
    if stats.count_medium_comments > 0 {
        return Ok(review::SecuritySummary::Medium);
    }
    if stats.total_review_count == 0 {
        return Ok(review::SecuritySummary::None);
    }
    Ok(review::SecuritySummary::Low)
}

fn get_dependency_note(stats: &DependencyStats) -> Option<String> {
    let mut note_parts = Vec::<_>::new();
    if stats.count_critical_comments > 0 {
        note_parts.push(format!("critical ({})", stats.count_critical_comments));
    }

    if stats.count_medium_comments > 0 {
        note_parts.push(format!("medium ({})", stats.count_medium_comments));
    }

    if note_parts.is_empty() {
        None
    } else {
        Some(note_parts.join("; "))
    }
}

fn merge_notes(primary_note: Option<String>, secondary_note: Option<String>) -> Option<String> {
    match (primary_note, secondary_note) {
        (None, None) => None,
        (Some(note), None) => Some(note),
        (None, Some(note)) => Some(note),
        (Some(primary), Some(secondary)) => Some(format!("{}; {}", primary, secondary)),
    }
}
