use crate::config::credentials;
use crate::error::AppError;
use crate::util::display;

pub fn run() -> Result<(), AppError> {
    if credentials::remove()? {
        display::success("Logged out successfully.");
    } else {
        display::info("Not logged in — nothing to do.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::credentials::Credentials;
    use crate::test_utils::env_lock;

    #[test]
    fn test_run_when_not_logged_in() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        run().unwrap();
        // No credentials file existed and none created.
        assert!(!tmp.path().join(".apkg").join("credentials.json").exists());
    }

    #[test]
    fn test_run_removes_existing_credentials() {
        let _lock = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", tmp.path()) };

        credentials::save(&Credentials {
            registry: "https://registry.apkg.ai/api/v1".to_string(),
            access_token: "tok".to_string(),
            refresh_token: "rt".to_string(),
            username: "user".to_string(),
        })
        .unwrap();

        run().unwrap();

        assert!(!tmp.path().join(".apkg").join("credentials.json").exists());
    }
}
