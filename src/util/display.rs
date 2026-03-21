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
