use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

use eframe::egui;
use gydt_results_compiler::{
    Catalogue, ConflictSnapshot, Contact, ExportRequest, PreparedExport, WriteError, WriteSuccess,
    prepare_export,
};
use jiff::Timestamp;

use super::{
    archive,
    model::{Channel, Draft, Result, filename_stem},
    store::History,
};

#[derive(Clone)]
pub(super) struct Selection {
    pub(super) tabs: Vec<usize>,
    pub(super) wordpress_only: bool,
    pub(super) confirmed: bool,
}

pub(super) struct Plan {
    pub(super) approved: ConflictSnapshot,
    output: PreparedExport,
    drafts: Vec<Draft>,
}

pub(super) struct Prepared {
    pub(super) plan: Plan,
    pub(super) needs_confirmation: Vec<String>,
}

pub(super) enum Event {
    Prepared(Selection, Vec<Draft>, Result<Prepared>),
    Written(
        Selection,
        Box<Plan>,
        std::result::Result<WriteSuccess, WriteError>,
    ),
    Repeat(Selection, Vec<String>),
    Blocked(String),
}

#[derive(Default)]
pub(super) struct Worker {
    rx: Option<mpsc::Receiver<Event>>,
}

impl Worker {
    pub(super) fn busy(&self) -> bool {
        self.rx.is_some()
    }

    pub(super) fn poll(&mut self) -> Option<Event> {
        match self.rx.as_ref()?.try_recv() {
            Ok(event) => {
                self.rx = None;
                Some(event)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                Some(Event::Blocked(
                    "The export stopped unexpectedly. Review the destination before retrying."
                        .into(),
                ))
            }
        }
    }

    fn run(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> Event + Send + 'static,
    ) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        thread::Builder::new()
            .name("gydt-export".into())
            .spawn(move || {
                let _ = tx.send(job());
                ctx.request_repaint();
            })?;
        self.rx = Some(rx);
        Ok(())
    }

    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        selection: Selection,
        mut drafts: Vec<Draft>,
        folder: PathBuf,
        history: Vec<History>,
        catalogue: Arc<Catalogue>,
    ) -> Result<()> {
        self.run(ctx, move || {
            let result = prepare(&selection, &mut drafts, &folder, &history, &catalogue);
            Event::Prepared(selection, drafts, result)
        })
    }

    pub(super) fn write(
        &mut self,
        ctx: &egui::Context,
        selection: Selection,
        plan: Box<Plan>,
        catalogue: Arc<Catalogue>,
    ) -> Result<()> {
        self.run(ctx, move || {
            match check_existing(&plan, &catalogue) {
                Ok(found) if found.is_empty() || selection.confirmed => {}
                Ok(titles) => return Event::Repeat(selection, titles),
                Err(error) => return Event::Blocked(error.to_string()),
            }
            let written = plan.write();
            Event::Written(selection, plan, written)
        })
    }
}

impl Plan {
    pub(super) fn drafts(&self) -> &[Draft] {
        &self.drafts
    }

    pub(super) fn folder(&self) -> &Path {
        self.approved.directory()
    }

    pub(super) fn write(&self) -> std::result::Result<WriteSuccess, WriteError> {
        self.output.write(self.folder(), Some(&self.approved))
    }
}

pub(super) fn batch_recipient<'a>(
    drafts: impl IntoIterator<Item = &'a Draft>,
) -> std::result::Result<Option<(Channel, &'a str)>, &'static str> {
    let mut recipient = None;
    for draft in drafts {
        let address = draft.address.trim();
        if draft.channel == Channel::None || address.is_empty() {
            continue;
        }
        let candidate = (draft.channel, address);
        if *recipient.get_or_insert(candidate) != candidate {
            return Err(
                "Selected samples have different contact details. Select WordPress only or export the samples separately.",
            );
        }
    }
    Ok(recipient)
}

pub(super) fn prepare(
    selection: &Selection,
    drafts: &mut [Draft],
    folder: &Path,
    history: &[History],
    catalogue: &Catalogue,
) -> Result<Prepared> {
    let recipient = if selection.wordpress_only {
        None
    } else {
        batch_recipient(&*drafts)?
    };
    let mut samples = drafts
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            draft
                .to_sample()
                .map_err(|error| format!("Sample {} ({}). {error}", index + 1, draft.tab_label()))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let history = history
        .iter()
        .map(History::entry)
        .collect::<Result<Vec<_>>>()?;
    let confirmed_reexports: Vec<_> = drafts
        .iter()
        .filter(|_| selection.confirmed)
        .map(|draft| draft.id)
        .collect();
    let contact = recipient.map_or(Contact::WordPressOnly, |(channel, address)| {
        channel.contact(address)
    });
    let output = prepare_export(
        &mut samples,
        catalogue,
        ExportRequest {
            contact,
            exported_at: Timestamp::now(),
            history: &history,
            confirmed_reexports: &confirmed_reexports,
        },
    )?;
    for (draft, sample) in drafts.iter_mut().zip(samples) {
        draft.code_word = sample.code_word;
    }
    let approved = output.inspect_folder(folder)?;
    let plan = Plan {
        approved,
        output,
        drafts: drafts.to_vec(),
    };
    let needs_confirmation = check_existing(&plan, catalogue)?;
    Ok(Prepared {
        plan,
        needs_confirmation,
    })
}

fn check_existing(plan: &Plan, catalogue: &Catalogue) -> Result<Vec<String>> {
    let mut same_sample = Vec::new();
    for filename in plan.approved.filenames().filter(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    }) {
        let Ok(existing) = archive::reopen(&plan.folder().join(filename), catalogue) else {
            continue;
        };
        for draft in &plan.drafts {
            let matches_title = draft.title == existing.title
                && (standard_title(&draft.title)
                    || (draft.site == existing.site
                        && draft.analysis_date == existing.analysis_date));
            if matches_title && existing.id != draft.id {
                return Err(format!("Sample number {} has already been used.", draft.title).into());
            }
            if existing.id == draft.id {
                same_sample.push(draft.title.clone());
            }
        }
    }
    Ok(same_sample)
}

fn standard_title(title: &str) -> bool {
    let mut parts = title.split('-');
    let (Some(city), Some(site), Some(year), Some(month), Some(day), Some(serial), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    [city, site, year, month, day, serial]
        .into_iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && site.len() == 3
        && year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && serial.len() >= 3
        && serial.bytes().any(|byte| byte != b'0')
        && super::model::valid_date(&format!("{year}-{month}-{day}")).is_some()
}

pub(super) fn preview(draft: &Draft) -> Option<(String, Option<String>)> {
    if draft.title.trim().is_empty() {
        return None;
    }
    let stem = filename_stem(&super::model::flat_text(&draft.title));
    let client_draft = draft
        .channel
        .extension(&draft.address)
        .map(|ext| format!("{stem}.{ext}"));
    Some((format!("{stem}.zip"), client_draft))
}

pub(super) fn write_error(error: &WriteError) -> String {
    match error {
        WriteError::Prepare { path, error } => format!("Can't use {}. {error}", path.display()),
        WriteError::Interrupted(failure) => {
            let mut message = format!("{failure} {}", failure.error);
            if !failure.written.is_empty() {
                message.push_str("\nFiles already written:");
                for path in &failure.written {
                    let _ = write!(message, "\n{}", path.display());
                }
            }
            message
        }
        WriteError::Cleanup { failure, warning } => format!("{}\n{warning}", write_error(failure)),
        WriteError::RecoveryRequired(recovery) => {
            let originals = recovery
                .originals()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            format!("{recovery}\n{originals}")
        }
        _ => error.to_string(),
    }
}
