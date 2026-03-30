use console::Style;

pub fn success(msg: &str) {
    let style = Style::new().green().bold();
    eprintln!("{}", style.apply_to(msg));
}

pub fn info(msg: &str) {
    let style = Style::new().cyan();
    eprintln!("{}", style.apply_to(msg));
}

pub fn warn(msg: &str) {
    let style = Style::new().yellow().bold();
    eprintln!("{}", style.apply_to(format!("warning: {msg}")));
}

pub fn label_value(label: &str, value: &str) {
    let label_style = Style::new().bold();
    println!("{} {}", label_style.apply_to(format!("{label}:")), value);
}

#[allow(clippy::cast_precision_loss)]
pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 5 + 512 * 1024), "5.5 MB");
    }

    #[test]
    fn test_display_functions_do_not_panic() {
        success("test");
        info("test");
        warn("test");
        label_value("key", "value");
    }
}
