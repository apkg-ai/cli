use crate::config::settings::Settings;
use crate::error::AppError;
use crate::util::display;

#[derive(Clone, Copy)]
pub enum ConfigAction<'a> {
    Set { key: &'a str, value: &'a str },
    Get { key: &'a str },
    List,
    Delete { key: &'a str },
}

pub fn run(action: ConfigAction<'_>) -> Result<(), AppError> {
    match action {
        ConfigAction::Set { key, value } => {
            validate_key(key)?;
            let mut settings = Settings::load()?;
            settings.set(key, value);
            settings.save()?;
            display::success(&format!("Set {key} = {value}"));
        }
        ConfigAction::Get { key } => {
            let settings = Settings::load()?;
            match settings.get(key) {
                Some(value) => println!("{value}"),
                None => {
                    return Err(AppError::Other(format!("Config key not set: {key}")));
                }
            }
        }
        ConfigAction::List => {
            let settings = Settings::load()?;
            let entries = settings.entries();
            if entries.is_empty() {
                display::info("No configuration set. Using defaults.");
            } else {
                for (key, value) in &entries {
                    display::label_value(key, value);
                }
            }
        }
        ConfigAction::Delete { key } => {
            let mut settings = Settings::load()?;
            if settings.delete(key) {
                settings.save()?;
                display::success(&format!("Deleted {key}"));
            } else {
                return Err(AppError::Other(format!("Config key not set: {key}")));
            }
        }
    }
    Ok(())
}

const KNOWN_SETUP_TOOLS: &[&str] = &["cursor", "claude-code", "windsurf", "kiro", "codex"];

fn validate_key(key: &str) -> Result<(), AppError> {
    if key == "registry" || key.starts_with("services.") {
        Ok(())
    } else if let Some(tool) = key.strip_prefix("defaultSetup.") {
        if KNOWN_SETUP_TOOLS.contains(&tool) {
            Ok(())
        } else {
            Err(AppError::Other(format!(
                "Unknown tool: {tool}\nValid tools: {}",
                KNOWN_SETUP_TOOLS.join(", ")
            )))
        }
    } else {
        Err(AppError::Other(format!(
            "Unknown config key: {key}\nValid keys: registry, services.<name>, defaultSetup.<tool> ({})",
            KNOWN_SETUP_TOOLS.join(", ")
        )))
    }
}
