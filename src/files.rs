use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    time::SystemTime,
};

use same_file::Handle;
use sha2::{Digest as _, Sha256};
use tempfile::{NamedTempFile, TempDir};

use crate::PreparedExport;

const STAGING_PREFIX: &str = ".gydt-export-";
const STAGING_SUFFIX_LEN: usize = 6;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("Export files already exist or the approved conflicts changed in {0}")]
    Conflicts(ConflictSnapshot),
    #[error("Unable to inspect or prepare the export")]
    Prepare {
        path: PathBuf,
        #[source]
        error: io::Error,
    },
    #[error("An export is using this folder. Wait for it to finish, then try again.")]
    ExportInProgress,
    #[error(transparent)]
    RecoveryRequired(#[from] RecoveryRequired),
    #[error(transparent)]
    Interrupted(#[from] WriteFailure),
    #[error("Export failed. {warning}")]
    Cleanup {
        #[source]
        failure: Box<Self>,
        warning: CleanupWarning,
    },
}

/// Files from a previous overwrite that staff must reconcile before retrying.
#[derive(thiserror::Error)]
#[error("Review the retained originals in {} before retrying. Move them to a private location or restore them to vacant names.", recovery_locations(.originals))]
pub struct RecoveryRequired {
    originals: Vec<PathBuf>,
}

impl RecoveryRequired {
    /// Exact paths for the caller to show to authorized staff when needed.
    #[must_use]
    pub fn originals(&self) -> &[PathBuf] {
        &self.originals
    }
}

fn recovery_locations(originals: &[PathBuf]) -> String {
    originals
        .iter()
        .filter_map(|path| path.parent()?.parent()?.file_name())
        .map(|name| format!("{}/originals", name.to_string_lossy()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Debug for RecoveryRequired {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, output)
    }
}

/// A completed export can still need manual staging cleanup.
#[derive(Debug)]
pub struct WriteSuccess {
    pub written: Vec<PathBuf>,
    pub cleanup_warning: Option<CleanupWarning>,
}

#[derive(Debug, thiserror::Error)]
#[error("Export staging cleanup failed in {}", .directory.file_name().unwrap_or_default().to_string_lossy())]
pub struct CleanupWarning {
    pub directory: PathBuf,
    #[source]
    pub error: io::Error,
}

/// A write stopped after staging. Earlier outputs may already be installed.
#[derive(Debug, thiserror::Error)]
#[error("Export stopped after staging. {}", recovery_message(.recovery.as_deref()))]
pub struct WriteFailure {
    #[source]
    pub error: io::Error,
    pub written: Vec<PathBuf>,
    /// A private directory containing originals under their original filenames.
    pub recovery: Option<PathBuf>,
}

fn recovery_message(directory: Option<&Path>) -> String {
    directory.map_or_else(
        || "Review any outputs already written.".into(),
        |path| {
            format!(
                "Review the originals in {}/originals before retrying.",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        },
    )
}

/// Existing output files at the time staff are asked for overwrite approval.
#[derive(Debug, PartialEq, Eq)]
pub struct ConflictSnapshot {
    directory: PathBuf,
    identity: Handle,
    names: Vec<String>,
    existing: BTreeMap<String, Fingerprint>,
}

impl ConflictSnapshot {
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn filenames(&self) -> impl ExactSizeIterator<Item = &str> {
        self.existing.keys().map(String::as_str)
    }
}

impl fmt::Display for ConflictSnapshot {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(output, "{}", self.directory.display())?;
        if !self.existing.is_empty() {
            write!(
                output,
                " ({})",
                self.filenames().collect::<Vec<_>>().join(", ")
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    identity: Handle,
    len: u64,
    modified: Option<SystemTime>,
    digest: [u8; 32],
}

impl PreparedExport {
    /// Finds and fingerprints the existing targets without changing the export folder.
    ///
    /// # Errors
    /// Returns an error for an unavailable directory, inaccessible output, or
    /// conflicting path that is not a regular file. Also reports active writes
    /// and retained originals for the selected outputs.
    pub fn inspect_folder(&self, folder: &Path) -> Result<ConflictSnapshot, WriteError> {
        self.inspect_guarded(folder, false)
            .map(|(snapshot, _)| snapshot)
    }

    fn inspect_guarded(
        &self,
        folder: &Path,
        exclusive: bool,
    ) -> Result<(ConflictSnapshot, Handle), WriteError> {
        let (directory, guard) = open_directory(folder)?;
        #[cfg(unix)]
        lock_directory(&directory, &guard, exclusive)?;
        #[cfg(not(unix))]
        let _ = exclusive;
        let snapshot = self.inspect_destination(&directory)?;
        if snapshot.directory != directory || snapshot.identity != guard {
            return Err(preparation(
                &directory,
                io::Error::other("The export folder changed."),
            ));
        }
        Ok((snapshot, guard))
    }

    fn inspect_destination(&self, folder: &Path) -> Result<ConflictSnapshot, WriteError> {
        let (directory, identity) = open_directory(folder)?;
        let names: Vec<_> = self.filenames().map(str::to_owned).collect();
        let originals = retained_originals(&directory, &names)
            .map_err(|error| preparation(&directory, error))?;
        if !originals.is_empty() {
            return Err(RecoveryRequired { originals }.into());
        }
        let mut existing = BTreeMap::new();
        for name in &names {
            let path = directory.join(name);
            if let Some(file) = fingerprint(&path).map_err(|error| preparation(&path, error))? {
                existing.insert(name.clone(), file);
            }
        }
        Ok(ConflictSnapshot {
            directory,
            identity,
            names,
            existing,
        })
    }

    /// Writes prepared outputs. Pass the exact conflict snapshot only after
    /// staff explicitly approve replacing those files.
    /// # Errors
    /// Returns active-write or recovery status, fresh conflicts if approval is
    /// missing or stale, or a failure listing any files already installed.
    /// Staging cleanup failures are reported separately from the write outcome.
    pub fn write(
        &self,
        folder: &Path,
        approval: Option<&ConflictSnapshot>,
    ) -> Result<WriteSuccess, WriteError> {
        let (current, _guard) = self.inspect_guarded(folder, true)?;
        if approval.map_or(!current.existing.is_empty(), |approved| {
            *approved != current
        }) {
            return Err(WriteError::Conflicts(current));
        }

        let mut stage = Staging::new(&current)?;
        let result = self.publish(folder, &mut stage);
        stage.finish(result)
    }

    fn publish(&self, folder: &Path, stage: &mut Staging<'_>) -> Result<(), WriteError> {
        let staged = stage.prepare(&self.outputs)?;
        // The write-side destination lock is already held.
        let fresh = self.inspect_destination(folder)?;
        if fresh != *stage.destination {
            return Err(WriteError::Conflicts(fresh));
        }
        for (output, file) in self.outputs.iter().zip(staged) {
            if let Err(error) = stage.install(&output.name, file) {
                return Err(stage.interruption(error));
            }
        }
        Ok(())
    }
}

fn open_directory(folder: &Path) -> Result<(PathBuf, Handle), WriteError> {
    let directory = fs::canonicalize(folder).map_err(|error| preparation(folder, error))?;
    if !directory.is_dir() {
        return Err(preparation(
            folder,
            io::Error::other("Choose an existing folder."),
        ));
    }
    let identity = Handle::from_path(&directory).map_err(|error| preparation(&directory, error))?;
    Ok((directory, identity))
}

// Keep the opened directory locked through inspection, publication and cleanup.
// A directory handle leaves no lock file to remove, even after an interruption.
#[cfg(unix)]
fn lock_directory(directory: &Path, guard: &Handle, exclusive: bool) -> Result<(), WriteError> {
    let lock = if exclusive {
        guard.as_file().try_lock()
    } else {
        guard.as_file().try_lock_shared()
    };
    match lock {
        Ok(()) => Ok(()),
        Err(fs::TryLockError::WouldBlock) => Err(WriteError::ExportInProgress),
        Err(fs::TryLockError::Error(error)) => Err(preparation(directory, error)),
    }
}

fn retained_originals(directory: &Path, names: &[String]) -> io::Result<Vec<PathBuf>> {
    // This is also the layout used before the retry gate was added. Restrict
    // recovery to the reserved staging name and exact planned filenames.
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(suffix) = name
            .to_str()
            .and_then(|name| name.strip_prefix(STAGING_PREFIX))
        else {
            continue;
        };
        if suffix.len() != STAGING_SUFFIX_LEN
            || !suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            continue;
        }
        let path = entry.path();
        let Some(metadata) = entry_metadata(&path)? else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                continue;
            }
        }
        let originals = path.join("originals");
        if !entry_metadata(&originals)?.is_some_and(|metadata| metadata.is_dir()) {
            continue;
        }
        for name in names {
            let original = originals.join(name);
            if entry_metadata(&original)?.is_some() {
                found.push(original);
            }
        }
    }
    found.sort_unstable();
    Ok(found)
}

struct Staging<'a> {
    directory: TempDir,
    identity: Handle,
    destination: &'a ConflictSnapshot,
    originals: Vec<Original<'a>>,
    written: Vec<PathBuf>,
}

struct Original<'a> {
    target: PathBuf,
    saved: PathBuf,
    expected: &'a Fingerprint,
    replacement: Option<Handle>,
    released: bool,
}

impl<'a> Staging<'a> {
    fn new(destination: &'a ConflictSnapshot) -> Result<Self, WriteError> {
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let directory = builder
            .prefix(STAGING_PREFIX)
            .rand_bytes(STAGING_SUFFIX_LEN)
            .disable_cleanup(true)
            .tempdir_in(&destination.directory)
            .map_err(|error| preparation(&destination.directory, error))?;
        let identity = match Handle::from_path(directory.path()) {
            Ok(identity) => identity,
            Err(error) => {
                let failure = preparation(&destination.directory, error);
                return Err(with_warning(
                    failure,
                    Some(CleanupWarning {
                        directory: directory.keep(),
                        error: io::Error::other("The staging directory could not be verified."),
                    }),
                ));
            }
        };
        Ok(Staging {
            directory,
            identity,
            destination,
            originals: Vec::new(),
            written: Vec::new(),
        })
    }

    fn prepare(
        &self,
        outputs: &[crate::package::Artifact],
    ) -> Result<Vec<NamedTempFile>, WriteError> {
        let root = self.directory.path();
        let pending = root.join("pending");
        self.check()
            .and_then(|()| fs::create_dir(&pending))
            .map_err(|error| preparation(root, error))?;
        if !self.destination.existing.is_empty() {
            fs::create_dir(root.join("originals"))
                .and_then(|()| sync_directory(root))
                .and_then(|()| sync_directory(&self.destination.directory))
                .map_err(|error| preparation(root, error))?;
        }
        outputs
            .iter()
            .map(|output| {
                let path = self.destination.directory.join(&output.name);
                let mut file = tempfile::Builder::new()
                    .disable_cleanup(true)
                    .tempfile_in(&pending)
                    .map_err(|error| preparation(&path, error))?;
                file.write_all(&output.bytes)
                    .and_then(|()| file.as_file().sync_all())
                    .map_err(|error| preparation(&path, error))?;
                Ok(file)
            })
            .collect()
    }

    fn check(&self) -> io::Result<()> {
        check_directory(&self.destination.directory, &self.destination.identity)?;
        check_directory(self.directory.path(), &self.identity)
    }

    fn capture(&mut self, name: &str, expected: &'a Fingerprint) -> io::Result<()> {
        let target = self.destination.directory.join(name);
        let saved = self.directory.path().join("originals").join(name);
        fs::rename(&target, &saved)?;
        // The old name can disappear in this step. Its bytes remain under the same
        // filename in originals, even if the process exits before it can report.
        self.originals.push(Original {
            target,
            saved,
            expected,
            replacement: None,
            released: false,
        });
        sync_directory(&self.directory.path().join("originals"))?;
        sync_directory(&self.destination.directory)?;
        self.originals
            .last()
            .ok_or_else(|| io::Error::other("The original was not recorded."))?
            .check_saved()
    }

    fn install(&mut self, name: &str, file: NamedTempFile) -> io::Result<()> {
        self.check()?;
        let existing = self.destination.existing.get(name);
        if let Some(expected) = existing {
            self.capture(name, expected)?;
        }
        self.check()?;
        let path = self.destination.directory.join(name);
        let file = file.persist_noclobber(&path).map_err(|error| error.error)?;
        self.written.push(path);
        if existing.is_some()
            && let Some(original) = self.originals.last_mut()
        {
            original.replacement = Some(Handle::from_file(file)?);
        }
        if !self.destination.existing.is_empty() {
            sync_directory(&self.destination.directory)?;
        }
        Ok(())
    }

    fn interruption(&self, error: io::Error) -> WriteError {
        WriteFailure {
            error,
            written: self.written.clone(),
            recovery: self
                .originals
                .iter()
                .any(|original| !original.released)
                .then(|| self.directory.path().to_owned()),
        }
        .into()
    }

    fn release_originals(&mut self, cleanup: &mut Option<io::Error>) -> io::Result<()> {
        self.check()?;
        for original in &self.originals {
            original.check_replacement()?;
        }
        for original in &mut self.originals {
            original.check_replacement()?;
            record_cleanup(cleanup, original.release());
        }
        Ok(())
    }

    fn restore_vacant_originals(&mut self, cleanup: &mut Option<io::Error>) {
        if self.check().is_err() {
            return;
        }
        for original in &mut self.originals {
            if !original.released
                && original.replacement.is_none()
                && original.restore_link(&self.destination.directory).is_ok()
            {
                record_cleanup(cleanup, original.release());
            }
        }
    }

    fn clean_directories(&self) -> io::Result<()> {
        self.check()?;
        let root = self.directory.path();
        ignore_missing(fs::remove_dir_all(root.join("pending")))?;
        if self.originals.iter().any(|original| !original.released) {
            return Ok(());
        }
        ignore_missing(fs::remove_dir(root.join("originals")))?;
        fs::remove_dir(root)
    }

    fn finish(mut self, mut outcome: Result<(), WriteError>) -> Result<WriteSuccess, WriteError> {
        let mut cleanup = None;
        if outcome.is_ok()
            && let Err(error) = self.release_originals(&mut cleanup)
        {
            outcome = Err(self.interruption(error));
        }
        if let Err(WriteError::Interrupted(failure)) = &mut outcome {
            self.restore_vacant_originals(&mut cleanup);
            if self.originals.iter().all(|original| original.released) {
                failure.recovery = None;
            }
        }
        record_cleanup(&mut cleanup, self.clean_directories());
        let warning = cleanup.map(|error| CleanupWarning {
            directory: self.directory.path().to_owned(),
            error,
        });
        match outcome {
            Ok(()) => Ok(WriteSuccess {
                written: self.written,
                cleanup_warning: warning,
            }),
            Err(failure) => Err(with_warning(failure, warning)),
        }
    }
}

impl Original<'_> {
    fn check_saved(&self) -> io::Result<()> {
        if fingerprint(&self.saved)?.as_ref() == Some(self.expected) {
            Ok(())
        } else {
            Err(io::Error::other("An approved original changed."))
        }
    }

    fn check_replacement(&self) -> io::Result<()> {
        if !self.target.symlink_metadata()?.is_file()
            || self.replacement.as_ref() != Some(&Handle::from_path(&self.target)?)
        {
            return Err(io::Error::other("An installed export was replaced."));
        }
        self.check_saved()
    }

    fn release(&mut self) -> io::Result<()> {
        fs::remove_file(&self.saved)?;
        self.released = true;
        Ok(())
    }

    fn restore_link(&self, destination: &Path) -> io::Result<()> {
        if !self.saved.symlink_metadata()?.is_file() {
            return Err(io::Error::other(
                "The saved original is not a regular file.",
            ));
        }
        let saved = Handle::from_path(&self.saved)?;
        fs::hard_link(&self.saved, &self.target)?;
        sync_directory(destination)?;
        if !self.target.symlink_metadata()?.is_file() || Handle::from_path(&self.target)? != saved {
            return Err(io::Error::other("The restored file was replaced."));
        }
        Ok(())
    }
}

fn preparation(path: &Path, error: io::Error) -> WriteError {
    WriteError::Prepare {
        path: path.into(),
        error,
    }
}

fn with_warning(failure: WriteError, warning: Option<CleanupWarning>) -> WriteError {
    match warning {
        Some(warning) => WriteError::Cleanup {
            failure: Box::new(failure),
            warning,
        },
        None => failure,
    }
}

fn record_cleanup(cleanup: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result {
        let _ = cleanup.get_or_insert(error);
    }
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn check_directory(path: &Path, identity: &Handle) -> io::Result<()> {
    if !path.symlink_metadata()?.is_dir() || &Handle::from_path(path)? != identity {
        return Err(io::Error::other("The export or staging folder changed."));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    if !path.is_dir() {
        return Err(io::Error::other("The export directory is unavailable."));
    }
    Ok(())
}

fn entry_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn fingerprint(path: &Path) -> io::Result<Option<Fingerprint>> {
    let Some(metadata) = entry_metadata(path)? else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Err(io::Error::other("The output path is not a regular file."));
    }
    let identity = Handle::from_path(path)?;
    let mut file = identity.as_file();
    let original = file.metadata()?;
    if !original.is_file() {
        return Err(io::Error::other(
            "The output path changed during inspection.",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let bytes = buffer
            .get(..count)
            .ok_or_else(|| io::Error::other("Invalid byte count."))?;
        digest.update(bytes);
    }
    let latest = file.metadata()?;
    if latest.len() != original.len()
        || latest.modified().ok() != original.modified().ok()
        || !path.symlink_metadata()?.is_file()
        || Handle::from_path(path)? != identity
    {
        return Err(io::Error::other("The output changed during inspection."));
    }
    Ok(Some(Fingerprint {
        identity,
        len: latest.len(),
        modified: latest.modified().ok(),
        digest: digest.finalize().into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Artifact;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const OUTPUT: &str = "synthetic-export.zip";

    fn plan() -> PreparedExport {
        PreparedExport {
            outputs: vec![Artifact {
                name: OUTPUT.into(),
                bytes: b"new export".to_vec(),
            }],
            history: Vec::new(),
        }
    }

    #[test]
    fn original_survives_an_abandoned_overwrite_and_is_discoverable() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        fs::write(&target, b"approved original")?;
        let plan = plan();
        let snapshot = plan.inspect_folder(directory.path())?;
        let mut stage = Staging::new(&snapshot)?;
        let _pending = stage.prepare(&plan.outputs)?;
        let expected = snapshot.existing.get(OUTPUT).ok_or("missing approval")?;
        stage.capture(OUTPUT, expected)?;
        let recovery = stage.directory.path().to_owned();
        drop(stage);

        let original = recovery.join("originals").join(OUTPUT);
        let Err(WriteError::RecoveryRequired(found)) = plan.inspect_folder(directory.path()) else {
            return Err("inspection missed the retained original".into());
        };
        assert_eq!(found.originals(), std::slice::from_ref(&original));
        assert_eq!(fs::read(&original)?, b"approved original");
        assert!(!target.try_exists()?);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn live_overwrite_is_reported_as_busy() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        fs::write(&target, b"approved original")?;
        let plan = plan();
        let (snapshot, guard) = plan.inspect_guarded(directory.path(), true)?;
        let mut stage = Staging::new(&snapshot)?;
        let _pending = stage.prepare(&plan.outputs)?;
        stage.capture(
            OUTPUT,
            snapshot.existing.get(OUTPUT).ok_or("missing approval")?,
        )?;
        assert!(matches!(
            plan.inspect_folder(directory.path()),
            Err(WriteError::ExportInProgress)
        ));
        assert!(matches!(
            plan.write(directory.path(), None),
            Err(WriteError::ExportInProgress)
        ));
        assert!(!target.try_exists()?);
        drop(stage);
        drop(guard);
        assert!(matches!(
            plan.inspect_folder(directory.path()),
            Err(WriteError::RecoveryRequired(_))
        ));
        Ok(())
    }

    #[test]
    fn final_check_blocks_recovery_that_appeared_after_initial_inspection() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        let plan = plan();
        let initial = plan.inspect_folder(directory.path())?;
        let mut new_stage = Staging::new(&initial)?;

        fs::write(&target, b"approved original")?;
        let approved = plan.inspect_folder(directory.path())?;
        let mut prior_stage = Staging::new(&approved)?;
        let _pending = prior_stage.prepare(&plan.outputs)?;
        prior_stage.capture(
            OUTPUT,
            approved.existing.get(OUTPUT).ok_or("missing approval")?,
        )?;
        let original = prior_stage.directory.path().join("originals").join(OUTPUT);
        drop(prior_stage);

        let outcome = plan.publish(directory.path(), &mut new_stage);
        let Err(WriteError::RecoveryRequired(found)) = new_stage.finish(outcome) else {
            return Err("publication bypassed the newly retained original".into());
        };
        assert_eq!(found.originals(), std::slice::from_ref(&original));
        assert_eq!(fs::read(&original)?, b"approved original");
        assert!(!target.try_exists()?);
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn unrelated_files_do_not_block_an_export() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("my-backup").join("originals");
        let second = directory
            .path()
            .join(".gydt-export-personal")
            .join("originals");
        for originals in [&first, &second] {
            fs::create_dir_all(originals)?;
            fs::write(originals.join(OUTPUT), b"unrelated copy")?;
        }
        let plan = plan();
        let initial = plan.inspect_folder(directory.path())?;
        let stage = Staging::new(&initial)?;
        let originals = stage.directory.path().join("originals");
        fs::create_dir(&originals)?;
        let unrelated = originals.join("different-export.zip");
        fs::write(&unrelated, b"unrelated export")?;
        drop(stage);

        assert_eq!(plan.inspect_folder(directory.path())?.filenames().len(), 0);
        let completed = plan.write(directory.path(), None)?;
        assert!(completed.cleanup_warning.is_none());
        assert_eq!(completed.written.len(), 1);
        assert_eq!(fs::read(first.join(OUTPUT))?, b"unrelated copy");
        assert_eq!(fs::read(second.join(OUTPUT))?, b"unrelated copy");
        assert_eq!(fs::read(unrelated)?, b"unrelated export");
        Ok(())
    }

    #[test]
    fn original_changed_during_capture_is_restored() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        fs::write(&target, b"approved original")?;
        let plan = plan();
        let snapshot = plan.inspect_folder(directory.path())?;
        let mut stage = Staging::new(&snapshot)?;
        let file = stage
            .prepare(&plan.outputs)?
            .pop()
            .ok_or("missing output")?;
        fs::write(&target, b"changed original")?;
        let Err(error) = stage.install(OUTPUT, file) else {
            return Err("accepted a changed original".into());
        };
        let failure = stage.interruption(error);
        let Err(WriteError::Interrupted(failure)) = stage.finish(Err(failure)) else {
            return Err("expected an interrupted overwrite".into());
        };
        assert!(failure.recovery.is_none());
        assert_eq!(failure.written, [] as [PathBuf; 0]);
        assert_eq!(fs::read(target)?, b"changed original");
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn original_edited_through_open_handle_is_retained_after_publication() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        let mut original = fs::File::create(&target)?;
        original.write_all(b"approved original")?;
        let plan = plan();
        let snapshot = plan.inspect_folder(directory.path())?;
        let mut stage = Staging::new(&snapshot)?;
        let file = stage
            .prepare(&plan.outputs)?
            .pop()
            .ok_or("missing output")?;
        stage.install(OUTPUT, file)?;
        original.write_all(b" with a concurrent edit")?;

        let Err(WriteError::Interrupted(failure)) = stage.finish(Ok(())) else {
            return Err("accepted a changed original".into());
        };
        assert!(!failure.to_string().contains(OUTPUT));
        assert_eq!(failure.written, [target.canonicalize()?]);
        let recovery = failure.recovery.ok_or("original was not retained")?;
        assert_eq!(
            fs::read(recovery.join("originals").join(OUTPUT))?,
            b"approved original with a concurrent edit"
        );
        assert_eq!(fs::read(target)?, b"new export");
        assert_eq!(fs::read_dir(recovery)?.count(), 1);
        Ok(())
    }

    #[test]
    fn a_new_target_does_not_consume_the_displaced_original() -> TestResult {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join(OUTPUT);
        fs::write(&target, b"approved original")?;
        let plan = plan();
        let snapshot = plan.inspect_folder(directory.path())?;
        let mut stage = Staging::new(&snapshot)?;
        let file = stage
            .prepare(&plan.outputs)?
            .pop()
            .ok_or("missing output")?;
        stage.capture(
            OUTPUT,
            snapshot.existing.get(OUTPUT).ok_or("missing approval")?,
        )?;
        fs::write(&target, b"other process output")?;
        let Err(error) = file.persist_noclobber(&target) else {
            return Err("overwrote a new target".into());
        };
        let failure = stage.interruption(error.error);
        let Err(WriteError::Interrupted(failure)) = stage.finish(Err(failure)) else {
            return Err("expected an interrupted overwrite".into());
        };

        let recovery = failure.recovery.ok_or("missing original to recover")?;
        assert_eq!(fs::read(&target)?, b"other process output");
        assert_eq!(
            fs::read(recovery.join("originals").join(OUTPUT))?,
            b"approved original"
        );
        assert_eq!(fs::read_dir(recovery)?.count(), 1);
        Ok(())
    }

    #[test]
    fn cleanup_warning_keeps_the_original_write_outcome() -> TestResult {
        for succeeds in [true, false] {
            let directory = tempfile::tempdir()?;
            let plan = plan();
            let snapshot = plan.inspect_folder(directory.path())?;
            let mut stage = Staging::new(&snapshot)?;
            let root = stage.directory.path().to_owned();
            let outcome = if succeeds {
                plan.publish(directory.path(), &mut stage)
            } else {
                Err(preparation(
                    directory.path(),
                    io::Error::other("test failure"),
                ))
            };
            // A nonempty staging directory forces the final removal to fail.
            fs::write(root.join("synthetic-held-file"), b"unrelated file")?;
            let warning = match stage.finish(outcome) {
                Ok(success) if succeeds => {
                    assert_eq!(success.written.len(), 1);
                    success.cleanup_warning.ok_or("missing cleanup warning")?
                }
                Err(WriteError::Cleanup { failure, warning }) if !succeeds => {
                    assert!(matches!(*failure, WriteError::Prepare { .. }));
                    warning
                }
                _ => return Err("unexpected write outcome".into()),
            };
            assert_eq!(warning.directory, root);
            assert!(!warning.to_string().contains("synthetic-held-file"));
            assert_eq!(
                fs::read(root.join("synthetic-held-file"))?,
                b"unrelated file"
            );
        }
        Ok(())
    }
}
