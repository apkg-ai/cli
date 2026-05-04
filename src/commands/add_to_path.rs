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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::env_lock;

    #[test]
    fn test_detect_shell_zsh() {
        let _lock = env_lock();
        unsafe { std::env::set_var("SHELL", "/bin/zsh") };
        assert!(matches!(detect_shell(), Shell::Zsh));
    }

    #[test]
    fn test_detect_shell_bash() {
        let _lock = env_lock();
        unsafe { std::env::set_var("SHELL", "/usr/bin/bash") };
        assert!(matches!(detect_shell(), Shell::Bash));
    }

    #[test]
    fn test_detect_shell_fish() {
        let _lock = env_lock();
        unsafe { std::env::set_var("SHELL", "/usr/bin/fish") };
        assert!(matches!(detect_shell(), Shell::Fish));
    }

    #[test]
    fn test_detect_shell_fallback() {
        let _lock = env_lock();
        unsafe { std::env::remove_var("SHELL") };
        // On macOS defaults to Zsh, on Linux defaults to Bash
        let shell = detect_shell();
        if cfg!(target_os = "macos") {
            assert!(matches!(shell, Shell::Zsh));
        } else {
            assert!(matches!(shell, Shell::Bash));
        }
    }

    #[test]
    fn test_rc_file_zsh() {
        let tmp = tempfile::tempdir().unwrap();
        let path = rc_file(&Shell::Zsh, tmp.path());
        assert_eq!(path, tmp.path().join(".zshrc"));
    }

    #[test]
    fn test_rc_file_fish() {
        let tmp = tempfile::tempdir().unwrap();
        let path = rc_file(&Shell::Fish, tmp.path());
        assert_eq!(path, tmp.path().join(".config/fish/config.fish"));
    }

    #[test]
    fn test_rc_file_bash_no_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = rc_file(&Shell::Bash, tmp.path());
        assert_eq!(path, tmp.path().join(".bashrc"));
    }

    #[test]
    fn test_rc_file_bash_bashrc_exists() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".bashrc"), "").unwrap();
        let path = rc_file(&Shell::Bash, tmp.path());
        assert_eq!(path, tmp.path().join(".bashrc"));
    }

    #[test]
    fn test_rc_file_bash_only_bash_profile() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".bash_profile"), "").unwrap();
        let path = rc_file(&Shell::Bash, tmp.path());
        assert_eq!(path, tmp.path().join(".bash_profile"));
    }

    #[test]
    fn test_rc_file_bash_only_profile() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".profile"), "").unwrap();
        let path = rc_file(&Shell::Bash, tmp.path());
        assert_eq!(path, tmp.path().join(".profile"));
    }

    #[test]
    fn test_update_rc_file_creates_new() {
        let tmp = tempfile::tempdir().unwrap();
        let rc_path = tmp.path().join(".zshrc");
        let result = update_rc_file(&Shell::Zsh, &rc_path).unwrap();
        assert!(result);
        let content = fs::read_to_string(&rc_path).unwrap();
        assert!(content.contains(PATH_COMMENT));
        assert!(content.contains(PATH_EXPORT));
    }

    #[test]
    fn test_update_rc_file_appends_to_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let rc_path = tmp.path().join(".zshrc");
        fs::write(&rc_path, "# existing content\n").unwrap();
        let result = update_rc_file(&Shell::Zsh, &rc_path).unwrap();
        assert!(result);
        let content = fs::read_to_string(&rc_path).unwrap();
        assert!(content.contains("# existing content"));
        assert!(content.contains(PATH_EXPORT));
    }

    #[test]
    fn test_update_rc_file_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let rc_path = tmp.path().join(".zshrc");
        fs::write(&rc_path, "some content\n.apkg/bin\n").unwrap();
        let result = update_rc_file(&Shell::Zsh, &rc_path).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_update_rc_file_fish() {
        let tmp = tempfile::tempdir().unwrap();
        let rc_path = tmp.path().join("config.fish");
        let result = update_rc_file(&Shell::Fish, &rc_path).unwrap();
        assert!(result);
        let content = fs::read_to_string(&rc_path).unwrap();
        assert!(content.contains(FISH_PATH_LINE));
        assert!(!content.contains(PATH_EXPORT));
    }

    #[test]
    fn test_update_rc_file_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let rc_path = tmp.path().join(".config").join("fish").join("config.fish");
        let result = update_rc_file(&Shell::Fish, &rc_path).unwrap();
        assert!(result);
        assert!(rc_path.exists());
    }

    #[test]
    fn test_print_instructions_modified() {
        let path = PathBuf::from("/tmp/.zshrc");
        print_instructions(&path, true);
    }

    #[test]
    fn test_print_instructions_not_modified() {
        let path = PathBuf::from("/tmp/.zshrc");
        print_instructions(&path, false);
    }

    #[test]
    fn test_resolve_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = resolve_paths(tmp.path()).unwrap();
        assert_eq!(paths.bin_dir, tmp.path().join(".apkg").join("bin"));
        assert_eq!(
            paths.target_bin,
            tmp.path().join(".apkg").join("bin").join("apkg")
        );
    }

    #[test]
    fn test_ensure_binary_creates_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
        let paths = Paths {
            current_exe: exe.clone(),
            bin_dir: tmp.path().join(".apkg").join("bin"),
            target_bin: tmp.path().join(".apkg").join("bin").join("apkg"),
        };
        ensure_binary(&paths).unwrap();
        assert!(paths.target_bin.exists() || paths.target_bin.symlink_metadata().is_ok());
    }

    #[test]
    fn test_ensure_binary_already_linked() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
        let bin_dir = tmp.path().join(".apkg").join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let target = bin_dir.join("apkg");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&exe, &target).unwrap();
        #[cfg(windows)]
        fs::copy(&exe, &target).unwrap();

        let paths = Paths {
            current_exe: exe,
            bin_dir,
            target_bin: target,
        };
        // Should succeed without error (idempotent)
        ensure_binary(&paths).unwrap();
    }
}
