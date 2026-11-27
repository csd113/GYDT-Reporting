mod archive;
mod export;
mod model;
mod store;
mod ui;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(test)]
mod tests;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{self, Context};
use gydt_results_compiler::{Profile, WriteError};
use jiff::{Timestamp, civil::Date, tz::TimeZone};

use self::{
    export::{Event, Plan, Selection, Worker},
    model::{Draft, PhotoCopy, Result, Site, parse_visit, valid_date},
    store::{Book, Record, Sequence, Settings, Store},
};

pub(super) const APP_NAME: &str = "GYDT Sample Reporting";
#[cfg(target_os = "macos")]
const REPORTING_SIZE: [f32; 2] = [1136.0, 823.0];
#[cfg(target_os = "macos")]
const SETTINGS_SIZE: [f32; 2] = [1136.0, 553.0];
const SAVE_DELAY: Duration = Duration::from_millis(280);

pub(super) fn run() -> Result<()> {
    let profile = Arc::new(Profile::embedded()?);
    let store = Store::open(&profile)?;
    let book = store.book(&profile)?;
    let records = store.records()?;
    let orbit = ui::orbit()?;
    let viewport = egui::ViewportBuilder::default()
        .with_drag_and_drop(true)
        .with_icon(orbit.clone());
    #[cfg(target_os = "windows")]
    let viewport = viewport
        .with_inner_size(windows::REPORTING_SIZE)
        .with_min_inner_size([620.0, 340.0])
        .with_resizable(true);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_inner_size(REPORTING_SIZE)
        .with_resizable(false)
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false);
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            ui::configure(&cc.egui_ctx);
            #[cfg(target_os = "windows")]
            if let Some(window) = cc.winit_window()
                && let Ok(icon) =
                    winit::window::Icon::from_rgba(orbit.rgba.clone(), orbit.width, orbit.height)
            {
                use winit::platform::windows::WindowExtWindows as _;
                window.set_taskbar_icon(Some(icon));
            }
            #[cfg(target_os = "macos")]
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Light));
            let app = App::new(&cc.egui_ctx, store, book, records, profile, &orbit);
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| error.to_string().into())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TabOrigin {
    Proposed,
    SameVisit,
    Recorded,
    Archive,
}

impl TabOrigin {
    fn is_provisional(self) -> bool {
        matches!(self, Self::Proposed | Self::SameVisit)
    }
}

struct Tab {
    draft: Draft,
    origin: TabOrigin,
    missing: bool,
    changed: Option<Instant>,
    thumbnail: Option<egui::TextureHandle>,
}

impl Tab {
    fn new(ctx: &Context, draft: Draft, origin: TabOrigin) -> Self {
        let thumbnail = draft
            .photo
            .as_ref()
            .and_then(|photo| ui::thumbnail(ctx, photo));
        Self {
            draft,
            origin,
            missing: false,
            changed: None,
            thumbnail,
        }
    }

    fn edit(&mut self) {
        let save = match self.origin {
            TabOrigin::Proposed | TabOrigin::SameVisit => self.draft.has_entry_data(),
            TabOrigin::Recorded => true,
            TabOrigin::Archive => false,
        };
        self.changed = save.then(Instant::now);
        self.missing &= self.draft.has_missing();
    }
}

enum Dialog {
    ExportFolder,
    PhotoPrompt,
    ReplacePhoto(usize, PendingPhoto),
    PhotoOptions,
    Batch(Vec<bool>, bool),
    Repeat(Selection, Vec<String>),
    Overwrite(Selection, Box<Plan>),
    Notice(&'static str, String),
}

struct PendingPhoto {
    copy: PhotoCopy,
    #[cfg(target_os = "windows")]
    rename_from: Option<PathBuf>,
}

struct Completion {
    folder: PathBuf,
    files: Vec<PathBuf>,
    warning: Option<String>,
}

struct SettingsPane {
    original: Settings,
    draft: Settings,
    error: Option<String>,
    section: ui::SettingsSection,
}

struct App {
    store: Store,
    book: Book,
    records: Vec<Record>,
    profile: Arc<Profile>,
    options: Vec<ui::SubstanceChoice>,
    orbit: egui::TextureHandle,
    sequence: Sequence,
    tabs: Vec<Tab>,
    active: Option<usize>,
    dialog: Option<Dialog>,
    settings: Option<SettingsPane>,
    show_settings: bool,
    worker: Worker,
    completion: Option<Completion>,
    status: Option<String>,
    save_error: Option<String>,
    history_pending: bool,
    folder_prompt_day: Option<Date>,
    deferred_photo_prompt: bool,
    #[cfg(target_os = "windows")]
    last_work_area: Option<[i32; 4]>,
}

impl App {
    fn new(
        ctx: &Context,
        store: Store,
        book: Book,
        records: Vec<Record>,
        profile: Arc<Profile>,
        orbit: &egui::IconData,
    ) -> Self {
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [orbit.width as usize, orbit.height as usize],
            &orbit.rgba,
        );
        let mut app = Self {
            store,
            book,
            records,
            options: ui::substances(profile.catalogue()),
            profile,
            orbit: ctx.load_texture("Indigo Orbit", pixels, egui::TextureOptions::LINEAR),
            sequence: Sequence::default(),
            tabs: Vec::new(),
            active: None,
            dialog: None,
            settings: None,
            show_settings: false,
            worker: Worker::default(),
            completion: None,
            status: None,
            save_error: None,
            history_pending: false,
            folder_prompt_day: None,
            deferred_photo_prompt: false,
            #[cfg(target_os = "windows")]
            last_work_area: None,
        };
        app.rescan();
        app.new_tab(ctx, false);
        if let Ok(day) = analysis_day()
            && app.book.settings.today_folder(day).is_none()
            && matches!(app.dialog, Some(Dialog::PhotoPrompt))
        {
            app.deferred_photo_prompt = true;
            app.ask_for_daily_folder(day);
        }
        app
    }

    fn ask_for_daily_folder(&mut self, day: Date) {
        self.folder_prompt_day = Some(day);
        self.dialog = Some(Dialog::ExportFolder);
    }

    fn tick_daily_folder(&mut self, ctx: &Context) {
        let now = Timestamp::now();
        let Ok(zone) = TimeZone::get("America/Vancouver") else {
            return;
        };
        let today = now.to_zoned(zone.clone()).date();
        if let Ok(tomorrow) = today.tomorrow()
            && let Ok(midnight) = tomorrow.to_zoned(zone)
        {
            let until = midnight.timestamp().as_second() - now.as_second();
            // Recheck sooner occasionally to cover system clock corrections.
            let seconds = u64::try_from(until).unwrap_or(1).clamp(1, 60);
            ctx.request_repaint_after(Duration::from_secs(seconds));
        }
        if self.dialog.is_none() && !self.show_settings && !self.worker.busy() {
            if self.folder_prompt_day != Some(today)
                && self.book.settings.today_folder(today).is_none()
            {
                self.ask_for_daily_folder(today);
            } else if std::mem::take(&mut self.deferred_photo_prompt) && self.active.is_some() {
                self.dialog = Some(Dialog::PhotoPrompt);
            }
        }
    }

    fn rescan(&mut self) {
        let today = analysis_day()
            .ok()
            .and_then(|day| self.book.settings.today_folder(day));
        self.sequence.scan(today);
    }

    fn new_tab(&mut self, ctx: &Context, same_visit: bool) {
        if let Err(error) = self.create_tab(ctx, same_visit) {
            self.notice("Can't open a new sample", error.to_string());
        }
    }

    fn active_intake(&self, day: Date) -> Option<(Site, u32)> {
        let tab = self.tabs.get(self.active?)?;
        if tab.origin.is_provisional() || valid_date(&tab.draft.analysis_date) != Some(day) {
            return None;
        }
        Some((tab.draft.site, parse_visit(&tab.draft.visit)?))
    }

    fn create_tab(&mut self, ctx: &Context, same_visit: bool) -> Result<()> {
        self.flush(true)?;
        let day = analysis_day()?;
        self.rescan();
        let (mut draft, origin) = if same_visit {
            let (site, visit) = self
                .active_intake(day)
                .ok_or("Open a started sample from today's intake first.")?;
            let mut draft = Draft::fresh(site, day)?;
            draft.visit = visit.to_string();
            (draft, TabOrigin::SameVisit)
        } else {
            let site = self.book.settings.site;
            let mut draft = Draft::fresh(site, day)?;
            draft.suggest_visit(self.book.proposed_visit(site, day, &self.records)?);
            (draft, TabOrigin::Proposed)
        };
        self.suggest_title(&mut draft);
        self.push_tab(ctx, draft, origin);
        self.dialog = Some(Dialog::PhotoPrompt);
        Ok(())
    }

    fn suggest_title(&mut self, draft: &mut Draft) {
        let title = valid_date(&draft.analysis_date)
            .and_then(|day| {
                let prefix = self.book.settings.prefix(draft.site, day);
                self.sequence.next(&prefix, &self.book, &self.records)
            })
            .unwrap_or_default();
        draft.title.clone_from(&title);
        draft.suggested_title = Some(title);
    }

    fn refill_titles(&mut self) {
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get(index) else {
                continue;
            };
            if tab.draft.exported
                || !tab.draft.title.is_empty()
                || tab.draft.suggested_title.as_deref() != Some("")
            {
                continue;
            }
            let mut draft = tab.draft.clone();
            self.suggest_title(&mut draft);
            if let Some(tab) = self.tabs.get_mut(index)
                && tab.draft != draft
            {
                tab.draft = draft;
                tab.edit();
            }
        }
    }

    fn switch_site(&mut self, index: usize, prior: Site) -> Result<()> {
        let Some(tab) = self.tabs.get(index) else {
            return Ok(());
        };
        let mut draft = tab.draft.clone();
        let repeat = tab.origin == TabOrigin::SameVisit;
        self.rescan();
        if !draft.exported
            && let Some(day) = valid_date(&draft.analysis_date)
        {
            if draft.suggested_title.is_some()
                && self.book.settings.prefix(prior, day)
                    != self.book.settings.prefix(draft.site, day)
            {
                self.suggest_title(&mut draft);
            }
            if repeat || draft.suggested_visit.is_some() {
                let visit = self.visit_for(&draft, day)?;
                draft.suggest_visit(visit);
            }
        }
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.draft = draft;
            if repeat {
                tab.origin = TabOrigin::Proposed;
            }
            tab.edit();
        }
        Ok(())
    }

    fn visit_for(&self, draft: &Draft, day: Date) -> Result<u32> {
        if let Some(visit) = draft.claimed_visit(draft.site) {
            Ok(visit)
        } else {
            self.book.proposed_visit(draft.site, day, &self.records)
        }
    }

    fn push_tab(&mut self, ctx: &Context, draft: Draft, origin: TabOrigin) {
        self.sequence.reserve(&draft.title);
        self.tabs.push(Tab::new(ctx, draft, origin));
        self.select_tab(ctx, self.tabs.len() - 1);
    }

    fn select_tab(&mut self, ctx: &Context, index: usize) {
        self.active = Some(index);
        self.show_settings = false;
        self.completion = None;
        self.status = None;
        // A tab's text edit focus should never carry over to another record.
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }

    fn close_tab(&mut self, ctx: &Context, index: usize) {
        if let Err(error) = self.flush(true) {
            self.notice("Can't save the sample", error.to_string());
            return;
        }
        self.tabs.remove(index);
        if self.active == Some(index) {
            if self.tabs.is_empty() {
                self.active = None;
                self.completion = None;
            } else {
                self.select_tab(ctx, index.min(self.tabs.len() - 1));
            }
        } else if let Some(active) = self.active
            && active > index
        {
            self.active = Some(active - 1);
        }
    }

    fn persist(&mut self, draft: &Draft) -> Result<()> {
        let updated = Record::from(draft);
        self.store.save_record(&updated)?;
        if let Some(record) = self.records.iter_mut().find(|record| record.id == draft.id) {
            *record = updated;
        } else {
            self.records.push(updated);
        }
        Ok(())
    }

    fn flush(&mut self, now: bool) -> Result<()> {
        if self.history_pending {
            self.store.save_book(&self.book)?;
            self.history_pending = false;
        }
        let mut saved = false;
        for index in 0..self.tabs.len() {
            if self
                .tabs
                .get(index)
                .and_then(|tab| tab.changed)
                .is_some_and(|since| now || since.elapsed() >= SAVE_DELAY)
            {
                self.save_tab(index, analysis_day()?)?;
                saved = true;
            }
        }
        if saved {
            self.refresh_proposed_visits();
        }
        self.save_error = None;
        Ok(())
    }

    fn save_tab(&mut self, index: usize, today: Date) -> Result<()> {
        let Some(tab) = self.tabs.get(index) else {
            return Ok(());
        };
        let mut draft = tab.draft.clone();
        let provisional = tab.origin.is_provisional();
        let new_day = provisional && valid_date(&draft.analysis_date) != Some(today);
        if new_day {
            if draft.collection_date == draft.analysis_date {
                draft.collection_date = today.to_string();
            }
            draft.analysis_date = today.to_string();
        }
        let suggest_visit =
            draft.suggested_visit.is_some() || (new_day && tab.origin == TabOrigin::SameVisit);
        if new_day && draft.suggested_title.is_some() {
            self.rescan();
            self.suggest_title(&mut draft);
        }
        if suggest_visit && let Some(day) = valid_date(&draft.analysis_date) {
            draft.suggest_visit(self.visit_for(&draft, day)?);
        }
        draft.claim_visit();
        self.persist(&draft)?;
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.draft = draft;
            tab.origin = TabOrigin::Recorded;
            tab.changed = None;
        }
        Ok(())
    }

    fn refresh_proposed_visits(&mut self) {
        for tab in &mut self.tabs {
            if tab.origin == TabOrigin::Proposed
                && tab.draft.suggested_visit.is_some()
                && let Some(day) = valid_date(&tab.draft.analysis_date)
                && let Ok(visit) = self.book.proposed_visit(tab.draft.site, day, &self.records)
            {
                tab.draft.suggest_visit(visit);
            }
        }
    }

    fn notice(&mut self, title: &'static str, message: impl Into<String>) {
        self.dialog = Some(Dialog::Notice(title, message.into()));
    }

    fn open_settings(&mut self) {
        let saved = &self.book.settings;
        if let Some(settings) = &mut self.settings {
            settings.draft = saved.merge_edits(&settings.original, &settings.draft);
            settings.original = saved.clone();
        } else {
            self.settings = Some(SettingsPane {
                original: saved.clone(),
                draft: saved.clone(),
                error: None,
                section: ui::SettingsSection::Location,
            });
        }
        self.show_settings = true;
    }

    fn save_settings(&mut self) {
        let Some(settings) = &mut self.settings else {
            return;
        };
        let next = self
            .book
            .settings
            .merge_edits(&settings.original, &settings.draft);
        if !next.codes.iter().all(|code| Settings::valid_code(code)) {
            settings.error = Some("Enter exactly three digits for each location.".into());
            return;
        }
        match self.apply_settings(next) {
            Ok(()) => self.cancel_settings(),
            Err(error) => {
                if let Some(settings) = &mut self.settings {
                    settings.error = Some(error.to_string());
                }
            }
        }
    }

    fn cancel_settings(&mut self) {
        self.settings = None;
        self.show_settings = false;
    }

    fn apply_settings(&mut self, settings: Settings) -> Result<()> {
        let mut next = self.book.clone();
        next.settings = settings;
        self.store.save_book(&next)?;
        self.book = next;
        self.history_pending = false;
        self.rescan();
        self.refill_titles();
        self.status = None;
        Ok(())
    }

    fn choose_folder(&mut self, in_settings: bool) {
        let Ok(opened) = analysis_day() else {
            return;
        };
        let settings = self
            .settings
            .as_ref()
            .filter(|_| in_settings)
            .map_or(&self.book.settings, |settings| &settings.draft);
        let mut dialog = rfd::FileDialog::new().set_title("Choose today’s export folder");
        if let Some(parent) = settings.folder.as_deref().and_then(nearest_directory) {
            dialog = dialog.set_directory(parent);
        }
        let Some(folder) = dialog.pick_folder() else {
            return;
        };
        let Ok(finished) = analysis_day() else {
            return;
        };
        if opened != finished {
            self.ask_for_daily_folder(finished);
            return;
        }
        if in_settings {
            if let Some(settings) = &mut self.settings
                && !settings.draft.select_folder(folder, opened, finished)
            {
                settings.error = Some("Choose an existing readable export folder.".into());
            }
        } else {
            let mut next = self.book.settings.clone();
            if !next.select_folder(folder, opened, finished) {
                self.ask_for_daily_folder(finished);
            } else if let Err(error) = self.apply_settings(next) {
                self.notice("Can't save the export folder", error.to_string());
            }
        }
    }

    fn choose_opus_folder(&mut self) {
        let Some(settings) = &mut self.settings else {
            return;
        };
        let mut picker = rfd::FileDialog::new().set_title("Choose OPUS spectrum folder");
        if let Some(parent) = settings
            .draft
            .opus_folder
            .as_deref()
            .and_then(nearest_directory)
        {
            picker = picker.set_directory(parent);
        }
        if let Some(folder) = picker.pick_folder() {
            settings.draft.opus_folder = Some(folder);
        }
    }

    fn open_zip(&mut self, ctx: &Context, path: &Path) {
        match archive::reopen(path, &self.profile) {
            Ok(draft) => {
                self.dialog = None;
                self.push_tab(ctx, draft, TabOrigin::Archive);
            }
            Err(error) => self.notice("Can't open this ZIP", error.to_string()),
        }
    }

    fn choose_photo(&mut self, ctx: &Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Attach sample photo (.jpg or .jpeg)")
            .add_filter(
                "JPG or JPEG photos (*.jpg, *.jpeg)",
                model::PHOTO_EXTENSIONS,
            )
            .pick_file()
        {
            self.add_photo(ctx, &path, false);
        }
    }

    fn add_photo(&mut self, ctx: &Context, path: &Path, dropped: bool) {
        #[cfg(not(target_os = "windows"))]
        let _ = dropped;
        let Some(index) = self.active else {
            self.notice(
                "Open a sample first",
                "Open a sample tab, then attach the JPEG to it.",
            );
            return;
        };
        let copy = match PhotoCopy::from_path(path) {
            Ok(copy) => copy,
            Err(error) => {
                self.notice(
                    "Can't attach this photo",
                    format!("Both .jpg and .jpeg are supported. {error}"),
                );
                return;
            }
        };
        let photo = PendingPhoto {
            copy,
            #[cfg(target_os = "windows")]
            rename_from: dropped.then(|| path.to_owned()),
        };
        if self
            .tabs
            .get(index)
            .is_some_and(|tab| tab.draft.photo.is_some())
        {
            self.dialog = Some(Dialog::ReplacePhoto(index, photo));
        } else {
            self.finish_photo(ctx, index, photo);
        }
    }

    fn finish_photo(&mut self, ctx: &Context, index: usize, pending: PendingPhoto) {
        self.dialog = None;
        #[cfg(target_os = "windows")]
        let mut photo = pending.copy;
        #[cfg(not(target_os = "windows"))]
        let photo = pending.copy;
        #[cfg(target_os = "windows")]
        if let Some(source) = pending.rename_from
            && let Some(tab) = self.tabs.get(index)
        {
            match windows::rename_dropped_photo(&source, &tab.draft.title, &photo) {
                Ok(name) => photo.name = name,
                Err(error) => {
                    self.notice("Photo attached; original not renamed", error.to_string());
                }
            }
        }
        self.set_photo(ctx, index, Some(photo));
    }

    fn set_photo(&mut self, ctx: &Context, index: usize, photo: Option<PhotoCopy>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.thumbnail = photo.as_ref().and_then(|image| ui::thumbnail(ctx, image));
            tab.draft.photo = photo;
            tab.edit();
        }
        self.completion = None;
    }

    fn dropped(&mut self, ctx: &Context) {
        let drop = ctx.input(|input| input.raw.dropped_files.clone());
        if drop.is_empty() || self.worker.busy() {
            return;
        }
        if matches!(
            self.dialog,
            Some(Dialog::Overwrite(..) | Dialog::Repeat(..) | Dialog::Batch(..))
        ) {
            self.status = Some("Finish the open export dialog before dropping a file.".into());
            return;
        }
        if let [file] = drop.as_slice()
            && let Some(path) = &file.path
        {
            let ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default();
            if ext.eq_ignore_ascii_case("zip") {
                self.open_zip(ctx, path);
                return;
            }
            if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
                self.add_photo(ctx, path, true);
                return;
            }
        }
        self.notice(
            "Choose one JPEG or ZIP",
            "Drop one sample JPEG or one exported sample ZIP at a time.",
        );
    }
}

fn analysis_day() -> Result<Date> {
    Ok(Timestamp::now()
        .to_zoned(TimeZone::get("America/Vancouver")?)
        .date())
}

fn nearest_directory(path: &Path) -> Option<&Path> {
    path.ancestors().find(|ancestor| ancestor.is_dir())
}

impl App {
    fn export_active(&mut self, ctx: &Context) {
        if let Some(index) = self.active {
            self.begin_export(
                ctx,
                Selection {
                    tabs: vec![index],
                    wordpress_only: false,
                    confirmed: false,
                },
            );
        }
    }

    fn begin_export(&mut self, ctx: &Context, selection: Selection) {
        let mut missing = Vec::new();
        for index in &selection.tabs {
            if let Some(tab) = self.tabs.get_mut(*index) {
                let before = tab.draft.clone();
                tab.draft.normalize();
                if before != tab.draft {
                    tab.edit();
                }
                tab.missing = tab.draft.has_missing();
                if tab.missing {
                    missing.push(*index);
                }
            }
        }
        if let Some(first) = missing.first() {
            self.select_tab(ctx, *first);
            return;
        }
        if let Err(error) = self.flush(true) {
            self.notice("Can't save the sample", error.to_string());
            return;
        }
        let Ok(day) = analysis_day() else {
            self.notice(
                "Can’t check today’s date",
                "Check the computer clock and try Export again.",
            );
            return;
        };
        let Some(folder) = self.book.settings.today_folder(day).map(Path::to_owned) else {
            self.ask_for_daily_folder(day);
            return;
        };
        self.rescan();
        let drafts: Vec<_> = selection
            .tabs
            .iter()
            .filter_map(|index| self.tabs.get(*index))
            .map(|tab| tab.draft.clone())
            .collect();
        let repeats: Vec<_> = drafts
            .iter()
            .filter(|draft| {
                draft.exported
                    || self
                        .book
                        .history
                        .iter()
                        .any(|history| history.id == draft.id)
            })
            .map(|draft| draft.title.clone())
            .collect();

        if !repeats.is_empty() && !selection.confirmed {
            self.dialog = Some(Dialog::Repeat(selection, repeats));
            return;
        }
        let opus_folder = if self.book.settings.attach_opus {
            let Some(source) = self.book.settings.opus_folder.clone() else {
                self.notice(
                    "Choose an OPUS folder",
                    "Choose the OPUS spectrum folder in Settings, or turn off OPUS attachment.",
                );
                return;
            };
            Some(source)
        } else {
            None
        };
        self.status = None;
        if let Err(error) = self.worker.prepare(
            ctx,
            selection,
            drafts,
            export::Destination {
                folder,
                app_data: self.store.data_folder().to_owned(),
                opus_folder,
            },
            self.book.history.clone(),
            self.profile.clone(),
        ) {
            self.notice("Can't prepare export", error.to_string());
        }
    }

    fn show_overwrite_or_write(&mut self, ctx: &Context, selection: Selection, plan: Box<Plan>) {
        if plan.approved.filenames().len() > 0 {
            self.dialog = Some(Dialog::Overwrite(selection, plan));
        } else {
            self.write_plan(ctx, selection, plan);
        }
    }

    fn write_plan(&mut self, ctx: &Context, selection: Selection, plan: Box<Plan>) {
        let Ok(day) = analysis_day() else {
            self.notice(
                "Can’t check today’s date",
                "Check the computer clock and try Export again.",
            );
            return;
        };
        if self
            .book
            .settings
            .today_folder(day)
            .is_none_or(|folder| !same_file::is_same_file(folder, plan.folder()).unwrap_or(false))
        {
            self.ask_for_daily_folder(day);
            return;
        }
        if let Err(error) = self
            .worker
            .write(ctx, selection, plan, self.profile.clone())
        {
            self.notice("Can't start the export", error.to_string());
        }
    }

    fn retain_codes(&mut self, selection: &Selection, drafts: &[Draft]) {
        for (index, draft) in selection.tabs.iter().zip(drafts) {
            if let Some(tab) = self.tabs.get_mut(*index)
                && tab.draft.code_word != draft.code_word
            {
                tab.draft.code_word.clone_from(&draft.code_word);
                tab.edit();
            }
        }
    }

    fn handle_export(&mut self, ctx: &Context, event: Event) {
        match event {
            Event::Prepared(selection, drafts, result) => {
                self.retain_codes(&selection, &drafts);
                if let Err(error) = self.flush(true) {
                    self.notice("Can't save the sample", error.to_string());
                    return;
                }
                match result {
                    Ok(prepared)
                        if selection.confirmed || prepared.needs_confirmation.is_empty() =>
                    {
                        self.show_overwrite_or_write(ctx, selection, Box::new(prepared.plan));
                    }
                    Ok(prepared) => {
                        self.dialog = Some(Dialog::Repeat(selection, prepared.needs_confirmation));
                    }
                    Err(error) => self.notice("Can't prepare export", error.to_string()),
                }
            }
            Event::Written(selection, mut plan, Err(WriteError::Conflicts(snapshot))) => {
                plan.approved = snapshot;
                self.show_overwrite_or_write(ctx, selection, plan);
            }
            Event::Written(_, _, Err(error)) => {
                self.notice("Export incomplete", export::write_error(&error));
            }
            Event::Written(selection, plan, Ok(success)) => {
                let warning = success.cleanup_warning.map(|warning| warning.to_string());
                self.complete(&selection, &plan, success.written, warning);
            }
            Event::Repeat(selection, names) => self.dialog = Some(Dialog::Repeat(selection, names)),
            Event::Blocked(message) => self.notice("Export stopped", message),
        }
    }

    fn complete(
        &mut self,
        selection: &Selection,
        plan: &Plan,
        files: Vec<PathBuf>,
        warning: Option<String>,
    ) {
        for (index, draft) in selection.tabs.iter().zip(plan.drafts()) {
            self.book.add_history(draft);
            let already_saved = self.records.iter().any(|record| record.id == draft.id);
            if let Some(tab) = self.tabs.get_mut(*index) {
                tab.draft.exported = true;
                tab.draft.suggested_title = None;
                tab.draft.suggested_visit = None;
                if !already_saved {
                    tab.origin = TabOrigin::Recorded;
                }
                tab.edit();
            }
        }
        self.history_pending = true;
        if let Err(error) = self.flush(true) {
            self.save_error = Some(format!(
                "Files were written. Local history still needs to be saved. {error}"
            ));
        }
        self.rescan();
        self.completion = Some(Completion {
            folder: plan.folder().to_owned(),
            files,
            warning,
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        windows::fit_to_work_area(&mut self.last_work_area);
        if let Some(event) = self.worker.poll() {
            self.handle_export(ctx, event);
        }
        self.dropped(ctx);
        self.tick_daily_folder(ctx);
        if self.show_settings
            && self.dialog.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_settings();
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.worker.busy() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if let Err(error) = self.flush(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.notice("Can't save local data", error.to_string());
            }
        } else if self.save_error.is_none()
            && let Err(error) = self.flush(false)
        {
            self.save_error = Some(format!("Local changes still need to be saved. {error}"));
        }
        if self.save_error.is_none() && self.tabs.iter().any(|tab| tab.changed.is_some()) {
            ctx.request_repaint_after(SAVE_DELAY);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
