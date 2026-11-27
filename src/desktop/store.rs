use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

use gydt_results_compiler::{CollectionSite, ExportHistoryEntry, Profile};
use jiff::civil::Date;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::model::{Draft, Result, Site, parse_visit, valid_date};

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Settings {
    pub(super) site: Site,
    pub(super) codes: [String; 2],
    pub(super) folder: Option<PathBuf>,
    #[serde(default)]
    pub(super) folder_day: Option<String>,
    #[serde(default)]
    pub(super) attach_opus: bool,
    #[serde(default)]
    pub(super) opus_folder: Option<PathBuf>,
}

impl Settings {
    fn for_profile(profile: &Profile) -> Self {
        Self {
            site: Site::default(),
            codes: [CollectionSite::ExampleEast, CollectionSite::ExampleWest]
                .map(|site| profile.site(site).code().into()),
            folder: None,
            folder_day: None,
            attach_opus: false,
            opus_folder: None,
        }
    }

    pub(super) fn today_folder(&self, day: Date) -> Option<&Path> {
        if self.folder_day.as_deref() != Some(day.to_string().as_str()) {
            return None;
        }
        self.folder
            .as_deref()
            .filter(|folder| fs::read_dir(folder).is_ok())
    }

    pub(super) fn select_folder(&mut self, folder: PathBuf, opened: Date, finished: Date) -> bool {
        if opened != finished || fs::read_dir(&folder).is_err() {
            return false;
        }
        self.folder = Some(folder);
        self.folder_day = Some(finished.to_string());
        true
    }

    pub(super) fn valid_code(code: &str) -> bool {
        code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit())
    }

    /// Keep newer saved values for any settings the operator left alone.
    pub(super) fn merge_edits(&self, original: &Self, edited: &Self) -> Self {
        let mut next = self.clone();
        if edited.site != original.site {
            next.site = edited.site;
        }
        if edited.folder != original.folder || edited.folder_day != original.folder_day {
            next.folder.clone_from(&edited.folder);
            next.folder_day.clone_from(&edited.folder_day);
        }
        if edited.attach_opus != original.attach_opus {
            next.attach_opus = edited.attach_opus;
        }
        if edited.opus_folder != original.opus_folder {
            next.opus_folder.clone_from(&edited.opus_folder);
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
            self.codes.get(site.slot()).map_or("", String::as_str)
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

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Book {
    pub(super) settings: Settings,
    #[serde(default)]
    pub(super) history: Vec<History>,
    #[serde(default)]
    visits: HashMap<String, u32>,
}

impl Book {
    pub(super) fn for_profile(profile: &Profile) -> Self {
        Self {
            settings: Settings::for_profile(profile),
            history: Vec::new(),
            visits: HashMap::new(),
        }
    }

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
    pub(super) fn open(profile: &Profile) -> Result<Self> {
        let base = directories::BaseDirs::new()
            .ok_or("Can't locate the local application data folder.")?;
        Self::open_at(Self::profile_root(base.data_local_dir(), profile))
    }

    pub(super) fn profile_root(base: &Path, profile: &Profile) -> PathBuf {
        base.join("GYDT Sample Reporting Profiles")
            .join(profile.storage_key())
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

    pub(super) fn book(&self, profile: &Profile) -> Result<Book> {
        let path = self.root.join("book.json");
        let book = load::<Book>(&path)?.unwrap_or_else(|| Book::for_profile(profile));
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
