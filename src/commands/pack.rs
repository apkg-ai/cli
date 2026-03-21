use std::env;

use crate::config::manifest;
use crate::error::AppError;
use crate::util::{display, integrity, tarball};

pub fn run() -> Result<(), AppError> {
    let cwd = env::current_dir()?;
    let m = manifest::load(&cwd)?;

    display::info(&format!("Packing {}@{} ...", m.name, m.version));

    let data = tarball::create_tarball(&cwd)?;
    let hash = integrity::sha256_integrity(&data);
    let size = data.len();

    let filename = format!(
        "{}-{}.tgz",
        m.name.replace('/', "-").replace('@', ""),
        m.version
    );
    let out_path = cwd.join(&filename);
    tarball::write_tarball(&out_path, &data)?;

    display::success(&format!("Packed {filename}"));
    display::label_value("Size", &display::format_size(size));
    display::label_value("Integrity", &hash);
    println!("{}", out_path.display());

    Ok(())
}
