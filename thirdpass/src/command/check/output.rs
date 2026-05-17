use anyhow::Result;

use super::report;
use super::table;
use super::OutputFormat;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DependencyGroup {
    pub registry_host_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<std::path::PathBuf>,
    pub dependencies: Vec<report::DependencyReport>,
    #[serde(skip)]
    pub first_row_separate: bool,
}

pub fn print(groups: &[DependencyGroup], output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Table => print_table(groups)?,
        OutputFormat::Plain => print_plain(groups),
        OutputFormat::Json => print_json(groups)?,
    }
    Ok(())
}

fn print_table(groups: &[DependencyGroup]) -> Result<()> {
    for (index, group) in groups.iter().enumerate() {
        if let Some(source_path) = &group.source_path {
            println!(
                "Registry: {name}\n{path}",
                name = group.registry_host_name,
                path = source_path.display(),
            );
        } else {
            println!("Registry: {name}", name = group.registry_host_name);
        }

        let table = table::get(&group.dependencies, group.first_row_separate)?;
        table.printstd();

        if index + 1 != groups.len() {
            println!();
        }
    }
    Ok(())
}

fn print_plain(groups: &[DependencyGroup]) {
    for (index, group) in groups.iter().enumerate() {
        if let Some(source_path) = &group.source_path {
            println!(
                "registry={registry} source={path}",
                registry = group.registry_host_name,
                path = source_path.display(),
            );
        } else {
            println!("registry={registry}", registry = group.registry_host_name);
        }
        println!("summary\tname\tversion\treviews\tnotes");
        for dependency in &group.dependencies {
            let version = dependency.version.as_deref().unwrap_or("");
            let review_count = dependency
                .review_count
                .map(|count| count.to_string())
                .unwrap_or_default();
            let note = dependency.note.as_deref().unwrap_or("");
            println!(
                "{summary}\t{name}\t{version}\t{review_count}\t{note}",
                summary = &dependency.summary,
                name = dependency.name.as_str(),
                version = version,
                review_count = review_count,
                note = note
            );
        }
        if index + 1 != groups.len() {
            println!();
        }
    }
}

fn print_json(groups: &[DependencyGroup]) -> Result<()> {
    let output = serde_json::to_string_pretty(groups)?;
    println!("{}", output);
    Ok(())
}
