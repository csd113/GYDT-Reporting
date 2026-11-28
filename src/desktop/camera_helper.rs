use std::{
    fs,
    io::{self, Write as _},
    path::{Component, Path, PathBuf, Prefix},
};

use winsafe::{HFILE, co, guard::CloseHandleGuard};

use super::model::digest;

const CACHE: &str = "camera";
const FORMAT: &[u8] = b"GYDT-CAMERA-BUNDLE-1\0";
const PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/camera.bundle"));
const MAX_BYTES: usize = 8 * 1024 * 1024;

struct Bundle<'a> {
    bytes: &'a [u8],
    sha: &'a str,
}

impl<'a> Bundle<'a> {
    fn decode(encoded: &'a [u8]) -> io::Result<Self> {
        let encoded = encoded.strip_prefix(FORMAT).ok_or_else(invalid)?;
        let (size, encoded) = encoded.split_first_chunk::<4>().ok_or_else(invalid)?;
        let (sha, bytes) = encoded.split_first_chunk::<64>().ok_or_else(invalid)?;
        let sha = std::str::from_utf8(sha).map_err(io::Error::other)?;
        if bytes.len() != u32::from_le_bytes(*size) as usize
            || bytes.len() > MAX_BYTES
            || !bytes.starts_with(b"MZ")
            || digest(bytes) != sha
        {
            return Err(invalid());
        }
        Ok(Self { bytes, sha })
    }

    fn filename(&self) -> String {
        format!("GYDT-camera-{}.exe", self.sha)
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid bundled camera helper")
}

fn open(
    path: &Path,
    share: co::FILE_SHARE,
    flags: co::FILE_FLAG,
) -> io::Result<CloseHandleGuard<HFILE>> {
    let path = path.to_str().ok_or_else(invalid)?;
    HFILE::CreateFile(
        path,
        co::GENERIC::READ,
        Some(share),
        None,
        co::DISPOSITION::OPEN_EXISTING,
        co::FILE_ATTRIBUTE::NORMAL,
        Some(flags | co::FILE_FLAG::OPEN_REPARSE_POINT),
        None,
        None,
    )
    .map(|(handle, _)| handle)
    .map_err(|e| match e {
        co::ERROR::FILE_NOT_FOUND | co::ERROR::PATH_NOT_FOUND => io::ErrorKind::NotFound.into(),
        _ => io::Error::other(e),
    })
}

fn pin_directory(path: &Path) -> io::Result<CloseHandleGuard<HFILE>> {
    let handle = open(
        path,
        co::FILE_SHARE::READ | co::FILE_SHARE::WRITE,
        co::FILE_FLAG::BACKUP_SEMANTICS,
    )?;
    let attrs = handle
        .GetFileInformationByHandle()
        .map_err(io::Error::other)?
        .dwFileAttributes;
    if !attrs.has(co::FILE_ATTRIBUTE::DIRECTORY) || attrs.has(co::FILE_ATTRIBUTE::REPARSE_POINT) {
        return Err(invalid());
    }
    Ok(handle)
}

fn settings_chain(root: &Path) -> io::Result<Vec<CloseHandleGuard<HFILE>>> {
    if !root.is_absolute()
        || !matches!(root.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid());
    }
    let ancestors: Vec<_> = root.ancestors().collect();
    // Open each component without following reparse points, top down. Holding
    // handles without delete sharing pins each directory through process launch.
    ancestors.into_iter().rev().map(pin_directory).collect()
}

fn verify(path: &Path, bundle: &Bundle<'_>) -> io::Result<CloseHandleGuard<HFILE>> {
    // Only read sharing: neither the checked file nor its contents can be
    // replaced between this check and CreateProcess.
    let file = open(path, co::FILE_SHARE::READ, co::FILE_FLAG::default())?;
    let info = file
        .GetFileInformationByHandle()
        .map_err(io::Error::other)?;
    if info.dwFileAttributes.has(co::FILE_ATTRIBUTE::DIRECTORY)
        || info.dwFileAttributes.has(co::FILE_ATTRIBUTE::REPARSE_POINT)
        || info.nNumberOfLinks != 1
        || info.nFileSize() != bundle.bytes.len() as u64
    {
        return Err(invalid());
    }
    let mut actual = vec![0u8; bundle.bytes.len()];
    let read = file.ReadFile(&mut actual).map_err(io::Error::other)?;
    if read as usize != actual.len() || digest(&actual) != bundle.sha {
        return Err(invalid());
    }
    Ok(file)
}

fn install(cache: &Path, path: &Path, bundle: &Bundle<'_>) -> io::Result<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".camera-")
        .tempfile_in(cache)?;
    temporary.write_all(bundle.bytes)?;
    temporary.as_file().sync_all()?;
    match temporary.into_temp_path().persist_noclobber(path) {
        Ok(()) => Ok(()),
        // Another caller may have finished first. Always verify the winner.
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

pub(super) struct VerifiedHelper {
    pub(super) path: PathBuf,
    _file: CloseHandleGuard<HFILE>,
    _directories: Vec<CloseHandleGuard<HFILE>>,
}

pub(super) fn prepare(root: &Path) -> io::Result<VerifiedHelper> {
    let bundle = Bundle::decode(PAYLOAD)?;
    let mut directories = settings_chain(root)?;
    let cache = root.join(CACHE);
    match fs::create_dir(&cache) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    directories.push(pin_directory(&cache)?);
    let path = cache.join(bundle.filename());
    let file = match verify(&path, &bundle) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            install(&cache, &path, &bundle)?;
            verify(&path, &bundle)?
        }
        Err(error) => return Err(error),
    };
    Ok(VerifiedHelper {
        path,
        _file: file,
        _directories: directories,
    })
}

#[cfg(test)]
mod tests {
    use super::{Bundle, CACHE, PAYLOAD, fs, prepare};

    #[test]
    fn missing_cache_is_created_and_reused_then_recreated_if_removed() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        assert!(!root.path().join(CACHE).exists());
        let first = prepare(root.path())?;
        let path = first.path.clone();
        let second = prepare(root.path())?;
        assert_eq!(path, second.path);
        assert_eq!(fs::read(&path)?, Bundle::decode(PAYLOAD)?.bytes);
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(first);
        drop(second);
        fs::remove_file(&path)?;
        assert_eq!(prepare(root.path())?.path, path);
        assert_eq!(fs::read_dir(root.path().join(CACHE))?.count(), 1);
        Ok(())
    }

    #[test]
    fn same_length_tamper_and_hardlinks_are_rejected_without_overwrite() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let path = prepare(root.path())?.path.clone();
        let linked = root.path().join("outside-cache.exe");
        fs::hard_link(&path, &linked)?;
        assert!(prepare(root.path()).is_err());
        fs::remove_file(&linked)?;
        let altered = vec![0; Bundle::decode(PAYLOAD)?.bytes.len()];
        fs::write(&path, &altered)?;
        assert!(prepare(root.path()).is_err());
        assert_eq!(fs::read(&path)?, altered);
        Ok(())
    }

    #[test]
    fn concurrent_preparation_produces_one_verified_copy() -> std::io::Result<()> {
        use std::{sync::Barrier, thread};

        let root = tempfile::tempdir()?;
        let barrier = Barrier::new(4);
        let paths = thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        prepare(root.path()).map(|helper| helper.path)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| std::io::Error::other("preparation thread panicked"))?
                })
                .collect::<std::io::Result<Vec<_>>>()
        })?;
        let first = paths
            .first()
            .ok_or_else(|| std::io::Error::other("missing camera helper"))?;
        assert!(paths.iter().all(|path| path == first));
        assert_eq!(fs::read(first)?, Bundle::decode(PAYLOAD)?.bytes);
        assert_eq!(fs::read_dir(root.path().join(CACHE))?.count(), 1);
        Ok(())
    }

    #[test]
    fn a_corrupt_embedded_digest_is_rejected() -> std::io::Result<()> {
        let mut altered = PAYLOAD.to_vec();
        let sha_byte = altered
            .get_mut(super::FORMAT.len() + 4)
            .ok_or_else(super::invalid)?;
        *sha_byte ^= 1;
        assert!(Bundle::decode(&altered).is_err());
        Ok(())
    }

    #[test]
    fn unexpected_cache_entry_is_not_replaced() -> std::io::Result<()> {
        let root = tempfile::tempdir()?;
        let entry = root.path().join(CACHE);
        fs::write(&entry, b"leave this alone")?;
        assert!(prepare(root.path()).is_err());
        assert_eq!(fs::read(entry)?, b"leave this alone");
        Ok(())
    }
}
