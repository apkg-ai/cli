use console::Style;

use crate::config::cache;
use crate::error::AppError;
use crate::util::display;

const SIZE_COL_WIDTH: usize = 10;
const MIN_NAME_WIDTH: usize = 7; // len("Package")

#[derive(Clone, Copy)]
pub enum CacheAction {
    Clean,
    List,
    Verify,
}

struct ListRow {
    label: String,
    size: String,
    cached_at: String,
}

struct ListTable {
    name_width: usize,
    rows: Vec<ListRow>,
    summary: String,
}

struct VerifyReport {
    header: String,
    ok_line: String,
    warnings: Vec<String>,
    summary: String,
}

#[allow(clippy::cast_possible_truncation)]
fn format_clean_message(result: &cache::CacheCleanResult) -> String {
    if result.count == 0 {
        "Cache is already empty.".to_string()
    } else {
        format!(
            "Cleared {} cached package{} ({} freed).",
            result.count,
            if result.count == 1 { "" } else { "s" },
            display::format_size(result.bytes_freed as usize),
        )
    }
}

#[allow(clippy::cast_possible_truncation)]
fn format_list_table(entries: &[cache::CacheEntryInfo]) -> Option<ListTable> {
    if entries.is_empty() {
        return None;
    }

    let name_width = entries
        .iter()
        .map(|e| format!("{}@{}", e.name, e.version).len())
        .max()
        .unwrap_or(MIN_NAME_WIDTH)
        .max(MIN_NAME_WIDTH);

    let total_bytes: u64 = entries.iter().map(|e| e.size).sum();

    let rows = entries
        .iter()
        .map(|entry| {
            let label = format!("{}@{}", entry.name, entry.version);
            let size_width = SIZE_COL_WIDTH;
            let size = format!("{:>size_width$}", display::format_size(entry.size as usize));
            ListRow {
                label,
                size,
                cached_at: entry.cached_at.clone(),
            }
        })
        .collect();

    let summary = format!(
        "{} package{} cached ({} total)",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        display::format_size(total_bytes as usize),
    );

    Some(ListTable {
        name_width,
        rows,
        summary,
    })
}

fn format_verify_report(result: &cache::CacheVerifyResult) -> Option<VerifyReport> {
    if result.checked == 0 {
        return None;
    }

    let header = format!(
        "Checked {} package{}:",
        result.checked,
        if result.checked == 1 { "" } else { "s" }
    );

    let ok_line = format!("  {} ok", result.ok);

    let warnings = result
        .corrupted
        .iter()
        .map(|label| format!("corrupted ({label} — hash mismatch, removed)"))
        .collect();

    let summary = if result.corrupted.is_empty() {
        "All cached packages verified successfully.".to_string()
    } else {
        format!("  {} corrupted (removed)", result.corrupted.len())
    };

    Some(VerifyReport {
        header,
        ok_line,
        warnings,
        summary,
    })
}

pub fn run(action: CacheAction) -> Result<(), AppError> {
    match action {
        CacheAction::Clean => run_clean(),
        CacheAction::List => run_list(),
        CacheAction::Verify => run_verify(),
    }
}

fn run_clean() -> Result<(), AppError> {
    let result = cache::clean()?;
    let msg = format_clean_message(&result);

    if result.count == 0 {
        display::info(&msg);
    } else {
        display::success(&msg);
    }

    Ok(())
}

fn run_list() -> Result<(), AppError> {
    let entries = cache::list_entries()?;

    let Some(table) = format_list_table(&entries) else {
        display::info("Cache is empty.");
        return Ok(());
    };

    let header_style = Style::new().bold().underlined();
    let name_style = Style::new().cyan();
    let name_width = table.name_width;
    let size_width = SIZE_COL_WIDTH;

    println!(
        "{}  {}  {}",
        header_style.apply_to(format!("{:<name_width$}", "Package")),
        header_style.apply_to(format!("{:>size_width$}", "Size")),
        header_style.apply_to("Cached At"),
    );

    for row in &table.rows {
        println!(
            "{}  {}  {}",
            name_style.apply_to(format!("{:<name_width$}", row.label)),
            row.size,
            row.cached_at,
        );
    }

    println!();
    display::info(&table.summary);

    Ok(())
}

fn run_verify() -> Result<(), AppError> {
    let result = cache::verify()?;

    let Some(report) = format_verify_report(&result) else {
        display::info("Cache is empty, nothing to verify.");
        return Ok(());
    };

    println!("{}", report.header);
    println!("{}", report.ok_line);

    for warning in &report.warnings {
        display::warn(warning);
    }

    if result.corrupted.is_empty() {
        display::success(&report.summary);
    } else {
        println!("{}", report.summary);
    }

    Ok(())
}

// NOTE: Tests for run() that modify APKG_CACHE_DIR are in tests/cli.rs
// to avoid env var race conditions with config::cache tests.

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str, version: &str, size: u64, cached_at: &str) -> cache::CacheEntryInfo {
        cache::CacheEntryInfo {
            name: name.to_string(),
            version: version.to_string(),
            size,
            cached_at: cached_at.to_string(),
        }
    }

    // --- format_clean_message ---

    #[test]
    fn test_format_clean_empty() {
        let result = cache::CacheCleanResult {
            count: 0,
            bytes_freed: 0,
        };
        assert_eq!(format_clean_message(&result), "Cache is already empty.");
    }

    #[test]
    fn test_format_clean_single() {
        let result = cache::CacheCleanResult {
            count: 1,
            bytes_freed: 512,
        };
        let msg = format_clean_message(&result);
        assert!(msg.starts_with("Cleared 1 cached package "));
        assert!(!msg.contains("packages"));
        assert!(msg.contains("512 B"));
    }

    #[test]
    fn test_format_clean_multiple() {
        let result = cache::CacheCleanResult {
            count: 5,
            bytes_freed: 1536,
        };
        let msg = format_clean_message(&result);
        assert!(msg.starts_with("Cleared 5 cached packages"));
        assert!(msg.contains("1.5 KB"));
    }

    #[test]
    fn test_format_clean_large_size() {
        let result = cache::CacheCleanResult {
            count: 10,
            bytes_freed: 5 * 1024 * 1024,
        };
        let msg = format_clean_message(&result);
        assert!(msg.contains("10 cached packages"));
        assert!(msg.contains("5.0 MB"));
    }

    // --- format_list_table ---

    #[test]
    fn test_format_list_empty() {
        let entries: Vec<cache::CacheEntryInfo> = vec![];
        assert!(format_list_table(&entries).is_none());
    }

    #[test]
    fn test_format_list_single_entry() {
        let entries = vec![make_entry("foo", "1.0.0", 1024, "2025-01-01T00:00:00Z")];
        let table = format_list_table(&entries).unwrap();

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].label, "foo@1.0.0");
        assert_eq!(table.rows[0].cached_at, "2025-01-01T00:00:00Z");
        assert!(table.name_width >= MIN_NAME_WIDTH);
        assert!(table.summary.contains("1 package cached"));
        assert!(!table.summary.contains("packages"));
    }

    #[test]
    fn test_format_list_multiple_entries() {
        let entries = vec![
            make_entry("foo", "1.0.0", 1024, "2025-01-01T00:00:00Z"),
            make_entry("bar", "2.0.0", 2048, "2025-01-02T00:00:00Z"),
        ];
        let table = format_list_table(&entries).unwrap();

        assert_eq!(table.rows.len(), 2);
        assert!(table.summary.contains("2 packages cached"));
        assert!(table.summary.contains("3.0 KB"));
    }

    #[test]
    fn test_format_list_name_width_minimum() {
        let entries = vec![make_entry("a", "1", 0, "")];
        let table = format_list_table(&entries).unwrap();
        assert_eq!(table.name_width, MIN_NAME_WIDTH);
    }

    #[test]
    fn test_format_list_name_width_long() {
        let entries = vec![make_entry("@my-org/long-package-name", "12.34.56", 0, "")];
        let table = format_list_table(&entries).unwrap();
        let label_len = "@my-org/long-package-name@12.34.56".len();
        assert_eq!(table.name_width, label_len);
    }

    #[test]
    fn test_format_list_size_right_aligned() {
        let entries = vec![make_entry("foo", "1.0.0", 512, "")];
        let table = format_list_table(&entries).unwrap();
        assert_eq!(table.rows[0].size.len(), SIZE_COL_WIDTH);
    }

    // --- format_verify_report ---

    #[test]
    fn test_format_verify_empty() {
        let result = cache::CacheVerifyResult {
            checked: 0,
            ok: 0,
            corrupted: vec![],
        };
        assert!(format_verify_report(&result).is_none());
    }

    #[test]
    fn test_format_verify_single_ok() {
        let result = cache::CacheVerifyResult {
            checked: 1,
            ok: 1,
            corrupted: vec![],
        };
        let report = format_verify_report(&result).unwrap();
        assert_eq!(report.header, "Checked 1 package:");
        assert_eq!(report.ok_line, "  1 ok");
        assert!(report.warnings.is_empty());
        assert_eq!(report.summary, "All cached packages verified successfully.");
    }

    #[test]
    fn test_format_verify_multiple_ok() {
        let result = cache::CacheVerifyResult {
            checked: 3,
            ok: 3,
            corrupted: vec![],
        };
        let report = format_verify_report(&result).unwrap();
        assert_eq!(report.header, "Checked 3 packages:");
        assert_eq!(report.ok_line, "  3 ok");
        assert!(report.warnings.is_empty());
        assert_eq!(report.summary, "All cached packages verified successfully.");
    }

    #[test]
    fn test_format_verify_some_corrupted() {
        let result = cache::CacheVerifyResult {
            checked: 3,
            ok: 1,
            corrupted: vec!["foo@1.0".to_string(), "bar@2.0".to_string()],
        };
        let report = format_verify_report(&result).unwrap();
        assert_eq!(report.header, "Checked 3 packages:");
        assert_eq!(report.ok_line, "  1 ok");
        assert_eq!(report.warnings.len(), 2);
        assert!(report.warnings[0].contains("foo@1.0"));
        assert!(report.warnings[1].contains("bar@2.0"));
        assert_eq!(report.summary, "  2 corrupted (removed)");
    }

    #[test]
    fn test_format_verify_all_corrupted() {
        let result = cache::CacheVerifyResult {
            checked: 2,
            ok: 0,
            corrupted: vec!["a@1.0".to_string(), "b@2.0".to_string()],
        };
        let report = format_verify_report(&result).unwrap();
        assert_eq!(report.ok_line, "  0 ok");
        assert_eq!(report.warnings.len(), 2);
        assert_eq!(report.summary, "  2 corrupted (removed)");
    }

    #[test]
    fn test_format_verify_single_corrupted() {
        let result = cache::CacheVerifyResult {
            checked: 1,
            ok: 0,
            corrupted: vec!["x@1.0".to_string()],
        };
        let report = format_verify_report(&result).unwrap();
        assert_eq!(report.header, "Checked 1 package:");
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("x@1.0"));
        assert_eq!(report.summary, "  1 corrupted (removed)");
    }
}
