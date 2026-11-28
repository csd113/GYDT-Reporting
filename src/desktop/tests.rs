use std::{
    fs,
    io::{Cursor, Read as _, Write as _},
    path::Path,
};

use gydt_results_compiler::{ExportRequest, Profile, prepare_profile_export};
use jiff::Timestamp;
use serde_json::{Value, json};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::{
    archive::reopen,
    model::{Channel, Draft, PhotoCopy, Result, ResultRow, Selection, Site, Strip, digest},
    store::{Book, Sequence},
};

pub(super) fn make_sample() -> Result<Draft> {
    let mut draft = Draft::fresh(Site::ExampleWest, "2026-11-22".parse()?)?;
    draft.title = "Synthetic / specimen".into();
    draft.collection_date = "2026-11-21".into();
    draft.visit = "7".into();
    draft.expected = Some(Selection::Unknown);
    draft.rows = vec![
        ResultRow {
            substance: Some(Selection::Other),
            other: "Synthetic marker".into(),
            ..ResultRow::default()
        },
        ResultRow {
            substance: Some(Selection::Unknown),
            range: "6-12%".into(),
            custom: true,
            ..ResultRow::default()
        },
    ];
    draft.colour = "White".into();
    draft.texture = "Pressed Pill".into();
    draft.shape = "other".into();
    draft.shape_other = "Synthetic shape".into();
    draft.used = Some(true);
    draft.effects = Some(false);
    draft.strips = [Strip::Blank, Strip::Negative, Strip::Blank, Strip::Positive];
    draft.code_word = "SYNTH0001".into();
    draft.notes = "Synthetic sample only.".into();
    draft.channel = Channel::Email;
    draft.address = "gydt-synthetic@example.invalid".into();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode(
        &[60, 80, 100].repeat(64),
        8,
        8,
        image::ExtendedColorType::Rgb8,
    )?;
    draft.photo = Some(PhotoCopy::new("original.JPEG", "JPEG", &jpeg));
    Ok(draft)
}

pub(super) fn fixture_profile() -> Result<Profile> {
    let tmp = tempfile::tempdir()?;
    write_fixture_profile(tmp.path(), ["fixture-east", "fixture-west"])?;
    Ok(Profile::load(tmp.path())?)
}

fn write_fixture_profile(root: &Path, [first, second]: [&str; 2]) -> Result<()> {
    let xml = include_bytes!("../../data/substances-catalog.xml");
    fs::write(root.join("substances-catalog.xml"), xml)?;
    fs::write(
        root.join("results-email.eml"),
        include_bytes!("../../templates/results-email.eml"),
    )?;
    fs::write(
        root.join("results-sms.txt"),
        include_bytes!("../../templates/results-sms.txt"),
    )?;
    fs::write(
        root.join("profile.json"),
        serde_json::to_vec(&json!({
            "format_version": 1,
            "profile_id": "synthetic-lab",
            "catalogue_sha256": digest(xml),
            "sites":[
                {"label":"Synthetic East","token":first,"code":"901"},
                {"label":"Synthetic West","token":second,"code":"901"}
            ]
        }))?,
    )?;
    Ok(())
}

pub(super) fn export_sample(draft: &Draft, profile: &Profile, folder: &Path) -> Result<()> {
    let export = prepare_profile_export(
        &mut [draft.to_sample()?],
        profile,
        ExportRequest {
            contact: draft.channel.contact(&draft.address),
            exported_at: Timestamp::now(),
            history: &[],
            confirmed_reexports: &[],
        },
    )?;
    let approval = export.inspect_folder(folder)?;
    export.write(folder, Some(&approval))?;
    Ok(())
}

fn members(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut zip = ZipArchive::new(fs::File::open(path)?)?;
    let mut result = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name()?.into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        result.push((name, bytes));
    }
    Ok(result)
}

fn repackage(path: &Path, members: &[(String, Vec<u8>)]) -> Result<()> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let option = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in members.iter().rev() {
        zip.start_file(name, option)?;
        zip.write_all(bytes)?;
    }
    fs::write(path, zip.finish()?.into_inner())?;
    Ok(())
}

#[test]
fn desktop_file_name_preview_matches_single_sample_outputs() -> Result<()> {
    let catalogue = fixture_profile()?;
    let mut draft = make_sample()?;
    for (title, channel, address) in [
        (
            "Synthetic / specimen",
            Channel::Email,
            "gydt-synthetic@example.invalid",
        ),
        ("Synthetic / specimen", Channel::Text, "+16045550123"),
        ("CON", Channel::Email, ""),
        ("CON .fixture", Channel::None, ""),
    ] {
        draft.title = title.into();
        draft.channel = channel;
        draft.address = address.into();
        let (zip, client_draft) = super::export::preview(&draft).ok_or("Missing file names")?;
        let prepared = prepare_profile_export(
            &mut [draft.to_sample()?],
            &catalogue,
            ExportRequest {
                contact: channel.contact(address),
                exported_at: Timestamp::now(),
                history: &[],
                confirmed_reexports: &[],
            },
        )?;
        assert_eq!(
            std::iter::once(zip.as_str())
                .chain(client_draft.as_deref())
                .collect::<Vec<_>>(),
            prepared.filenames().collect::<Vec<_>>(),
        );
    }
    Ok(())
}

#[test]
fn desktop_reopens_exported_zip_with_same_identity_and_no_contact() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let catalogue = fixture_profile()?;
    let draft = make_sample()?;
    export_sample(&draft, &catalogue, folder.path())?;
    let stem = format!("sample-{}", digest(draft.title.as_bytes()));
    let path = folder.path().join(format!("{stem}.zip"));
    let stored = reopen(&path, &catalogue)?;
    let compressed_path = folder.path().join("renamed.zip");
    repackage(&compressed_path, &members(&path)?)?;
    let compressed = reopen(&compressed_path, &catalogue)?;
    assert_eq!(stored, compressed);
    assert_eq!(stored.id, draft.id);
    assert_eq!(stored.title, draft.title);
    assert_eq!(
        (stored.analysis_date, stored.collection_date),
        (draft.analysis_date, draft.collection_date)
    );
    assert_eq!(stored.rows, draft.rows);
    assert_eq!(stored.strips, draft.strips);
    assert!(stored.exported);
    assert_eq!(stored.channel, Channel::None);
    assert_eq!(stored.address, "");
    let restored = stored.photo.ok_or("Photo was lost during ZIP reopen")?;
    assert_eq!(restored.name, format!("{stem}.jpg"));
    let mut legacy = members(&path)?;
    let legacy_name = format!("{stem}.JPEG");
    legacy
        .iter_mut()
        .find(|(name, _)| *name == restored.name)
        .ok_or("missing canonical image")?
        .0
        .clone_from(&legacy_name);
    let (_, manifest) = legacy
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("missing manifest")?;
    let mut metadata: Value = serde_json::from_slice(manifest)?;
    let files = metadata
        .get_mut("files")
        .and_then(Value::as_object_mut)
        .ok_or("missing manifest files")?;
    let member = files
        .remove(&restored.name)
        .ok_or("missing canonical member")?;
    files.insert(legacy_name.clone(), member);
    *manifest = serde_json::to_vec(&metadata)?;
    let legacy_path = folder.path().join("legacy.zip");
    repackage(&legacy_path, &legacy)?;
    let legacy = reopen(&legacy_path, &catalogue)?
        .photo
        .ok_or("missing legacy JPEG")?;
    assert_eq!(legacy.name, legacy_name);
    assert_eq!(legacy.bytes()?, restored.bytes()?);
    assert_eq!(
        restored.bytes()?,
        draft.photo.ok_or("Expected synthetic JPEG")?.bytes()?
    );
    assert!(folder.path().join(format!("{stem}.eml")).is_file());
    Ok(())
}

#[test]
fn dropping_a_zip_keeps_open_edits_and_legacy_records() -> Result<()> {
    let root = tempfile::tempdir()?;
    let exports = root.path().join("exports");
    let samples = root.path().join("samples");
    fs::create_dir(&exports)?;
    fs::create_dir(&samples)?;
    let catalogue = fixture_profile()?;
    let draft = make_sample()?;
    export_sample(&draft, &catalogue, &exports)?;
    let zip = exports.join(format!("sample-{}.zip", digest(draft.title.as_bytes())));
    let source = fs::read(&zip)?;
    let legacy = serde_json::to_vec(&draft)?;
    let previous = samples.join(format!("{}.json", draft.id));
    fs::write(&previous, &legacy)?;

    let (ctx, mut app) = open_app(root.path())?;
    let current = app.tabs.first_mut().ok_or("Missing startup tab")?;
    current.draft.notes = "Unsaved intake notes".into();
    current.edit();
    let intake = current.draft.clone();
    let drop_file = |app: &mut super::App, path: &Path| {
        let input = eframe::egui::RawInput {
            dropped_files: vec![eframe::egui::DroppedFile {
                path: Some(path.to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.dropped(ui.ctx()));
    };
    drop_file(&mut app, &zip);
    let reopened = app.tabs.last_mut().ok_or("Missing ZIP tab")?;
    assert_eq!(app.active, Some(1));
    assert_eq!(reopened.draft.id, draft.id);
    assert!(reopened.draft.exported);
    assert_eq!(reopened.draft.channel, Channel::None);
    assert_eq!(reopened.draft.address, "");
    reopened.draft.notes = "Unsaved edit to the reopened sample".into();
    reopened.edit();
    let edited_archive = reopened.draft.clone();

    drop_file(&mut app, &zip);
    let opened_again = app.tabs.last().ok_or("Missing second ZIP tab")?;
    assert_eq!(app.active, Some(2));
    assert_eq!(opened_again.draft.id, draft.id);
    assert_eq!(opened_again.draft.notes, draft.notes);
    let invalid = exports.join("invalid.zip");
    fs::write(&invalid, b"Invalid synthetic ZIP")?;
    let expected: Vec<_> = app.tabs.iter().map(|tab| tab.draft.clone()).collect();
    drop_file(&mut app, &invalid);
    assert!(matches!(
        app.dialog,
        Some(super::Dialog::Notice("Can't open this ZIP", _))
    ));
    assert_eq!(app.active, Some(2));
    assert!(app.tabs.iter().map(|tab| &tab.draft).eq(&expected));
    assert_eq!(expected.first(), Some(&intake));
    assert_eq!(expected.get(1), Some(&edited_archive));
    assert_eq!(fs::read(zip)?, source);
    assert_eq!(fs::read(previous)?, legacy);
    let day = draft.analysis_date.parse()?;
    assert_eq!(app.book.proposed_visit(draft.site, day, &app.records)?, 8);
    Ok(())
}

#[test]
fn desktop_rejects_invalid_json_and_unsupported_zip_before_populating_form() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let catalogue = fixture_profile()?;
    let draft = make_sample()?;
    export_sample(&draft, &catalogue, folder.path())?;
    let stem = format!("sample-{}", digest(draft.title.as_bytes()));
    let path = folder.path().join(format!("{stem}.zip"));
    let original = members(&path)?;
    let manifest_bytes = |entries: &[(String, Vec<u8>)]| -> Result<Vec<u8>> {
        Ok(entries
            .iter()
            .find(|(name, _)| name == "manifest.json")
            .ok_or("missing synthetic manifest")?
            .1
            .clone())
    };
    let invalid = folder.path().join("invalid.zip");
    let manifest: Value = serde_json::from_slice(&manifest_bytes(&original)?)?;

    let mut broken = original.clone();
    let item = broken
        .iter_mut()
        .find(|(name, _)| name == "test_data.json")
        .ok_or("missing data")?;
    item.1.push(b' ');
    repackage(&invalid, &broken)?;
    assert!(reopen(&invalid, &catalogue).is_err());

    let mut broken = original.clone();
    let item = broken
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("missing manifest")?;
    let mut unsupported = manifest.clone();
    *unsupported
        .get_mut("format_version")
        .ok_or("missing version")? = json!(2);
    item.1 = serde_json::to_vec(&unsupported)?;
    repackage(&invalid, &broken)?;
    assert!(reopen(&invalid, &catalogue).is_err());

    let mut broken = original;
    let item = broken
        .iter_mut()
        .find(|(name, _)| name == "test_data.json")
        .ok_or("missing data")?;
    let mut invalid_payload: Value = serde_json::from_slice(&item.1)?;
    *invalid_payload.get_mut("notes").ok_or("missing notes")? = Value::Null;
    item.1 = serde_json::to_vec(&invalid_payload)?;
    let mut adjusted = manifest;
    *adjusted
        .get_mut("files")
        .and_then(|files| files.get_mut("test_data.json"))
        .ok_or("missing member metadata")? =
        json!({ "bytes": item.1.len(), "sha256": digest(&item.1) });
    let item = broken
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("missing manifest")?;
    item.1 = serde_json::to_vec(&adjusted)?;
    repackage(&invalid, &broken)?;
    assert!(reopen(&invalid, &catalogue).is_err());
    Ok(())
}

#[test]
fn shared_prefix_scans_only_primary_sample_filenames_and_survives_a_missing_folder() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let book = Book::for_profile(&fixture_profile()?);
    let day = "2026-11-22".parse()?;
    let east = book.settings.prefix(Site::ExampleEast, day);
    let west = book.settings.prefix(Site::ExampleWest, day);
    assert_eq!(east, west);
    for file in [
        "3-901-2026-11-22-007.zip",
        "3-901-2026-11-22-091.txt",
        "3-901-2026-11-22-098.zip.tmp",
        "3-901-2026-11-21-099.zip",
        "3-019-2026-11-22-077.zip",
    ] {
        fs::write(folder.path().join(file), "synthetic filename only")?;
    }
    let mut sequence = Sequence::default();
    assert!(sequence.next(&east, &book, &[]).is_none());
    sequence.scan(Some(folder.path()));
    assert_eq!(
        sequence.next(&east, &book, &[]).as_deref(),
        Some("3-901-2026-11-22-008")
    );
    sequence.scan(Some(&folder.path().join("gone")));
    assert_eq!(
        sequence.next(&west, &book, &[]).as_deref(),
        Some("3-901-2026-11-22-009")
    );
    assert!(sequence.next("3-901-2026-11-23-", &book, &[]).is_none());
    Ok(())
}

fn open_app(root: &Path) -> Result<(eframe::egui::Context, super::App)> {
    use super::{App, store::Store, ui};
    use std::sync::Arc;

    let store = Store::open_at(root.to_owned())?;
    let profile = Arc::new(fixture_profile()?);
    let book = store.book(&profile)?;
    let records = store.records()?;
    let ctx = eframe::egui::Context::default();
    let app = App::new(&ctx, store, book, records, profile, &ui::orbit()?);
    Ok((ctx, app))
}

#[test]
fn startup_and_site_correction_leave_visit_proposals_unclaimed() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let tab = app.tabs.first_mut().ok_or("Missing startup tab")?;
    assert_eq!(tab.draft.visit, "1");
    assert!(tab.origin.is_provisional());
    tab.draft.site = Site::ExampleWest;
    tab.edit();
    app.switch_site(0, Site::ExampleEast)?;
    let tab = app.tabs.first().ok_or("Missing corrected tab")?;
    assert_eq!(tab.draft.visit, "1");
    assert!(tab.origin.is_provisional());
    app.dialog = None;
    app.close_tab(&ctx, 0);
    app.flush(true)?;
    assert_eq!(app.store.records()?.len(), 0);
    assert!(!root.path().join("book.json").exists());
    drop(app);

    let (_, app) = open_app(root.path())?;
    assert_eq!(
        app.tabs.first().ok_or("Missing restarted tab")?.draft.visit,
        "1"
    );
    assert_eq!(app.store.records()?.len(), 0);
    Ok(())
}

#[test]
fn entered_samples_keep_visit_claims_and_deliberate_repeats_across_restart() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let tab = app.tabs.first_mut().ok_or("Missing startup tab")?;
    let day = super::model::valid_date(&tab.draft.analysis_date).ok_or("Missing analysis date")?;
    let id = tab.draft.id;
    tab.draft.notes = "Start by entering sample notes.".into();
    tab.edit();
    app.flush(true)?;
    app.create_tab(&ctx, true)?;
    let repeat = app.tabs.last_mut().ok_or("Missing repeat tab")?;
    assert_eq!(repeat.draft.visit, "1");
    assert_ne!(repeat.draft.id, id);
    repeat.draft.colour = "White".into();
    repeat.edit();
    app.flush(true)?;
    app.create_tab(&ctx, false)?;
    let later = app.tabs.last_mut().ok_or("Missing new intake")?;
    assert_eq!(later.draft.visit, "2");
    later.draft.notes = "Start a separate intake.".into();
    later.edit();
    app.flush(true)?;

    // A staff correction can repeat Visit 1 without reusing the committed high-water mark.
    let later = app.tabs.last_mut().ok_or("Missing saved intake")?;
    later.draft.visit = "1".into();
    later.draft.suggested_visit = None;
    later.edit();
    app.flush(true)?;
    let stored: Value = serde_json::from_slice(&fs::read(
        root.path().join("samples").join(format!("{id}.json")),
    )?)?;
    assert_eq!(stored.get("id"), Some(&json!(id)));
    let keys = stored.as_object().ok_or("Invalid numbering record")?;
    assert_eq!(
        keys.keys().map(String::as_str).collect::<Vec<_>>(),
        ["analysis_date", "id", "site", "title", "visit_claims"],
    );
    drop(app);

    let (_, app) = open_app(root.path())?;
    assert_eq!(app.store.records()?.len(), 3);
    assert_eq!(
        app.book
            .proposed_visit(Site::ExampleEast, day, &app.records)?,
        3
    );
    assert_eq!(
        app.book
            .proposed_visit(Site::ExampleWest, day, &app.records)?,
        1
    );
    assert_eq!(
        app.book
            .proposed_visit(Site::ExampleEast, day.tomorrow()?, &app.records)?,
        1
    );
    Ok(())
}

#[test]
fn saving_a_hidden_settings_edit_keeps_the_new_main_folder() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let earlier = root.path().join("earlier");
    let later = root.path().join("later");
    fs::create_dir(&earlier)?;
    fs::create_dir(&later)?;
    let mut saved = app.book.settings.clone();
    saved.folder = Some(earlier);
    app.apply_settings(saved)?;
    app.open_settings();
    let settings = app.settings.as_mut().ok_or("Missing Settings")?;
    settings.draft.site = Site::ExampleWest;
    app.select_tab(&ctx, 0);
    let mut saved = app.book.settings.clone();
    saved.folder = Some(later.clone());
    app.apply_settings(saved)?;
    app.open_settings();
    let pane = app.settings.as_ref().ok_or("Missing reopened Settings")?;
    assert_eq!(pane.draft.site, Site::ExampleWest);
    assert_eq!(pane.draft.folder.as_deref(), Some(later.as_path()));
    app.save_settings();
    drop(app);

    let (_, app) = open_app(root.path())?;
    assert_eq!(app.book.settings.site, Site::ExampleWest);
    assert_eq!(app.book.settings.folder.as_deref(), Some(later.as_path()));
    assert_eq!(app.store.records()?.len(), 0);
    Ok(())
}

#[cfg(target_os = "windows")]
#[test]
fn camera_settings_save_and_cancel_use_the_stable_id_not_the_label() -> Result<()> {
    use super::webcam::{Camera, set_selection};

    let root = tempfile::tempdir()?;
    let first = Camera {
        id: "built-in-id".into(),
        name: "Color Camera".into(),
    };
    let external = Camera {
        id: "usb-stable-id".into(),
        name: first.name.clone(),
    };
    let (_, mut app) = open_app(root.path())?;
    assert!(app.webcam_target().is_none());
    app.open_settings();
    let draft = &mut app.settings.as_mut().ok_or("Missing Settings")?.draft;
    draft.webcam_enabled = true;
    set_selection(draft, &first);
    app.cancel_settings();
    assert!(!app.book.settings.webcam_enabled && app.book.settings.webcam_camera.is_none());

    app.open_settings();
    let draft = &mut app.settings.as_mut().ok_or("Missing Settings")?.draft;
    draft.webcam_enabled = true;
    set_selection(draft, &external);
    app.save_settings();
    drop(app);

    let (_, mut reopened) = open_app(root.path())?;
    assert!(reopened.book.settings.webcam_enabled);
    assert_eq!(
        reopened.book.settings.webcam_camera.as_deref(),
        Some(external.id.as_str())
    );
    assert_eq!(
        reopened.book.settings.webcam_camera_name.as_deref(),
        Some(external.name.as_str())
    );
    assert!(
        !root.path().join("camera").exists(),
        "Settings save alone should not start the helper"
    );
    reopened.open_settings();
    let draft = &mut reopened.settings.as_mut().ok_or("Missing Settings")?.draft;
    set_selection(draft, &first);
    reopened.cancel_settings();
    assert_eq!(reopened.book.settings.webcam_camera, Some(external.id));
    Ok(())
}

fn plain_email(draft: &str) -> Result<String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let (_, body) = draft
        .split_once(
            "Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n",
        )
        .ok_or("Missing plain email part")?;
    let encoded: String = body
        .lines()
        .take_while(|line| !line.starts_with("--"))
        .collect();
    Ok(String::from_utf8(STANDARD.decode(encoded)?)?)
}

#[test]
fn combined_contact_resolves_across_tab_orders_and_uses_the_first_title() -> Result<()> {
    use super::export::{self, Selection as Batch};

    let catalogue = fixture_profile()?;
    let all = Batch {
        tabs: vec![0, 1],
        wordpress_only: false,
        confirmed: false,
    };
    for (channel, address, extension) in [
        (Channel::Email, "case@example.invalid", "eml"),
        (Channel::Text, "+1 604 555 0100", "txt"),
    ] {
        let mut without = make_sample()?;
        without.title = "Synthetic-A".into();
        without.address.clear();
        without.notes = "Alpha marker".into();
        let mut with = make_sample()?;
        with.title = "Synthetic-B".into();
        with.channel = channel;
        with.address = address.into();
        with.notes = "Beta marker".into();
        for mut drafts in [
            [without.clone(), with.clone()],
            [with.clone(), without.clone()],
        ] {
            let folder = tempfile::tempdir()?;
            let prepared =
                export::prepare(&all, &mut drafts, folder.path(), &[], &catalogue, None)?;
            let [first, second] = &drafts;
            assert_eq!((prepared.plan.write()?).written.len(), 3);
            let draft =
                fs::read_to_string(folder.path().join(format!("{}.{extension}", first.title)))?;
            let text = if channel == Channel::Email {
                assert!(draft.contains("To: case@example.invalid"));
                plain_email(&draft)?
            } else {
                draft
            };
            assert!(
                text.find(&first.notes)
                    .zip(text.find(&second.notes))
                    .is_some_and(|(first, second)| first < second)
            );
            assert!(
                !folder
                    .path()
                    .join(format!("{}.{extension}", second.title))
                    .exists()
            );
        }
        let folder = tempfile::tempdir()?;
        without.address = "conflicting@example.invalid".into();
        assert!(
            export::prepare(
                &all,
                &mut [without.clone(), with],
                folder.path(),
                &[],
                &catalogue,
                None
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(folder.path())?.count(), 0);
        without.address.clear();
        let mut other = make_sample()?;
        other.title = "Synthetic-B".into();
        other.channel = Channel::None;
        let wordpress_only = export::prepare(
            &all,
            &mut [without, other],
            folder.path(),
            &[],
            &catalogue,
            None,
        )?;
        assert_eq!((wordpress_only.plan.write()?).written.len(), 2);
    }
    Ok(())
}

#[test]
fn desktop_previews_arithmetic_jpegs_and_exports_the_original_bytes() -> Result<()> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use oxiarc_jpeg::{
        EncodeOptions, EncodeProcess, EntropyCoding, InputColor, encode_to_vec_with_options,
    };

    let sequential = STANDARD.decode(concat!(
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsK",
        "CwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/yQALCAAGAAgBAREA/8wABgAQEAX/2gAI",
        "AQEAAD8A/wCz8oFoPv6isdM4NyFH0ArNYsDXzroyVkWfK1DG5cVEFFAwZPt5BxM1iNCRaf/Z"
    ))?;
    let progressive = encode_to_vec_with_options(
        &[40, 80, 150].repeat(16 * 16),
        16,
        16,
        InputColor::Rgb,
        &EncodeOptions {
            process: EncodeProcess::Progressive,
            entropy: EntropyCoding::Arithmetic,
            ..EncodeOptions::default()
        },
    )?;
    assert!(sequential.windows(2).any(|pair| pair == [0xff, 0xc9]));
    assert!(progressive.windows(2).any(|pair| pair == [0xff, 0xca]));
    let catalogue = fixture_profile()?;
    for (jpeg, dimensions) in [(sequential, (8, 6)), (progressive, (16, 16))] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("source.JPG");
        fs::write(&path, &jpeg)?;
        let mut draft = make_sample()?;
        draft.title = "Synthetic JPEG preview".into();
        draft.channel = Channel::None;
        draft.photo = Some(PhotoCopy::from_path(&path)?);
        let preview = super::ui::thumbnail_pixels(&jpeg).ok_or("Arithmetic preview failed")?;
        assert_eq!(preview.dimensions(), dimensions);
        assert!(preview.pixels().any(|pixel| {
            let [r, g, b, _] = pixel.0;
            [r, g, b] != [0; 3]
        }));
        export_sample(&draft, &catalogue, root.path())?;
        let saved = reopen(&root.path().join("Synthetic JPEG preview.zip"), &catalogue)?;
        assert_eq!(saved.photo.ok_or("Missing exported JPEG")?.bytes()?, jpeg);
        assert_eq!(fs::read(path)?, jpeg);
    }
    Ok(())
}

#[test]
fn picker_exports_canonical_jpg_without_changing_source_or_image_bytes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let jpeg = make_sample()?
        .photo
        .ok_or("Missing fixture JPEG")?
        .bytes()?;
    for extension in ["jpg", "JPG", "jpeg"] {
        let path = root.path().join(format!("synthetic-camera.{extension}"));
        fs::write(&path, &jpeg)?;
        let tab = app.tabs.first_mut().ok_or("Missing sample tab")?;
        tab.draft = make_sample()?;
        tab.draft.channel = Channel::None;
        tab.draft.photo = None;
        tab.draft.title = "Synthetic camera sample".into();
        app.add_photo(&ctx, &path, false);
        assert!(app.dialog.is_none());
        let tab = app.tabs.first().ok_or("Missing sample tab")?;
        assert!(tab.thumbnail.is_some());
        let photo = tab.draft.photo.as_ref().ok_or("Missing attached photo")?;
        assert_eq!(photo.extension, extension);
        assert_eq!(photo.bytes()?, jpeg);
        let exports = tempfile::tempdir()?;
        export_sample(&tab.draft, &app.profile, exports.path())?;
        assert!(
            members(&exports.path().join("Synthetic camera sample.zip"))?
                .contains(&("Synthetic camera sample.jpg".into(), jpeg.clone()))
        );
        assert_eq!(fs::read(&path)?, jpeg);
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
#[test]
fn dropped_photo_renames_without_overwriting_and_does_not_move_before_confirmation() -> Result<()> {
    use eframe::egui::{DroppedFile, RawInput};

    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let jpeg = make_sample()?
        .photo
        .ok_or("Missing fixture JPEG")?
        .bytes()?;
    let tab = app.tabs.first_mut().ok_or("Missing sample tab")?;
    tab.draft.title = "SYNTHETIC-901".into();
    let first = root.path().join("camera-one.JPG");
    let second = root.path().join("camera-two.jpeg");
    let target = root.path().join("SYNTHETIC-901.jpg");
    let send_drop = |app: &mut super::App, path: &Path| {
        let _ = ctx.run_ui(
            RawInput {
                dropped_files: vec![DroppedFile {
                    path: Some(path.to_owned()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            |ui| app.dropped(ui.ctx()),
        );
    };
    fs::write(&first, &jpeg)?;
    fs::write(&second, &jpeg)?;
    send_drop(&mut app, &first);
    assert!(!first.exists());
    assert_eq!(fs::read(&target)?, jpeg);
    send_drop(&mut app, &second);
    let Some(super::Dialog::ReplacePhoto(index, photo)) = app.dialog.take() else {
        return Err("Expected explicit replace confirmation".into());
    };
    assert_eq!(fs::read(&second)?, jpeg);
    app.finish_photo(&ctx, index, photo);
    assert!(matches!(
        app.dialog,
        Some(super::Dialog::Notice(
            "Photo attached; original not renamed",
            _
        ))
    ));
    assert_eq!(fs::read(&second)?, jpeg);
    assert_eq!(fs::read(&target)?, jpeg);
    let tab = app.tabs.first().ok_or("Missing sample tab")?;
    assert_eq!(
        tab.draft
            .photo
            .as_ref()
            .ok_or("No attached photo")?
            .bytes()?,
        jpeg
    );
    Ok(())
}

#[test]
fn rejected_picker_photo_keeps_current_tab_and_attachment() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(temp.path())?;
    let tab = app.tabs.first_mut().ok_or("Missing sample tab")?;
    let fake = make_sample()?;
    let clean = fake.photo.as_ref().ok_or("Missing fixture JPEG")?.bytes()?;
    tab.draft = fake;
    let original = tab.draft.clone();
    let source = temp
        .path()
        .join("synthetic-webcam-with-invalid-segment.jpg");
    let mut metadata = vec![
        0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff, b'E', b'x', b'i', b'f', 0, 0,
    ];
    metadata.extend_from_slice(clean.get(2..).ok_or("Invalid synthetic JPEG")?);
    fs::write(&source, &metadata)?;
    app.add_photo(&ctx, &source, false);
    let Some(super::Dialog::Notice("Can't attach this photo", reason)) = app.dialog else {
        return Err("The supported-extension photo did not show its validation reason".into());
    };
    assert!(reason.contains("Both .jpg and .jpeg are supported"));
    assert!(reason.contains("invalid structure"));
    assert_eq!(app.active, Some(0));
    assert_eq!(app.tabs.first().map(|tab| &tab.draft), Some(&original));
    assert_eq!(fs::read(&source)?, metadata);
    Ok(())
}

#[cfg(target_os = "windows")]
#[test]
fn picker_distinguishes_locked_camera_files_from_oversize_files() -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;

    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let oversized = root.path().join("synthetic-too-large.jpg");
    fs::File::create(&oversized)?.set_len(super::model::MAX_JPEG + 1)?;
    app.add_photo(&ctx, &oversized, false);
    assert!(
        matches!(&app.dialog, Some(super::Dialog::Notice(_, reason)) if reason.contains("exceeds 4 MiB"))
    );
    let locked = root.path().join("synthetic-locked.jpg");
    let mut handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&locked)?;
    handle.write_all(&make_sample()?.photo.ok_or("No synthetic JPEG")?.bytes()?)?;
    app.add_photo(&ctx, &locked, false);
    assert!(
        matches!(&app.dialog, Some(super::Dialog::Notice(_, reason)) if reason.contains("os error 32"))
    );
    assert!(
        app.tabs
            .first()
            .is_some_and(|tab| tab.draft.photo.is_none())
    );
    assert_eq!(app.active, Some(0));
    Ok(())
}

#[test]
fn private_profile_supplies_export_tokens_and_separate_state() -> Result<()> {
    use super::store::Store;
    let root = tempfile::tempdir()?;
    let missing = root.path().join("missing");
    assert!(Profile::load(&missing).is_err());
    assert!(!missing.exists());

    let first = root.path().join("first");
    let second = root.path().join("second");
    let exports = root.path().join("exports");
    for dir in [&first, &second, &exports] {
        fs::create_dir(dir)?;
    }
    write_fixture_profile(&first, ["trial-a", "trial-b"])?;
    write_fixture_profile(&second, ["test-a", "test-b"])?;
    let profile = Profile::load(&first)?;
    let mismatch = Profile::load(&second)?;
    let staff = root.path().join("GYDT Sample Reporting");
    fs::create_dir(&staff)?;
    fs::write(staff.join("book.json"), b"synthetic staff sentinel")?;
    let location = Store::profile_root(root.path(), &profile);
    assert_ne!(location, Store::profile_root(root.path(), &mismatch));
    let store = Store::open_at(location)?;
    assert!(store.book(&profile)?.history.is_empty());
    assert_eq!(
        fs::read(staff.join("book.json"))?,
        b"synthetic staff sentinel"
    );
    assert_eq!(fs::read_dir(staff)?.count(), 1);

    let mut draft = make_sample()?;
    draft.channel = Channel::None;
    draft.address.clear();
    export_sample(&draft, &profile, &exports)?;
    let path = exports.join(format!("{}.zip", super::model::filename_stem(&draft.title)));
    let files = members(&path)?;
    let payload = files
        .iter()
        .find(|(name, _)| name == "test_data.json")
        .ok_or("missing data")?;
    let data: Value = serde_json::from_slice(&payload.1)?;
    assert_eq!(data.get("site"), Some(&json!("trial-b")));
    assert_eq!(reopen(&path, &profile)?.site, draft.site);
    assert!(reopen(&path, &mismatch).is_err());

    fs::write(first.join("results-sms.txt"), b"not a valid template")?;
    assert!(Profile::load(&first).is_err());
    write_fixture_profile(&first, ["trial-a", "trial-b"])?;
    fs::write(first.join("substances-catalog.xml"), b"not the pinned XML")?;
    assert!(Profile::load(&first).is_err());
    Ok(())
}

#[test]
fn embedded_profile_preserves_previous_staff_store_and_jpeg_export() -> Result<()> {
    use super::store::Store;
    let profile = Profile::embedded()?;
    let sidecar = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/embedded-profile");
    let previous = Profile::load(&sidecar)?;
    let root = tempfile::tempdir()?;
    assert_eq!(
        Store::profile_root(root.path(), &profile),
        Store::profile_root(root.path(), &previous)
    );
    assert_eq!(
        profile.catalogue().version(),
        previous.catalogue().version()
    );
    assert_eq!(profile.catalogue().records().len(), 868);

    let option = |label: &str| {
        profile
            .catalogue()
            .records()
            .find(|r| r.label == label)
            .map(|record| Selection::Catalogue(record.id))
            .ok_or("Missing required catalogue option")
    };
    let mut input = make_sample()?;
    input.channel = Channel::None;
    input.address.clear();
    input.title = "Synthetic embedded profile".into();
    input.expected = Some(option("Fentanyl")?);
    input.rows = vec![
        ResultRow {
            substance: Some(option("4-MMC")?),
            ..ResultRow::default()
        },
        ResultRow {
            substance: Some(Selection::Unknown),
            ..ResultRow::default()
        },
    ];
    let photo = input
        .photo
        .as_ref()
        .ok_or("Missing synthetic JPG")?
        .bytes()?;
    export_sample(&input, &profile, root.path())?;
    let path = root.path().join("Synthetic embedded profile.zip");
    let roundtrip = reopen(&path, &profile)?;
    assert_eq!(roundtrip.expected, input.expected);
    assert!(
        roundtrip
            .rows
            .iter()
            .map(|r| r.substance)
            .eq(input.rows.iter().map(|r| r.substance))
    );
    assert_eq!(
        roundtrip.photo.ok_or("Missing exported JPEG")?.bytes()?,
        photo
    );
    Ok(())
}

#[test]
fn opus_batch_preflight_and_immutable_zip_and_sibling() -> Result<()> {
    use std::io::Read as _;

    let profile = fixture_profile()?;
    let source = tempfile::tempdir()?;
    let destination = tempfile::tempdir()?;
    let mut first = make_sample()?;
    first.title = "Synthetic-A".into();
    first.channel = Channel::Email;
    first.address = "case@example.invalid".into();
    let mut second = make_sample()?;
    second.title = "Synthetic-B".into();
    second.channel = Channel::Email;
    second.address = first.address.clone();
    let mut drafts = [first, second];
    let batch = super::export::Selection {
        tabs: vec![0, 1],
        wordpress_only: false,
        confirmed: false,
    };
    let export = |drafts: &mut [Draft], opus| {
        super::export::prepare(&batch, drafts, destination.path(), &[], &profile, opus)
    };
    let a = source.path().join("Synthetic-A.0");
    let b = source.path().join("Synthetic-B.0");
    let old = destination.path().join("Synthetic-A.0");
    fs::write(&a, b"\0first\xff")?;
    fs::write(source.path().join("synthetic-b.0"), b"wrong case")?;
    fs::write(&old, b"existing")?;
    let before = drafts.clone();
    let Err(blocked) = export(&mut drafts, Some(source.path())) else {
        return Err("A case-mismatched OPUS name was accepted.".into());
    };
    assert!(blocked.to_string().contains("Can't find Synthetic-B.0"));
    assert_eq!(drafts, before);
    assert_eq!(fs::read(&old)?, b"existing");
    assert_eq!(fs::read_dir(destination.path())?.count(), 1);
    assert!(gydt_results_compiler::OpusSource::new(source.path(), source.path()).is_err());
    let off = export(&mut drafts, None)?;
    assert_eq!(off.plan.write()?.written.len(), 3);
    assert!(!destination.path().join("Synthetic-B.0").exists());
    fs::remove_file(source.path().join("synthetic-b.0"))?;
    fs::write(&b, b"\xfeexact second\0")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        let exclusive = fs::OpenOptions::new().read(true).share_mode(0).open(&b)?;
        let Err(blocked) = export(&mut drafts, Some(source.path())) else {
            return Err("An actual deny-read handle did not block the batch.".into());
        };
        assert!(blocked.to_string().contains("Unload Synthetic-B.0 in OPUS"));
        assert_eq!(fs::read(&old)?, b"existing");
        assert_eq!(fs::read_dir(destination.path())?.count(), 4);
        drop(exclusive);
    }
    let full = export(&mut drafts, Some(source.path()))?;
    assert_eq!(full.plan.write()?.written.len(), 5);
    for (title, expected) in [
        ("Synthetic-A", b"\0first\xff".as_slice()),
        ("Synthetic-B", b"\xfeexact second\0".as_slice()),
    ] {
        let opus_name = format!("{title}.0");
        let zip_path = destination.path().join(format!("{title}.zip"));
        let mut zip = zip::ZipArchive::new(fs::File::open(&zip_path)?)?;
        let mut member = Vec::new();
        zip.by_name(&opus_name)?.read_to_end(&mut member)?;
        assert_eq!(member, expected);
        assert_eq!(fs::read(destination.path().join(&opus_name))?, expected);
        assert_eq!(fs::read(source.path().join(&opus_name))?, expected);
        let reopened = super::archive::reopen(&zip_path, &profile)?;
        assert_eq!(reopened.title, title);
        assert_eq!(reopened.address, "");
    }
    Ok(())
}

#[test]
fn opus_setting_defaults_off_and_keeps_chosen_folder() -> Result<()> {
    let original: super::store::Settings =
        serde_json::from_str(r#"{"site":"fixture-east","codes":["990","991"],"folder":null}"#)?;
    assert!(!original.attach_opus && original.opus_folder.is_none());
    assert!(!original.webcam_enabled);
    #[cfg(windows)]
    assert!(original.webcam_camera.is_none());
    let folder = tempfile::tempdir()?;
    let mut chosen = original.clone();
    chosen.opus_folder = Some(folder.path().to_owned());
    let mut persisted = original.merge_edits(&original, &chosen);
    persisted.attach_opus = true;
    let mut toggled = persisted.clone();
    toggled.attach_opus = false;
    let merged = persisted.merge_edits(&persisted, &toggled);
    let loaded: super::store::Settings = serde_json::from_slice(&serde_json::to_vec(&merged)?)?;
    assert!(!loaded.attach_opus);
    assert_eq!(loaded.opus_folder.as_deref(), Some(folder.path()));
    Ok(())
}

#[test]
#[ignore = "writes a synthetic local WordPress interop ZIP to an explicit QA directory"]
fn stage_opus_wordpress_qa() -> Result<()> {
    let input = std::env::var_os("GYDT_OPUS_QA_INPUT").ok_or("Set the synthetic QA input ZIP")?;
    let root = std::path::PathBuf::from(
        std::env::var_os("GYDT_OPUS_QA_DIR").ok_or("Set the synthetic QA folder")?,
    );
    let source = root.join("source");
    let destination = root.join("exports");
    fs::create_dir(&source)?;
    fs::create_dir(&destination)?;
    let profile = Profile::embedded()?;
    let mut draft = super::archive::reopen(std::path::Path::new(&input), &profile)?;
    draft.title = "3-993-2026-11-26-897".into();
    fs::write(
        source.join(format!("{}.0", draft.title)),
        b"\0Synthetic OPUS QA binary\xff",
    )?;
    let selection = super::export::Selection {
        tabs: vec![0],
        wordpress_only: true,
        confirmed: false,
    };
    let prepared = super::export::prepare(
        &selection,
        &mut [draft],
        &destination,
        &[],
        &profile,
        Some(&source),
    )?;
    assert_eq!(prepared.plan.write()?.written.len(), 2);
    Ok(())
}

#[test]
fn old_folder_requires_same_day_explicit_selection() -> Result<()> {
    let profile = fixture_profile()?;
    let data = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let store = super::store::Store::open_at(data.path().to_owned())?;
    let mut original = store.book(&profile)?;
    let yesterday = "2026-11-26".parse()?;
    let today = "2026-11-27".parse()?;
    let tomorrow = "2026-11-28".parse()?;
    original.settings.folder = Some(output.path().to_owned());
    store.save_book(&original)?;
    let mut settings = store.book(&profile)?.settings;
    assert!(settings.today_folder(today).is_none());
    assert!(!settings.select_folder(output.path().to_owned(), yesterday, today));
    assert!(settings.today_folder(today).is_none());
    assert!(settings.select_folder(output.path().to_owned(), today, today));
    original.settings = settings;
    store.save_book(&original)?;
    let saved = store.book(&profile)?;
    assert_eq!(saved.settings.today_folder(today), Some(output.path()));
    assert!(saved.settings.today_folder(tomorrow).is_none());
    assert_eq!(fs::read_dir(output.path())?.count(), 0);
    output.close()?;
    assert!(saved.settings.today_folder(today).is_none());
    Ok(())
}

#[test]
fn linked_visit_contact_follows_actual_source_but_never_an_archive() -> Result<()> {
    use super::{TabOrigin, analysis_day, export::Selection, ui};
    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    let today = analysis_day()?;
    let prefix = format!("3-990-{today}-");
    let mut first = make_sample()?;
    first.title = format!("{prefix}002");
    first.analysis_date = today.to_string();
    first.collection_date = today.to_string();
    first.address = "primary@example.invalid".into();
    let first_id = first.id;
    app.tabs.clear();
    app.push_tab(&ctx, first, TabOrigin::Recorded);
    app.create_tab(&ctx, true)?;
    let second = app.active.ok_or("Missing linked tab")?;
    app.create_tab(&ctx, true)?;
    let third = app.active.ok_or("Missing third tab")?;
    for index in [second, third] {
        let source = app.contact_source_index(index).ok_or("Missing source")?;
        let tab = app.tabs.get(source).ok_or("Missing source tab")?;
        assert_eq!(tab.draft.id, first_id);
        assert_eq!(ui::short_tab(&tab.draft), "Sample 002");
        assert_eq!(
            app.tabs
                .get(index)
                .ok_or("Missing linked tab")?
                .draft
                .address,
            ""
        );
    }
    let selection = Selection {
        tabs: vec![app.tabs.get(third).ok_or("Missing third tab")?.token],
        wordpress_only: false,
        confirmed: false,
    };
    let primary = app.tabs.first_mut().ok_or("Missing primary")?;
    primary.draft.title = format!("{prefix}007");
    primary.draft.address = "+1 (604) 555-0110".into();
    primary.draft.channel = Channel::Text;
    let resolved = app.export_drafts(&selection)?;
    assert_eq!(
        resolved.first().ok_or("Missing export draft")?.address,
        "+1 (604) 555-0110"
    );
    let primary = app.tabs.first_mut().ok_or("Missing primary")?;
    assert_eq!(ui::short_tab(&primary.draft), "Sample 007");
    primary.draft.visit = "88".into();
    assert!(app.export_drafts(&selection).is_err());
    primary_for_archive(&mut app, first_id)?;
    assert!(app.export_drafts(&selection).is_err());
    let wordpress = Selection {
        wordpress_only: true,
        ..selection
    };
    assert_eq!(
        app.export_drafts(&wordpress)?
            .first()
            .ok_or("Missing sample")?
            .address,
        ""
    );
    Ok(())
}

fn primary_for_archive(app: &mut super::App, id: uuid::Uuid) -> Result<()> {
    let primary = app.tabs.first_mut().ok_or("Missing primary")?;
    assert_eq!(primary.draft.id, id);
    primary.draft.visit = "7".into();
    primary.origin = super::TabOrigin::Archive;
    Ok(())
}

#[test]
fn a_different_profile_zip_still_reserves_its_number() -> Result<()> {
    let profile = fixture_profile()?;
    let other = tempfile::tempdir()?;
    write_fixture_profile(other.path(), ["other-east", "other-west"])?;
    let other_profile = Profile::load(other.path())?;
    let output = tempfile::tempdir()?;
    let title = "3-901-2026-11-22-001";
    let mut original = make_sample()?;
    title.clone_into(&mut original.title);
    export_sample(&original, &other_profile, output.path())?;
    let zip = output.path().join(format!("{title}.zip"));
    assert!(reopen(&zip, &profile).is_err());

    let mut replacement = make_sample()?;
    title.clone_into(&mut replacement.title);
    let before = fs::read(&zip)?;
    let selection = super::export::Selection {
        tabs: vec![0],
        wordpress_only: true,
        confirmed: true,
    };
    let Err(error) = super::export::prepare(
        &selection,
        &mut [replacement],
        output.path(),
        &[],
        &profile,
        None,
    ) else {
        return Err("A different sample was allowed to overwrite the numbered ZIP.".into());
    };
    assert!(error.to_string().contains("has already been used"));
    assert_eq!(fs::read(&zip)?, before);
    Ok(())
}

#[test]
fn startup_reminder_does_not_follow_an_archive_or_an_attached_photo() -> Result<()> {
    use super::Dialog;

    let records = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(records.path())?;
    assert!(matches!(app.dialog, Some(Dialog::ExportFolder)));
    assert!(app.deferred_photo_prompt.is_some());
    let mut sample = make_sample()?;
    "Synthetic-Reopen".clone_into(&mut sample.title);
    export_sample(&sample, &app.profile, output.path())?;
    app.open_zip(&ctx, &output.path().join("Synthetic-Reopen.zip"));
    assert!(matches!(
        app.tabs.last().map(|tab| tab.origin),
        Some(super::TabOrigin::Archive)
    ));
    app.tick_daily_folder(&ctx);
    assert!(app.dialog.is_none());

    let records = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(records.path())?;
    let photo_path = output.path().join("synthetic.jpg");
    fs::write(
        &photo_path,
        sample.photo.ok_or("Missing synthetic photo")?.bytes()?,
    )?;
    app.add_photo(&ctx, &photo_path, false);
    app.tick_daily_folder(&ctx);
    assert!(app.dialog.is_none());
    assert!(
        app.tabs
            .first()
            .is_some_and(|tab| tab.draft.photo.is_some())
    );
    Ok(())
}

#[test]
fn opus_snapshot_rejects_a_replaced_source_directory() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let output = root.path().join("output");
    fs::create_dir(&source)?;
    fs::create_dir(&output)?;
    let selected = gydt_results_compiler::OpusSource::new(&source, &output)?;
    fs::rename(&source, root.path().join("original"))?;
    fs::create_dir(&source)?;
    fs::write(source.join("Synthetic-A.0"), b"unrelated spectrum")?;
    let mut sample = make_sample()?;
    "Synthetic-A".clone_into(&mut sample.title);
    let mut input = [sample.to_sample()?];
    let result = gydt_results_compiler::prepare_profile_export_with_opus(
        &mut input,
        &fixture_profile()?,
        ExportRequest {
            contact: gydt_results_compiler::Contact::WordPressOnly,
            exported_at: Timestamp::now(),
            history: &[],
            confirmed_reexports: &[],
        },
        Some(&selected),
    );
    let Err(error) = result else {
        return Err("Files from a replaced OPUS directory were accepted.".into());
    };
    assert!(error.to_string().contains("OPUS folder changed"));
    assert!(fs::read_dir(output)?.next().is_none());
    Ok(())
}

#[test]
fn closing_untouched_and_reopened_tabs_does_not_prompt() -> Result<()> {
    let root = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    app.dialog = None;
    app.close_tab(&ctx, 0);
    assert!(app.tabs.is_empty());
    assert!(app.active.is_none());
    assert!(app.dialog.is_none());

    let mut sample = make_sample()?;
    "Synthetic-Reopen".clone_into(&mut sample.title);
    export_sample(&sample, &app.profile, output.path())?;
    let path = output.path().join("Synthetic-Reopen.zip");
    app.open_zip(&ctx, &path);
    app.open_zip(&ctx, &path);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(
        app.tabs.first().map(|tab| tab.draft.id),
        app.tabs.last().map(|tab| tab.draft.id)
    );
    app.close_tab(&ctx, 1);
    assert!(app.dialog.is_none());
    let archived = app.tabs.first_mut().ok_or("Missing reopened tab")?;
    "Edited after reopening".clone_into(&mut archived.draft.notes);
    archived.edit();
    app.close_tab(&ctx, 0);
    assert!(
        matches!(app.dialog, Some(super::Dialog::DiscardTab(ref request)) if request.warning == super::CloseWarning::Changed)
    );
    Ok(())
}

fn app_frame(
    ctx: &eframe::egui::Context,
    app: &mut super::App,
    events: Vec<eframe::egui::Event>,
) -> Result<eframe::egui::accesskit::TreeUpdate> {
    use eframe::egui::{Pos2, RawInput, Rect, vec2};
    let output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1136.0, 823.0))),
            events,
            ..RawInput::default()
        },
        |ui| app.show(ui),
    );
    Ok(output
        .platform_output
        .accesskit_update
        .ok_or("Missing accessibility tree")?)
}

fn button(
    tree: &eframe::egui::accesskit::TreeUpdate,
    label: &str,
) -> Result<(eframe::egui::accesskit::NodeId, eframe::egui::Pos2)> {
    use eframe::egui::{accesskit::Role, pos2};
    let (id, node) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
        .ok_or_else(|| format!("Missing button {label}"))?;
    let rect = node.bounds().ok_or("Missing button bounds")?;
    // AccessKit exposes these eGUI point coordinates as f64.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "The bounded test viewport uses egui f32 points"
    )]
    let point = pos2(
        rect.min_x().midpoint(rect.max_x()) as f32,
        rect.min_y().midpoint(rect.max_y()) as f32,
    );
    Ok((*id, point))
}

fn click(
    ctx: &eframe::egui::Context,
    app: &mut super::App,
    point: eframe::egui::Pos2,
) -> Result<eframe::egui::accesskit::TreeUpdate> {
    use eframe::egui::{Event, Modifiers, PointerButton};
    let _ = app_frame(ctx, app, vec![Event::PointerMoved(point)])?;
    for pressed in [true, false] {
        let tree = app_frame(
            ctx,
            app,
            vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }],
        )?;
        if !pressed {
            return Ok(tree);
        }
    }
    Err("The pointer was not released".into())
}

fn press(key: eframe::egui::Key) -> Vec<eframe::egui::Event> {
    use eframe::egui::{Event, Modifiers};
    [true, false]
        .into_iter()
        .map(|pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        })
        .collect()
}

#[test]
fn tab_close_pointer_and_keyboard_keep_edits_and_discard_only_the_named_tab() -> Result<()> {
    use super::{CloseWarning, Dialog, TabOrigin};
    use eframe::egui::{
        Event, Key,
        accesskit::{Action, ActionRequest, TreeId},
    };

    let root = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    super::ui::configure(&ctx);
    ctx.enable_accesskit();
    app.dialog = None;
    let other = app.active_token().ok_or("Missing untouched tab")?;
    let mut sample = make_sample()?;
    "Synthetic-Dirty".clone_into(&mut sample.title);
    let saved_notes = sample.notes.clone();
    app.push_tab(&ctx, sample, TabOrigin::Recorded);
    let dirty = app.active_token().ok_or("Missing edited tab")?;
    app.select_tab(&ctx, 0);

    let _ = app_frame(&ctx, &mut app, Vec::new())?;
    let tree = app_frame(&ctx, &mut app, Vec::new())?;
    let (_, close) = button(&tree, "Close Synthetic-Dirty")?;
    let tree = click(&ctx, &mut app, close)?;
    assert!(
        matches!(app.dialog, Some(Dialog::DiscardTab(ref req)) if req.token == dirty && req.warning == CloseWarning::NeverExported)
    );
    let (keep_id, _) = button(&tree, "Keep Editing")?;
    assert_eq!(tree.focus, keep_id);
    let _ = app_frame(&ctx, &mut app, press(Key::Escape))?;
    assert!(app.dialog.is_none());
    assert_eq!(app.active_token(), Some(other));
    assert_eq!(
        app.tabs.last().ok_or("Missing kept tab")?.draft.notes,
        saved_notes
    );

    let tree = app_frame(&ctx, &mut app, Vec::new())?;
    let (close_id, _) = button(&tree, "Close Synthetic-Dirty")?;
    let _ = app_frame(
        &ctx,
        &mut app,
        vec![Event::AccessKitActionRequest(ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: close_id,
            data: None,
        })],
    )?;
    let tree = app_frame(&ctx, &mut app, press(Key::Enter))?;
    assert!(matches!(app.dialog, Some(Dialog::DiscardTab(_))));
    assert_eq!(tree.focus, button(&tree, "Keep Editing")?.0);
    let _ = app_frame(&ctx, &mut app, press(Key::Enter))?;
    assert!(app.dialog.is_none());
    assert_eq!(app.active_token(), Some(other));

    let tree = app_frame(&ctx, &mut app, Vec::new())?;
    let (_, close) = button(&tree, "Close Synthetic-Dirty")?;
    let tree = click(&ctx, &mut app, close)?;
    let (_, discard) = button(&tree, "Discard and Close")?;
    app.tabs.swap(0, 1);
    app.select_tab(&ctx, 1);
    let _ = click(&ctx, &mut app, discard)?;
    assert!(app.dialog.is_none());
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_token(), Some(other));
    Ok(())
}

fn worker_event(app: &mut super::App) -> Result<super::export::Event> {
    use std::time::{Duration, Instant};
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = app.worker.poll() {
            return Ok(event);
        }
        if Instant::now() >= until {
            return Err("No export worker response".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn set_qa_destination(app: &mut super::App, path: &Path) -> Result<()> {
    let day = super::analysis_day()?;
    if !app.book.settings.select_folder(path.to_owned(), day, day) {
        return Err("Missing synthetic output directory".into());
    }
    app.dialog = None;
    Ok(())
}

#[test]
fn tab_close_stays_guarded_after_failed_and_cancelled_exports_then_clears_on_success() -> Result<()>
{
    use super::{CloseWarning, Dialog, TabOrigin};
    let root = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    super::ui::configure(&ctx);
    ctx.enable_accesskit();
    set_qa_destination(&mut app, output.path())?;
    let mut sample = make_sample()?;
    "Synthetic-Export".clone_into(&mut sample.title);
    app.push_tab(&ctx, sample, TabOrigin::Recorded);
    app.book.settings.attach_opus = true;
    app.book.settings.opus_folder = Some(root.path().join("unavailable"));
    app.export_active(&ctx);
    assert!(app.worker.busy());
    app.close_tab(&ctx, 1);
    assert_eq!(app.tabs.len(), 2);
    let event = worker_event(&mut app)?;
    app.handle_export(&ctx, event);
    assert!(matches!(app.dialog, Some(Dialog::Notice(..))));
    assert!(fs::read_dir(output.path())?.next().is_none());
    app.dialog = None;
    app.close_tab(&ctx, 1);
    assert!(
        matches!(app.dialog, Some(Dialog::DiscardTab(ref req)) if req.warning == CloseWarning::NeverExported)
    );

    app.dialog = None;
    app.book.settings.attach_opus = false;
    let destination = output.path().join("Synthetic-Export.zip");
    fs::write(&destination, b"previous output")?;
    app.export_active(&ctx);
    let event = worker_event(&mut app)?;
    app.handle_export(&ctx, event);
    assert!(matches!(app.dialog, Some(Dialog::Overwrite(..))));
    let _ = app_frame(&ctx, &mut app, Vec::new())?;
    let _ = app_frame(&ctx, &mut app, press(eframe::egui::Key::Escape))?;
    assert!(app.dialog.is_none());
    assert_eq!(fs::read(&destination)?, b"previous output");
    assert_eq!(
        app.tabs.last().and_then(super::Tab::close_warning),
        Some(CloseWarning::NeverExported)
    );

    fs::remove_file(&destination)?;
    app.export_active(&ctx);
    for _ in 0..2 {
        let event = worker_event(&mut app)?;
        app.handle_export(&ctx, event);
    }
    assert!(app.completion.is_some());
    assert!(destination.is_file());
    let tab = app.tabs.last_mut().ok_or("Missing exported tab")?;
    assert_eq!(tab.close_warning(), None);
    "An edit after the write".clone_into(&mut tab.draft.notes);
    tab.edit();
    app.close_tab(&ctx, 1);
    assert!(
        matches!(app.dialog, Some(Dialog::DiscardTab(ref req)) if req.warning == CloseWarning::Changed)
    );
    let _ = app_frame(&ctx, &mut app, Vec::new())?;
    let _ = app_frame(&ctx, &mut app, press(eframe::egui::Key::Escape))?;
    assert_eq!(
        app.tabs.last().ok_or("Missing kept tab")?.draft.notes,
        "An edit after the write"
    );
    app.open_zip(&ctx, &destination);
    app.close_tab(&ctx, 2);
    assert!(app.dialog.is_none());
    Ok(())
}

#[test]
fn worker_success_marks_only_the_captured_revision_of_the_stable_tab() -> Result<()> {
    use super::{CloseWarning, TabOrigin};
    let root = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let (ctx, mut app) = open_app(root.path())?;
    set_qa_destination(&mut app, output.path())?;
    let other = app.active_token().ok_or("Missing untouched tab")?;
    let mut draft = make_sample()?;
    "Synthetic-Captured".clone_into(&mut draft.title);
    draft.code_word.clear();
    let notes_at_export = draft.notes.clone();
    app.push_tab(&ctx, draft, TabOrigin::Recorded);
    let exported = app.active_token().ok_or("Missing captured tab")?;
    app.export_active(&ctx);
    let tab = app.tabs.last_mut().ok_or("Missing captured tab")?;
    "An edit while exporting".clone_into(&mut tab.draft.notes);
    "UPDATED01".clone_into(&mut tab.draft.code_word);
    tab.edit();
    app.tabs.swap(0, 1);
    app.select_tab(&ctx, 1);
    for _ in 0..2 {
        let event = worker_event(&mut app)?;
        app.handle_export(&ctx, event);
    }
    assert!(app.completion.is_some());
    assert_eq!(app.active_token(), Some(other));
    let tab = app.tabs.first().ok_or("Missing moved tab")?;
    assert_eq!(tab.token, exported);
    assert_eq!(tab.close_warning(), Some(CloseWarning::Changed));
    assert_eq!(tab.draft.notes, "An edit while exporting");
    assert_eq!(tab.draft.code_word, "UPDATED01");
    assert_eq!(app.tabs.last().and_then(super::Tab::close_warning), None);
    let archived = reopen(&output.path().join("Synthetic-Captured.zip"), &app.profile)?;
    assert_eq!(archived.notes, notes_at_export);
    assert_ne!(archived.code_word, "UPDATED01");
    Ok(())
}
