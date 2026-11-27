use std::{
    fs::{self, OpenOptions},
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{ExportError, package::filename_stem};

pub const MAX_OPUS_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_BATCH_OPUS_BYTES: usize = 32 * 1024 * 1024;

/// Reusable, validated source directory for one export preparation.
pub struct OpusSource {
    directory: PathBuf,
    pub(crate) identity: Arc<same_file::Handle>,
}

impl OpusSource {
    /// # Errors
    /// Returns an error if the source is unavailable or is also the export
    /// destination.
    pub fn new(source: &Path, destination: &Path) -> Result<Self, ExportError> {
        let source = source.canonicalize().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ExportError::Opus(
                    "The OPUS folder is missing. Choose an existing OPUS folder in Settings."
                        .into(),
                )
            } else {
                ExportError::Opus(format!("Can't open the OPUS folder. {e}"))
            }
        })?;
        if !source.is_dir() {
            return Err(ExportError::Opus(
                "Choose an existing OPUS folder in Settings.".into(),
            ));
        }
        let folder_error = |e| {
            ExportError::Opus(format!(
                "Can't verify that the OPUS and export folders are separate. {e}"
            ))
        };
        let identity = same_file::Handle::from_path(&source).map_err(folder_error)?;
        if identity == same_file::Handle::from_path(destination).map_err(folder_error)? {
            return Err(ExportError::Opus("Choose an export folder different from the OPUS folder. The source won't be overwritten.".into()));
        }
        Ok(Self {
            directory: source,
            identity: Arc::new(identity),
        })
    }

    pub(crate) fn snapshot(&self, title: &str) -> Result<(String, Vec<u8>), ExportError> {
        let name = member_name(title).ok_or_else(|| {
            ExportError::Opus("The submission number cannot be used as an exact OPUS filename. Use a safe submission number.".into())
        })?;
        // Compare the actual directory entry name, even on case-insensitive volumes.
        let mut exact = None;
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| ExportError::Opus(format!("Can't read the OPUS folder. {e}")))?
        {
            let entry =
                entry.map_err(|e| ExportError::Opus(format!("Can't read the OPUS folder. {e}")))?;
            if entry.file_name() == std::ffi::OsStr::new(&name) {
                exact = Some(entry.path());
                break;
            }
        }
        let path = exact.ok_or_else(|| missing(&name))?;
        let before = fs::symlink_metadata(&path).map_err(|e| read_error(&name, &e))?;
        if !regular(&before) {
            return Err(ExportError::Opus(format!(
                "{name} must be a regular OPUS file, not a link."
            )));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // Read-only sharing prevents concurrent write/delete during snapshot;
            // open the reparse point itself rather than following a replacement link.
            options.share_mode(1).custom_flags(0x0020_0000);
        }
        let mut file = options.open(&path).map_err(|e| read_error(&name, &e))?;
        let metadata = file.metadata().map_err(|e| read_error(&name, &e))?;
        if !regular(&metadata) || !same_open_file(&before, &metadata) {
            return Err(ExportError::Opus(format!(
                "{name} changed while opening it. Try Export again."
            )));
        }
        let size = metadata.len();
        if size == 0 || size > MAX_OPUS_BYTES {
            return Err(ExportError::Opus(format!(
                "{name} must be nonempty and no larger than 4 MiB."
            )));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_OPUS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| read_error(&name, &e))?;
        let after = file.metadata().map_err(|e| read_error(&name, &e))?;
        if bytes.len() as u64 != size
            || after.len() != size
            || after.modified().ok() != metadata.modified().ok()
        {
            return Err(ExportError::Opus(format!(
                "{name} changed while reading it. Unload it in OPUS, then click Export again."
            )));
        }
        Ok((name, bytes))
    }
}

pub(crate) fn member_name(title: &str) -> Option<String> {
    (filename_stem(title) == title && !title.split('.').next().unwrap_or_default().ends_with(' '))
        .then(|| format!("{title}.0"))
}

fn regular(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_file()
}

fn same_open_file(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        // Windows opens without following reparse points and denies writes and
        // deletes to the opened handle until the snapshot is complete.
        before.len() == after.len() && before.modified().ok() == after.modified().ok()
    }
}

fn missing(name: &str) -> ExportError {
    ExportError::Opus(format!(
        "Can't find {name} in the OPUS folder. Check the exact submission number and export the spectrum from OPUS."
    ))
}

fn read_error(name: &str, error: &io::Error) -> ExportError {
    match error.kind() {
        io::ErrorKind::NotFound => missing(name),
        _ if matches!(error.raw_os_error(), Some(32 | 33)) && cfg!(windows)
            || error.kind() == io::ErrorKind::PermissionDenied =>
        {
            ExportError::Opus(format!(
                "Can't read {name}. Unload {name} in OPUS, then click Export again. If it still fails, check file permissions."
            ))
        }
        _ => ExportError::Opus(format!("Can't read {name} from the OPUS folder. {error}")),
    }
}
