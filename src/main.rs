#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod desktop;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    if let Err(error) = desktop::run() {
        eprintln!("{}: {error}", desktop::APP_NAME);
        rfd::MessageDialog::new()
            .set_title(desktop::APP_NAME)
            .set_level(rfd::MessageLevel::Error)
            .set_description(error.to_string())
            .show();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("GYDT Sample Reporting desktop is available on macOS and Windows.");
    std::process::exit(1);
}
