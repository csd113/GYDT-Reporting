use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=assets/app-icon.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return Ok(());
    }

    for variable in ["RC", "PATH", "WindowsSdkVerBinPath", "ProgramFiles(x86)"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo did not supply OUT_DIR")?);
    let output = output_dir.join("gydt-app.res");
    let source = output_dir.join("gydt-app.rc");
    let version = env!("CARGO_PKG_VERSION");
    let parts = version
        .split('.')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()?;
    let [major, minor, patch] = parts.as_slice() else {
        return Err("Use a three-part numeric Windows version.".into());
    };
    let icon = fs::read_to_string("assets/app-icon.rc")?;
    fs::write(
        &source,
        format!(
            r#"{icon}
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004L
FILETYPE 0x1L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "ProductName", "GYDT Sample Reporting\0"
            VALUE "OriginalFilename", "GYDT Sample Reporting.exe\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
        ),
    )?;
    let compiler = resource_compiler();
    let status = Command::new(&compiler)
        .current_dir("assets")
        .args(["/nologo", "/fo"])
        .arg(&output)
        .arg(&source)
        .status()
        .map_err(|error| {
            format!(
                "Could not run Windows SDK resource compiler {}: {error}",
                compiler.display()
            )
        })?;
    if !status.success() {
        return Err("Windows SDK resource compilation failed".into());
    }
    println!(
        "cargo:rustc-link-arg-bin=gydt-results-compiler={}",
        output.display()
    );
    Ok(())
}

fn resource_compiler() -> PathBuf {
    if let Some(configured) = env::var_os("RC").filter(|value| !value.is_empty()) {
        return PathBuf::from(configured);
    }
    if let Some(found) = env::var_os("PATH").as_deref().and_then(|path| {
        env::split_paths(path)
            .map(|dir| dir.join("rc.exe"))
            .find(|rc| rc.is_file())
    }) {
        return found;
    }
    if let Some(found) = env::var_os("WindowsSdkVerBinPath")
        .map(PathBuf::from)
        .map(|dir| dir.join("x64").join("rc.exe"))
        .filter(|rc| rc.is_file())
    {
        return found;
    }
    env::var_os("ProgramFiles(x86)")
        .as_deref()
        .and_then(|root| sdk_compiler(Path::new(root)))
        .unwrap_or_else(|| PathBuf::from("rc.exe"))
}

fn sdk_compiler(root: &Path) -> Option<PathBuf> {
    fs::read_dir(root.join("Windows Kits").join("10").join("bin"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64").join("rc.exe"))
        .filter(|rc| rc.is_file())
        .max()
}
