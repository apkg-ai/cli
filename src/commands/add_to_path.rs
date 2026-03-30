use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::util::display;

const PATH_COMMENT: &str = "# Added by apkg";
const PATH_EXPORT: &str = r#"export PATH="$HOME/.apkg/bin:$PATH""#;
const FISH_PATH_LINE: &str = "fish_add_path $HOME/.apkg/bin";
const MARKER: &str = ".apkg/bin";

struct Paths {
    current_exe: PathBuf,
    bin_dir: PathBuf,
    target_bin: PathBuf,
}

enum Shell {
    Zsh,
    Bash,
    Fish,
}

pub fn run() -> Result<(), AppError> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Other("Cannot determine home directory".into()))?;

    let paths = resolve_paths(&home)?;
    ensure_binary(&paths)?;

    let shell = detect_shell();
    let rc_path = rc_file(&shell, &home);
    let modified = update_rc_file(&shell, &rc_path)?;

    print_instructions(&rc_path, modified);

    let bin_dir_str = paths.bin_dir.to_string_lossy();
    if std::env::var("PATH").is_ok_and(|p| p.contains(bin_dir_str.as_ref())) {
        display::info("~/.apkg/bin is already in your current PATH — you're all set!");
    }

    Ok(())
}

fn resolve_paths(home: &Path) -> Result<Paths, AppError> {
    let current_exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| AppError::Other(format!("Cannot determine current executable path: {e}")))?;

    let bin_dir = home.join(".apkg").join("bin");
    let target_bin = bin_dir.join("apkg");

    Ok(Paths {
        current_exe,
        bin_dir,
        target_bin,
    })
}

fn ensure_binary(paths: &Paths) -> Result<(), AppError> {
    fs::create_dir_all(&paths.bin_dir)?;

    if paths.current_exe == paths.target_bin {
        display::info("Binary is already at ~/.apkg/bin/apkg.");
        return Ok(());
    }

    // If target already exists, check if it already points to current exe
    if paths.target_bin.symlink_metadata().is_ok() {
        if let Ok(resolved) = fs::read_link(&paths.target_bin) {
            if resolved == paths.current_exe {
                display::info("~/.apkg/bin/apkg already points to the current binary.");
                return Ok(());
            }
        }
        fs::remove_file(&paths.target_bin)?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&paths.current_exe, &paths.target_bin)?;
    }
    #[cfg(windows)]
    {
        fs::copy(&paths.current_exe, &paths.target_bin)?;
    }

    display::success(&format!(
        "Linked {} -> {}",
        paths.target_bin.display(),
        paths.current_exe.display()
    ));
    Ok(())
}

fn detect_shell() -> Shell {
    if let Ok(shell) = std::env::var("SHELL") {
        if shell.contains("zsh") {
            return Shell::Zsh;
        }
        if shell.contains("fish") {
            return Shell::Fish;
        }
        if shell.contains("bash") {
            return Shell::Bash;
        }
    }
    if cfg!(target_os = "macos") {
        Shell::Zsh
    } else {
        Shell::Bash
    }
}

fn rc_file(shell: &Shell, home: &Path) -> PathBuf {
    match shell {
        Shell::Zsh => home.join(".zshrc"),
        Shell::Bash => {
            let bashrc = home.join(".bashrc");
            if bashrc.exists() {
                return bashrc;
            }
            let bash_profile = home.join(".bash_profile");
            if bash_profile.exists() {
                return bash_profile;
            }
            let profile = home.join(".profile");
            if profile.exists() {
                return profile;
            }
            bashrc
        }
        Shell::Fish => home.join(".config").join("fish").join("config.fish"),
    }
}

fn update_rc_file(shell: &Shell, rc_path: &Path) -> Result<bool, AppError> {
    let content = if rc_path.exists() {
        fs::read_to_string(rc_path)?
    } else {
        String::new()
    };

    if content.contains(MARKER) {
        return Ok(false);
    }

    if let Some(parent) = rc_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let line = match shell {
        Shell::Fish => FISH_PATH_LINE,
        _ => PATH_EXPORT,
    };

    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_path)?;

    writeln!(file)?;
    writeln!(file, "{PATH_COMMENT}")?;
    writeln!(file, "{line}")?;

    Ok(true)
}

fn print_instructions(rc_path: &Path, modified: bool) {
    if modified {
        display::success(&format!("Added PATH entry to {}", rc_path.display()));
    } else {
        display::info(&format!(
            "PATH entry already present in {}",
            rc_path.display()
        ));
    }

    eprintln!();
    eprintln!("To start using apkg, either:");
    eprintln!();
    eprintln!("  1. Open a new terminal window, or");
    eprintln!("  2. Run: source {}", rc_path.display());
    eprintln!();
}
