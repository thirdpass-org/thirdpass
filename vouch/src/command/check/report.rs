use anyhow::Result;

use crate::common::StoreTransaction;
use crate::review;

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct DependencyReport {
    pub summary: review::SecuritySummary,
    pub name: String,
    pub version: Option<String>,
    pub review_count: Option<usize>,
    pub note: Option<String>,
}

/// Given a local project dependency, create a corresponding review report from known reviews.
pub fn get_dependency_report(
    dependency: &vouch_lib::extension::Dependency,
    registry_host_name: &str,
    config: &crate::common::config::Config,
    tx: &StoreTransaction,
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

    let _ = pull_latest_reviews(
        registry_host_name,
        &dependency.name,
        &package_version,
        config,
        tx,
    );

    let reviews = review::index::get(
        &review::index::Fields {
            package_name: Some(&dependency.name),
            package_version: Some(&package_version),
            registry_host_names: Some(maplit::btreeset! {registry_host_name}),
            ..Default::default()
        },
        &tx,
    )?;

    if reviews.is_empty() {
        // Report no reviews found for dependency.
        return Ok(DependencyReport {
            summary: review::SecuritySummary::None,
            name: dependency.name.clone(),
            version: Some(package_version.clone()),
            review_count: Some(0),
            note: None,
        });
    }

    let stats = get_dependency_stats(&reviews)?;
    let status = get_dependency_status(&stats)?;
    let note = get_dependency_note(&stats)?;

    Ok(DependencyReport {
        summary: status,
        name: dependency.name.clone(),
        version: Some(package_version.clone()),
        review_count: Some(reviews.len()),
        note: Some(note),
    })
}

fn pull_latest_reviews(
    registry_host_name: &str,
    package_name: &str,
    package_version: &str,
    config: &crate::common::config::Config,
    tx: &StoreTransaction,
) -> Result<()> {
    let query = review::remote::ReviewQuery {
        registry_host: Some(registry_host_name.to_string()),
        package_name: Some(package_name.to_string()),
        package_version: Some(package_version.to_string()),
        file_path: None,
    };
    let records = review::remote::fetch(&query, config)?;
    review::remote::store_records(records, config, tx)?;
    Ok(())
}

#[derive(Debug, Default, Clone)]
struct DependencyStats {
    pub total_review_count: usize,
    pub count_critical_comments: i32,
    pub count_medium_comments: i32,
}

fn get_dependency_stats(reviews: &Vec<review::Review>) -> Result<DependencyStats> {
    let mut stats = DependencyStats::default();
    stats.total_review_count = reviews.len();

    for review in reviews {
        match review::overall_security_summary(&review)? {
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

fn get_dependency_note(stats: &DependencyStats) -> Result<String> {
    let mut note_parts = Vec::<_>::new();
    if stats.count_critical_comments > 0 {
        note_parts.push(format!("critical ({})", stats.count_critical_comments));
    }

    if stats.count_medium_comments > 0 {
        note_parts.push(format!("medium ({})", stats.count_medium_comments));
    }

    Ok(note_parts.join("; "))
}
