use anyhow::Result;

use crate::common;
use crate::extension;
use crate::review;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewCandidate {
    pub(crate) extension_name: String,
    pub(crate) registry_host_name: String,
    pub(crate) package_name: String,
    pub(crate) package_version: String,
    pub(crate) current_reviewer_review_count: usize,
    pub(crate) total_review_count: usize,
}

impl DependencyReviewCandidate {
    pub(super) fn review_package(&self) -> review::dependency_plan::DependencyReviewPackage {
        review::dependency_plan::DependencyReviewPackage {
            extension_name: self.extension_name.clone(),
            registry_host_name: self.registry_host_name.clone(),
            package_name: self.package_name.clone(),
            package_version: self.package_version.clone(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct DependencyReviewDiscovery {
    pub(crate) dependency_files: Vec<std::path::PathBuf>,
    pub(crate) candidates: Vec<DependencyReviewCandidate>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct DependencyReviewKey {
    extension_name: String,
    registry_host_name: String,
    package_name: String,
    package_version: String,
}

pub(crate) fn discover_local_review_dependencies(
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
    working_directory: &std::path::Path,
    config: &common::config::Config,
) -> Result<DependencyReviewDiscovery> {
    let all_dependencies = extension::identify_file_defined_dependencies(
        extensions,
        extension_args,
        working_directory,
    )?;

    let stored_reviews = review::fs::list()?;
    let mut dependency_files = std::collections::BTreeSet::<std::path::PathBuf>::new();
    let mut candidates =
        std::collections::BTreeMap::<DependencyReviewKey, DependencyReviewCandidate>::new();

    for (extension, extension_dependencies) in extensions.iter().zip(all_dependencies.into_iter()) {
        let extension_dependencies = match extension_dependencies {
            Ok(dependencies) => dependencies,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };

        for dependency_file in extension_dependencies {
            dependency_files.insert(dependency_file.path.clone());
            for dependency in dependency_file.dependencies {
                insert_dependency_candidate(
                    &mut candidates,
                    extension.name(),
                    dependency_file.registry_host_name.clone(),
                    dependency,
                    &stored_reviews,
                    &config.core.public_user_id,
                );
            }
        }
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    sort_dependency_review_candidates(&mut candidates);
    Ok(DependencyReviewDiscovery {
        dependency_files: dependency_files.into_iter().collect(),
        candidates,
    })
}

pub(crate) fn discover_package_review_dependencies(
    package_name: &str,
    package_version: &Option<String>,
    extensions: &[Box<dyn thirdpass_core::extension::Extension>],
    extension_args: &[String],
    config: &common::config::Config,
) -> Result<DependencyReviewDiscovery> {
    let package_version = package_version.as_deref();
    let all_dependencies = extension::identify_package_dependencies(
        package_name,
        &package_version,
        extensions,
        extension_args,
    )?;

    let stored_reviews = review::fs::list()?;
    let mut candidates =
        std::collections::BTreeMap::<DependencyReviewKey, DependencyReviewCandidate>::new();

    for (extension, extension_dependencies) in extensions.iter().zip(all_dependencies.into_iter()) {
        let extension_dependencies = match extension_dependencies {
            Ok(dependencies) => dependencies,
            Err(error) => {
                log::error!("Extension error: {}", error);
                continue;
            }
        };

        for package_dependencies in extension_dependencies {
            insert_dependency_candidate(
                &mut candidates,
                extension.name(),
                package_dependencies.registry_host_name.clone(),
                thirdpass_core::extension::Dependency {
                    name: package_name.to_string(),
                    version: package_dependencies.package_version,
                },
                &stored_reviews,
                &config.core.public_user_id,
            );
            for dependency in package_dependencies.dependencies {
                insert_dependency_candidate(
                    &mut candidates,
                    extension.name(),
                    package_dependencies.registry_host_name.clone(),
                    dependency,
                    &stored_reviews,
                    &config.core.public_user_id,
                );
            }
        }
    }

    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    sort_dependency_review_candidates(&mut candidates);
    Ok(DependencyReviewDiscovery {
        dependency_files: Vec::new(),
        candidates,
    })
}

fn insert_dependency_candidate(
    candidates: &mut std::collections::BTreeMap<DependencyReviewKey, DependencyReviewCandidate>,
    extension_name: String,
    registry_host_name: String,
    dependency: thirdpass_core::extension::Dependency,
    stored_reviews: &[review::Review],
    public_user_id: &str,
) {
    let package_version = match dependency.version {
        Ok(package_version) => package_version,
        Err(error) => {
            log::debug!(
                "Skipping dependency {} because version is not reviewable: {}",
                dependency.name,
                error
            );
            return;
        }
    };
    let key = DependencyReviewKey {
        extension_name,
        registry_host_name,
        package_name: dependency.name,
        package_version,
    };
    let (current_reviewer_review_count, total_review_count) =
        count_matching_reviews(&key, stored_reviews, public_user_id);
    candidates
        .entry(key.clone())
        .or_insert_with(|| DependencyReviewCandidate {
            extension_name: key.extension_name,
            registry_host_name: key.registry_host_name,
            package_name: key.package_name,
            package_version: key.package_version,
            current_reviewer_review_count,
            total_review_count,
        });
}

fn count_matching_reviews(
    candidate: &DependencyReviewKey,
    reviews: &[review::Review],
    public_user_id: &str,
) -> (usize, usize) {
    let mut current_reviewer_review_count = 0;
    let mut total_review_count = 0;
    for review in reviews {
        if !matches_dependency_candidate(candidate, review) {
            continue;
        }

        total_review_count += 1;
        if review.reviewer_details.public_user_id == public_user_id {
            current_reviewer_review_count += 1;
        }
    }
    (current_reviewer_review_count, total_review_count)
}

fn matches_dependency_candidate(candidate: &DependencyReviewKey, review: &review::Review) -> bool {
    review.package.name == candidate.package_name
        && review.package.version == candidate.package_version
        && review
            .package
            .registries
            .iter()
            .any(|registry| registry.host_name == candidate.registry_host_name)
}

fn sort_dependency_review_candidates(candidates: &mut [DependencyReviewCandidate]) {
    candidates.sort_by(|a, b| {
        a.current_reviewer_review_count
            .cmp(&b.current_reviewer_review_count)
            .then_with(|| a.total_review_count.cmp(&b.total_review_count))
            .then_with(|| a.registry_host_name.cmp(&b.registry_host_name))
            .then_with(|| a.package_name.cmp(&b.package_name))
            .then_with(|| a.package_version.cmp(&b.package_version))
            .then_with(|| a.extension_name.cmp(&b.extension_name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{package, peer, registry};

    #[test]
    fn sort_dependency_review_candidates_prefers_review_needs() {
        let mut candidates = vec![
            candidate("js", "npmjs.com", "covered-by-user", "1.0.0", 1, 1),
            candidate("js", "npmjs.com", "globally-covered", "1.0.0", 0, 2),
            candidate("js", "npmjs.com", "uncovered", "1.0.0", 0, 0),
        ];

        sort_dependency_review_candidates(&mut candidates);

        let names = candidates
            .iter()
            .map(|candidate| candidate.package_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["uncovered", "globally-covered", "covered-by-user"]
        );
    }

    #[test]
    fn count_matching_reviews_counts_current_reviewer_and_total() -> Result<()> {
        let candidate = DependencyReviewKey {
            extension_name: "js".to_string(),
            registry_host_name: "npmjs.com".to_string(),
            package_name: "left-pad".to_string(),
            package_version: "1.3.0".to_string(),
        };
        let reviews = vec![
            stored_review("user-a", "npmjs.com", "left-pad", "1.3.0")?,
            stored_review("user-b", "npmjs.com", "left-pad", "1.3.0")?,
            stored_review("user-a", "npmjs.com", "left-pad", "1.2.0")?,
            stored_review("user-a", "pypi.org", "left-pad", "1.3.0")?,
        ];

        assert_eq!(
            count_matching_reviews(&candidate, &reviews, "user-a"),
            (1, 2)
        );
        Ok(())
    }

    #[test]
    fn insert_dependency_candidate_adds_review_counts() -> Result<()> {
        let mut candidates = std::collections::BTreeMap::new();
        let reviews = vec![
            stored_review("user-a", "crates.io", "axum", "0.8.9")?,
            stored_review("user-b", "crates.io", "axum", "0.8.9")?,
        ];

        insert_dependency_candidate(
            &mut candidates,
            "rs".to_string(),
            "crates.io".to_string(),
            thirdpass_core::extension::Dependency {
                name: "axum".to_string(),
                version: Ok("0.8.9".to_string()),
            },
            &reviews,
            "user-a",
        );

        let candidate = candidates.values().next().expect("candidate was not added");
        assert_eq!(candidate.package_name, "axum");
        assert_eq!(candidate.package_version, "0.8.9");
        assert_eq!(candidate.current_reviewer_review_count, 1);
        assert_eq!(candidate.total_review_count, 2);
        Ok(())
    }

    fn candidate(
        extension_name: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
        current_reviewer_review_count: usize,
        total_review_count: usize,
    ) -> DependencyReviewCandidate {
        DependencyReviewCandidate {
            extension_name: extension_name.to_string(),
            registry_host_name: registry_host_name.to_string(),
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            current_reviewer_review_count,
            total_review_count,
        }
    }

    fn stored_review(
        public_user_id: &str,
        registry_host_name: &str,
        package_name: &str,
        package_version: &str,
    ) -> Result<review::Review> {
        let mut registries = std::collections::BTreeSet::new();
        registries.insert(registry::Registry {
            id: 0,
            host_name: registry_host_name.to_string(),
            human_url: url::Url::parse("https://registry.example/pkg")?,
            artifact_url: url::Url::parse("https://registry.example/pkg.tgz")?,
        });

        Ok(review::Review {
            id: 0,
            peer: peer::Peer::default(),
            package: package::Package {
                id: 0,
                name: package_name.to_string(),
                version: package_version.to_string(),
                registries,
                package_hash: "package-hash".to_string(),
            },
            targets: Vec::new(),
            reviewer_details: review::ReviewerDetails {
                public_user_id: public_user_id.to_string(),
                ..Default::default()
            },
            review_configuration: None,
            agent_summary: String::new(),
            overall_security_summary: review::SecuritySummary::default(),
            overall_security_confidence: None,
        })
    }
}
