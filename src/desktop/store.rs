use std::{
    collections::HashMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

use gydt_results_compiler::{DEFAULT_SITE_CODE, ExportHistoryEntry};
use jiff::civil::Date;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::model::{Draft, Result, Site, parse_visit, valid_date};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Settings {
    pub(super) site: Site,
    pub(super) codes: [String; 2],
    pub(super) folder: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            site: Site::default(),
            codes: [DEFAULT_SITE_CODE.into(), DEFAULT_SITE_CODE.into()],
            folder: None,
        }
    }
}

impl Settings {
    pub(super) fn valid_code(code: &str) -> bool {
        code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit())
    }

    /// Keep newer saved values for any settings the operator left alone.
    pub(super) fn merge_edits(&self, original: &Self, edited: &Self) -> Self {
        let mut next = self.clone();
        if edited.site != original.site {
            next.site = edited.site;
        }
        if edited.folder != original.folder {
            next.folder.clone_from(&edited.folder);
        }
        for ((current, original), edited) in next
            .codes
            .iter_mut()
            .zip(&original.codes)
            .zip(&edited.codes)
        {
            if edited != original {
                current.clone_from(edited);
            }
        }
        next
    }

    pub(super) fn prefix(&self, site: Site, day: Date) -> String {
        format!(
            "3-{}-{day}-",
            self.codes
                .get(site.slot())
                .map_or(DEFAULT_SITE_CODE, String::as_str)
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct History {
    pub(super) id: Uuid,
    title: String,
    site: Site,
    day: String,
}

impl History {
    pub(super) fn entry(&self) -> Result<ExportHistoryEntry> {
        Ok(ExportHistoryEntry {
            id: self.id,
            submission_number: self.title.clone(),
            site: self.site.into(),
            analysis_date: valid_date(&self.day)
                .ok_or("The saved export history contains an invalid date.")?,
        })
    }

    fn from_draft(draft: &Draft) -> Self {
        Self {
            id: draft.id,
            title: draft.title.clone(),
            site: draft.site,
            day: draft.analysis_date.clone(),
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Book {
    pub(super) settings: Settings,
    pub(super) history: Vec<History>,
    visits: HashMap<String, u32>,
}

impl Book {
    fn visit_key(site: Site, day: Date) -> String {
        format!("{}:{day}", site.slot())
    }

    pub(super) fn proposed_visit(&self, site: Site, day: Date, records: &[Record]) -> Result<u32> {
        let today = day.to_string();
        let saved = records
            .iter()
            .filter(|record| record.day == today)
            .filter_map(|record| record.visit_claims.get(site.slot()).copied())
            .max()
            .unwrap_or_default();
        // Preserve counters already stored by previous desktop builds. New claims
        // live in their sample record so the two are committed in one write.
        let previous = self
            .visits
            .get(&Self::visit_key(site, day))
            .copied()
            .unwrap_or_default();
        previous
            .max(saved)
            .checked_add(1)
            .filter(|visit| *visit <= 2_147_483_647)
            .ok_or_else(|| "The daily Visit counter has reached its limit.".into())
    }

    pub(super) fn add_history(&mut self, draft: &Draft) {
        if !self.history.iter().any(|entry| {
            (
                entry.id,
                entry.title.as_str(),
                entry.site,
                entry.day.as_str(),
            ) == (
                draft.id,
                draft.title.as_str(),
                draft.site,
                draft.analysis_date.as_str(),
            )
        }) {
            self.history.push(History::from_draft(draft));
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct Record {
    pub(super) id: Uuid,
    pub(super) title: String,
    pub(super) site: Site,
    #[serde(rename = "analysis_date")]
    pub(super) day: String,
    #[serde(default)]
    visit_claims: [u32; 2],
    // Earlier records stored the current Visit as form text.
    #[serde(default, skip_serializing, rename = "visit")]
    legacy_visit: String,
}

impl From<&Draft> for Record {
    fn from(draft: &Draft) -> Self {
        Self {
            id: draft.id,
            title: draft.tab_label().to_owned(),
            site: draft.site,
            day: draft.analysis_date.clone(),
            visit_claims: draft.visit_claims,
            legacy_visit: String::new(),
        }
    }
}

pub(super) struct Store {
    root: PathBuf,
    _lock: File,
}

impl Store {
    pub(super) fn open() -> Result<Self> {
        let root = Self::configured_root(
            || std::env::var_os("GYDT_DATA_DIR"),
            || directories::BaseDirs::new().map(|dirs| dirs.data_local_dir().to_owned()),
        )?;
        Self::open_at(root)
    }

    fn configured_root(
        staff_override: impl FnOnce() -> Option<OsString>,
        base_directory: impl FnOnce() -> Option<PathBuf>,
    ) -> Result<PathBuf> {
        if !cfg!(feature = "public-demo")
            && let Some(path) = staff_override()
        {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("GYDT_DATA_DIR must be an absolute path.".into());
            }
            return Ok(path);
        }

        let namespace = if cfg!(feature = "public-demo") {
            "GYDT Sample Reporting Demo"
        } else {
            "GYDT Sample Reporting"
        };
        Ok(base_directory()
            .ok_or("Can't locate the local application data folder.")?
            .join(namespace))
    }

    pub(super) fn open_at(root: PathBuf) -> Result<Self> {
        private_dir(&root)?;
        private_dir(&root.join("samples"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("desktop.lock"))?;
        lock.try_lock().map_err(|error| format!("Can't lock local records. Close any other GYDT Sample Reporting window and retry. {error}"))?;
        Ok(Self { root, _lock: lock })
    }

    pub(super) fn open_data_folder(&self) -> Result<()> {
        fs::read_dir(&self.root)
            .map_err(|error| format!("Can't read {}. {error}", self.root.display()))?;

        #[cfg(target_os = "macos")]
        {
            let output = Command::new("/usr/bin/open")
                .args(["-b", "com.apple.finder"])
                .arg(&self.root)
                .output()?;
            if !output.status.success() {
                return Err(format!(
                    "Finder couldn't open {}. {}",
                    self.root.display(),
                    String::from_utf8_lossy(&output.stderr).trim(),
                )
                .into());
            }
        }

        #[cfg(target_os = "windows")]
        {
            let system_root = std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or("Can't locate File Explorer.")?;
            Command::new(system_root.join("explorer.exe"))
                .arg(&self.root)
                .spawn()?;
        }

        Ok(())
    }

    pub(super) fn book(&self) -> Result<Book> {
        let path = self.root.join("book.json");
        let book = load::<Book>(&path)?.unwrap_or_default();
        if !book
            .settings
            .codes
            .iter()
            .all(|code| Settings::valid_code(code))
        {
            return Err("The saved Settings contain an invalid submission code.".into());
        }
        Ok(book)
    }

    pub(super) fn save_book(&self, book: &Book) -> Result<()> {
        save(&self.root.join("book.json"), book)
    }

    fn sample_path(&self, id: Uuid) -> PathBuf {
        self.root.join("samples").join(format!("{id}.json"))
    }

    pub(super) fn save_record(&self, record: &Record) -> Result<()> {
        save(&self.sample_path(record.id), record)
    }

    pub(super) fn records(&self) -> Result<Vec<Record>> {
        let mut records = Vec::new();
        for item in fs::read_dir(self.root.join("samples"))? {
            let path = item?.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Some(id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.parse::<Uuid>().ok())
            {
                let mut record =
                    load::<Record>(&path)?.ok_or("A local sample record is missing.")?;
                if record.id != id {
                    return Err("A local sample record has an invalid ID.".into());
                }
                if let Some(visit) = parse_visit(&record.legacy_visit)
                    && let Some(highest) = record.visit_claims.get_mut(record.site.slot())
                {
                    *highest = (*highest).max(visit);
                }
                records.push(record);
            }
        }
        Ok(records)
    }
}

fn private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match File::open(path) {
        Ok(file) => Ok(Some(
            serde_json::from_reader(BufReader::new(file))
                .map_err(|error| format!("Can't read saved data at {}. {error}", path.display()))?,
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or("The application data path has no parent.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let mut writer = BufWriter::new(file.as_file_mut());
    serde_json::to_writer(&mut writer, value)?;
    writer.flush()?;
    drop(writer);
    file.as_file().sync_all()?;
    file.persist(path)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[derive(Default)]
pub(super) struct Sequence {
    assigned: HashMap<String, u64>,
    scanned: Vec<String>,
    available: bool,
}

impl Sequence {
    pub(super) fn scan(&mut self, folder: Option<&Path>) {
        let scanned = folder.map(scan_names).transpose();
        self.available = matches!(scanned, Ok(Some(_)));
        self.scanned = scanned.ok().flatten().unwrap_or_default();
        // Retain previously verified numbers if a synced destination goes away.
        for stem in &self.scanned {
            if let Some((prefix, serial)) = split_title(stem) {
                let highest = self.assigned.entry(prefix.to_owned()).or_default();
                *highest = (*highest).max(serial);
            }
        }
    }

    pub(super) fn reserve(&mut self, title: &str) {
        if let Some((prefix, serial)) = split_title(title) {
            let highest = self.assigned.entry(prefix.to_owned()).or_default();
            *highest = (*highest).max(serial);
        }
    }

    pub(super) fn next(&mut self, prefix: &str, book: &Book, records: &[Record]) -> Option<String> {
        let highest_saved = book
            .history
            .iter()
            .map(|entry| entry.title.as_str())
            .chain(records.iter().map(|record| record.title.as_str()))
            .filter_map(|title| {
                let (candidate, serial) = split_title(title)?;
                (candidate == prefix).then_some(serial)
            })
            .max();
        let highest = self
            .assigned
            .get(prefix)
            .copied()
            .into_iter()
            .chain(highest_saved)
            .max();
        if highest.is_none() && !self.available {
            return None;
        }
        let serial = highest.unwrap_or_default().checked_add(1)?;
        self.assigned.insert(prefix.to_owned(), serial);
        Some(format!("{prefix}{serial:03}"))
    }
}

fn scan_names(folder: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && let Some(name) = entry.file_name().to_str()
            && let Some((stem, extension)) = name.rsplit_once('.')
            && extension.eq_ignore_ascii_case("zip")
        {
            names.push(stem.to_owned());
        }
    }
    Ok(names)
}

fn split_title(title: &str) -> Option<(&str, u64)> {
    let (prefix, serial) = title.rsplit_once('-')?;
    let (city, rest) = prefix.split_once('-')?;
    let (code, day) = rest.split_once('-')?;
    if city != "3"
        || !Settings::valid_code(code)
        || valid_date(day).is_none()
        || serial.len() < 3
        || !serial.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let value: u64 = serial.parse().ok()?;
    (value > 0).then(|| (title.get(..=prefix.len()).unwrap_or_default(), value))
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[cfg(feature = "public-demo")]
    #[test]
    fn demo_ignores_staff_override_and_opens_a_separate_store() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("app-data");
        let default_staff = base.join("GYDT Sample Reporting");
        let inherited_staff = tmp.path().join("explicit-staff-data");
        let sentinel = b"malformed synthetic staff book";
        for staff in [&default_staff, &inherited_staff] {
            fs::create_dir_all(staff)?;
            fs::write(staff.join("book.json"), sentinel)?;
        }

        for staff_override in [None, Some(inherited_staff.into_os_string())] {
            let consulted = std::cell::Cell::new(false);
            let root = Store::configured_root(
                || {
                    consulted.set(true);
                    staff_override
                },
                || Some(base.clone()),
            )?;
            assert!(!consulted.get());
            assert_eq!(root, base.join("GYDT Sample Reporting Demo"));
            let store = Store::open_at(root)?;
            let book = store.book()?;
            assert!(store.records()?.is_empty());
            store.save_book(&book)?;
        }

        for staff in [default_staff, tmp.path().join("explicit-staff-data")] {
            assert_eq!(fs::read(staff.join("book.json"))?, sentinel);
            assert_eq!(fs::read_dir(staff)?.count(), 1);
        }
        Ok(())
    }

    #[cfg(not(feature = "public-demo"))]
    #[test]
    fn ordinary_build_preserves_staff_override_and_default() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("app-data");
        let staff = tmp.path().join("staff");
        assert_eq!(
            Store::configured_root(|| Some(staff.clone().into_os_string()), || None)?,
            staff
        );
        assert!(
            Store::configured_root(|| Some(OsString::from("relative")), || Some(base.clone()),)
                .is_err()
        );
        assert_eq!(
            Store::configured_root(|| None, || Some(base.clone()))?,
            base.join("GYDT Sample Reporting")
        );
        Ok(())
    }
}
