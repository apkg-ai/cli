use console::Style;

use crate::config::cache;
use crate::error::AppError;
use crate::util::display;

#[derive(Clone, Copy)]
pub enum CacheAction {
    Clean,
    List,
    Verify,
}

pub fn run(action: CacheAction) -> Result<(), AppError> {
    match action {
        CacheAction::Clean => run_clean(),
        CacheAction::List => run_list(),
        CacheAction::Verify => run_verify(),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn run_clean() -> Result<(), AppError> {
    let result = cache::clean()?;

    if result.count == 0 {
        display::info("Cache is already empty.");
    } else {
        display::success(&format!(
            "Cleared {} cached package{} ({} freed).",
            result.count,
            if result.count == 1 { "" } else { "s" },
            display::format_size(result.bytes_freed as usize),
        ));
    }

    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn run_list() -> Result<(), AppError> {
    let entries = cache::list_entries()?;

    if entries.is_empty() {
        display::info("Cache is empty.");
        return Ok(());
    }

    let header_style = Style::new().bold().underlined();
    let name_style = Style::new().cyan();

    // Column widths
    let name_width = entries
        .iter()
        .map(|e| format!("{}@{}", e.name, e.version).len())
        .max()
        .unwrap_or(7)
        .max(7);
    let size_width = 10;

    println!(
        "{}  {}  {}",
        header_style.apply_to(format!("{:<name_width$}", "Package")),
        header_style.apply_to(format!("{:>size_width$}", "Size")),
        header_style.apply_to("Cached At"),
    );

    let mut total_bytes: u64 = 0;
    for entry in &entries {
        let label = format!("{}@{}", entry.name, entry.version);
        let size_str = format!("{:>size_width$}", display::format_size(entry.size as usize));
        total_bytes += entry.size;

        println!(
            "{}  {size_str}  {}",
            name_style.apply_to(format!("{label:<name_width$}")),
            entry.cached_at,
        );
    }

    println!();
    display::info(&format!(
        "{} package{} cached ({} total)",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        display::format_size(total_bytes as usize),
    ));

    Ok(())
}

fn run_verify() -> Result<(), AppError> {
    let result = cache::verify()?;

    if result.checked == 0 {
        display::info("Cache is empty, nothing to verify.");
        return Ok(());
    }

    println!(
        "Checked {} package{}:",
        result.checked,
        if result.checked == 1 { "" } else { "s" }
    );
    println!("  {} ok", result.ok);

    for label in &result.corrupted {
        display::warn(&format!(
            "corrupted ({label} — hash mismatch, removed)"
        ));
    }

    if result.corrupted.is_empty() {
        display::success("All cached packages verified successfully.");
    } else {
        println!("  {} corrupted (removed)", result.corrupted.len());
    }

    Ok(())
}
