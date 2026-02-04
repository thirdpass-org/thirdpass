use super::report;
use crate::review;
use anyhow::Result;
use prettytable::{self, cell};

fn get_row(dependency_report: &report::DependencyReport) -> prettytable::Row {
    let summary: prettytable::Cell = dependency_report.summary.clone().into();
    let package_version = match &dependency_report.version {
        Some(v) => v.as_str(),
        None => "",
    };
    let review_count = match dependency_report.review_count {
        Some(v) => v.to_string(),
        None => "".to_string(),
    };
    let note = get_note_cell(&dependency_report);
    prettytable::Row::new(vec![
        summary,
        prettytable::Cell::new_align(
            &dependency_report.name,
            prettytable::format::Alignment::LEFT,
        ),
        prettytable::Cell::new_align(&package_version, prettytable::format::Alignment::RIGHT),
        prettytable::Cell::new_align(&review_count, prettytable::format::Alignment::RIGHT),
        note,
    ])
}

/// Generates and returns a table from a given vector of dependency review reports.
pub fn get(
    dependency_reports: &Vec<report::DependencyReport>,
    first_row_separate: bool,
) -> Result<prettytable::Table> {
    let mut table = prettytable::Table::new();
    table.set_titles(prettytable::row![c => "  ", "name", "version", "reviews", "notes"]);
    table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);

    let mut dependency_reports_iter = dependency_reports.iter();
    if first_row_separate {
        if let Some(dependency_report) = dependency_reports_iter.next() {
            let row = get_row(&dependency_report);
            table.add_row(row);
            table.add_row(prettytable::row![c => "  ", "", "", "", ""]);
        }
    }

    for dependency_report in dependency_reports_iter {
        let row = get_row(&dependency_report);
        table.add_row(row);
    }
    Ok(table)
}

fn get_note_cell(dependency_report: &report::DependencyReport) -> prettytable::Cell {
    let note = match &dependency_report.note {
        Some(v) => v.as_str(),
        None => "",
    };
    let mut note = prettytable::Cell::new_align(&note, prettytable::format::Alignment::LEFT);

    if dependency_report.summary == review::SecuritySummary::Critical {
        note = note
            .with_style(prettytable::Attr::BackgroundColor(
                prettytable::color::BRIGHT_RED,
            ))
            .with_style(prettytable::Attr::ForegroundColor(
                prettytable::color::BLACK,
            ));
    }
    note
}

impl From<review::SecuritySummary> for prettytable::Cell {
    fn from(summary: review::SecuritySummary) -> Self {
        let label = match summary {
            review::SecuritySummary::None => "      ",
            review::SecuritySummary::Low => " LOW  ",
            review::SecuritySummary::Medium => " MED  ",
            review::SecuritySummary::Critical => " CRIT ",
        };

        let background_color = match summary {
            review::SecuritySummary::None => None,
            review::SecuritySummary::Low => Some(prettytable::color::BRIGHT_GREEN),
            review::SecuritySummary::Medium => Some(prettytable::color::YELLOW),
            review::SecuritySummary::Critical => Some(prettytable::color::BRIGHT_RED),
        };

        if let Some(background_color) = background_color {
            prettytable::Cell::new_align(label, prettytable::format::Alignment::CENTER)
                .with_style(prettytable::Attr::BackgroundColor(background_color))
                .with_style(prettytable::Attr::ForegroundColor(
                    prettytable::color::BLACK,
                ))
        } else {
            prettytable::Cell::new_align(label, prettytable::format::Alignment::CENTER)
        }
    }
}
