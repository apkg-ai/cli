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
