use std::{
    collections::BTreeSet,
    error::Error,
    fmt::Write as _,
    fs::{self, File},
    io::Read as _,
    path::Path,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gydt_results_compiler::{
    AdverseEvents, Catalogue, CollectionSite, Consumption, Contact, DEFAULT_SITE_CODE,
    DetectedResult, ExportError, ExportRequest, Percentage, Photo, PreparedExport, Sample,
    StripResult, Strips, Substance, WriteError, live_textures, new_sample_id, prepare_export,
};
use image::{Rgb, RgbImage, codecs::jpeg::JpegEncoder};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use zip::{System, ZipArchive};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const STAMP: &str = "2026-11-23T13:41:01Z";
const FIRST: &str = "3-901-2026-11-21-901";
const SECOND: &str = "3-901-2026-11-21-902";
const EAST: &str = "3-901-2026-11-21-903";
const FIRST_NOTE: &str = "Alpha & synthetic note";
const SECOND_NOTE: &str = "Beta synthetic note";
const STAFF_CODE: &str = "CEDAR  0002";
const ROTATE_RIGHT_EXIF: &[u8] =
    b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";

fn request(contact: Contact<'_>) -> TestResult<ExportRequest<'_>> {
    Ok(ExportRequest {
        contact,
        exported_at: STAMP.parse()?,
        history: &[],
        confirmed_reexports: &[],
    })
}

fn sample(title: &str, site: CollectionSite) -> TestResult<Sample> {
    Ok(Sample {
        id: new_sample_id()?,
        submission_number: title.into(),
        visit: 91,
        site,
        analysis_date: "2026-11-21".parse()?,
        collection_date: "2026-11-19".parse()?,
        expected_substance: Substance::Other,
        sold_as_other: Some("synthetic expected label".into()),
        results: vec![
            DetectedResult {
                substance: Substance::Catalogue(910_001),
                percentage: Some(Percentage { low: 12, high: 18 }),
                other: None,
            },
            DetectedResult {
                substance: Substance::Unknown,
                percentage: None,
                other: None,
            },
        ],
        colour: "other".into(),
        colour_other: Some("synthetic colour".into()),
        texture: "Pressed Pill".into(),
        texture_other: None,
        pill_shape: Some("other".into()),
        pill_shape_other: Some("synthetic shape".into()),
        consumption: Consumption::Post,
        adverse_events: AdverseEvents::Yes,
        adverse_events_other: Some("synthetic effect".into()),
        strips: Strips {
            fentanyl: StripResult::Positive,
            benzodiazepines: StripResult::NotUsed,
            medetomidine: StripResult::Invalid,
            lysergamide: StripResult::Negative,
        },
        code_word: String::new(),
        notes: Some("  Alpha &\r\nsynthetic note  ".into()),
        photo: None,
    })
}

fn attach_synthetic_photo(sample: &mut Sample, dir: &Path) -> TestResult {
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 83).encode_image(&RgbImage::from_pixel(
        3,
        2,
        Rgb([70, 120, 180]),
    ))?;
    let path = dir.join("input.JpEg");
    fs::write(&path, &jpeg)?;
    sample.photo = Some(Photo::from_path(&path)?);
    assert_eq!(fs::read(&path)?, jpeg);
    Ok(())
}

#[test]
fn progressive_photo_is_packaged_byte_for_byte() -> TestResult {
    use gamut_core::{Dimensions, EncodeImage as _, ImageRef, Rgb8};

    let inputs = tempdir()?;
    let output = tempdir()?;
    let path = inputs.path().join("progressive.jpg");
    let dimensions = Dimensions::new(3, 2)?;
    let source_pixels = [
        70, 120, 180, 237, 100, 45, 220, 37, 67, 85, 200, 45, 19, 47, 162, 108, 172, 200,
    ];
    let pixels = ImageRef::<Rgb8>::new(&source_pixels, dimensions)?;
    let mut jpeg = Vec::new();
    gamut_jpeg::JpegEncoder::new()
        .with_progressive(true)
        .encode_image(pixels, &mut jpeg)?;
    assert_eq!(
        gamut_jpeg::info(&jpeg)?.process,
        gamut_jpeg::JpegProcess::Progressive
    );
    fs::write(&path, &jpeg)?;

    let catalogue = Catalogue::bundled()?;
    let mut record = sample(EAST, CollectionSite::ExampleEast)?;
    record.photo = Some(Photo::from_path(&path)?);
    let export = prepare_export(
        std::slice::from_mut(&mut record),
        &catalogue,
        request(Contact::WordPressOnly)?,
    )?;
    export.write(output.path(), None)?;
    verify_zip(output.path(), &record, Some(("jpg", &jpeg)))?;
    assert_eq!(fs::read(&path)?, jpeg);

    let missing_end = jpeg
        .strip_suffix(&[0xff, 0xd9])
        .ok_or("missing progressive EOI")?;
    fs::write(&path, missing_end)?;
    assert!(Photo::from_path(&path).is_err());
    Ok(())
}

#[test]
fn arithmetic_coded_photo_is_packaged_byte_for_byte() -> TestResult {
    // Synthetic 8x6 grayscale JPEG made with sequential arithmetic coding.
    let jpeg = STANDARD.decode(concat!(
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsK",
        "CwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/yQALCAAGAAgBAREA/8wABgAQEAX/2gAI",
        "AQEAAD8A/wCz8oFoPv6isdM4NyFH0ArNYsDXzroyVkWfK1DG5cVEFFAwZPt5BxM1iNCRaf/Z"
    ))?;
    let inputs = tempdir()?;
    let output = tempdir()?;
    let path = inputs.path().join("arithmetic.jpg");
    fs::write(&path, &jpeg)?;
    let mut record = sample(EAST, CollectionSite::ExampleEast)?;
    record.photo = Some(Photo::from_path(&path)?);
    let export = prepare_export(
        std::slice::from_mut(&mut record),
        &Catalogue::bundled()?,
        request(Contact::WordPressOnly)?,
    )?;
    export.write(output.path(), None)?;
    verify_zip(output.path(), &record, Some(("jpg", &jpeg)))?;
    assert_eq!(fs::read(path)?, jpeg);
    Ok(())
}

#[test]
fn camera_metadata_is_removed_only_from_the_export_copy() -> TestResult {
    let inputs = tempdir()?;
    let output = tempdir()?;
    let path = inputs.path().join("input.jpg");
    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg).encode_image(&RgbImage::from_pixel(3, 2, Rgb([70, 120, 180])))?;
    let tail = jpeg.strip_prefix(&[0xff, 0xd8]).ok_or("missing SOI")?;
    // Standard synthetic sRGB ICC profile generated with Little CMS.
    let srgb = STANDARD.decode(concat!(
        "AAACTGxjbXMEQAAAbW50clJHQiBYWVogB+kAAQABAAAAAAAAYWNzcEFQUEwAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAPbWAAEAAAAA0y1sY21zAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAALZGVzYwAAAQgAAAA2Y3BydAAAAUAAAABMd3RwdAAAAYwAAAAUY2hh",
        "ZAAAAaAAAAAsclhZWgAAAcwAAAAUYlhZWgAAAeAAAAAUZ1hZWgAAAfQAAAAUclRSQwAAAggAAAAg",
        "Z1RSQwAAAggAAAAgYlRSQwAAAggAAAAgY2hybQAAAigAAAAkbWx1YwAAAAAAAAABAAAADGVuVVMA",
        "AAAaAAAAHABzAFIARwBCACAAYgB1AGkAbAB0AC0AaQBuAABtbHVjAAAAAAAAAAEAAAAMZW5VUwAA",
        "ADAAAAAcAE4AbwAgAGMAbwBwAHkAcgBpAGcAaAB0ACwAIAB1AHMAZQAgAGYAcgBlAGUAbAB5WFla",
        "IAAAAAAAAPbWAAEAAAAA0y1zZjMyAAAAAAABDEIAAAXe///zJQAAB5MAAP2Q///7of///aIAAAPc",
        "AADAblhZWiAAAAAAAABvoAAAOPUAAAOQWFlaIAAAAAAAACSfAAAPhAAAtsNYWVogAAAAAAAAYpcA",
        "ALeHAAAY2XBhcmEAAAAAAAMAAAACZmYAAPKnAAANWQAAE9AAAApbY2hybQAAAAAAAwAAAACj1wAA",
        "VHsAAEzNAACZmgAAJmYAAA9c",
    ))?;
    let mut camera = vec![0xff, 0xd8, 0xff, 0xe1, 0, 34];
    camera.extend_from_slice(b"Exif\0\0");
    camera.extend_from_slice(ROTATE_RIGHT_EXIF);
    camera.extend_from_slice(&[0xff, 0xe2]);
    let length = u16::try_from(srgb.len() + 16)?;
    camera.extend_from_slice(&length.to_be_bytes());
    camera.extend_from_slice(b"ICC_PROFILE\0\x01\x01");
    camera.extend_from_slice(&srgb);
    camera.extend_from_slice(&[0xff, 0xfe, 0, 5]);
    camera.extend_from_slice(b"QA!");
    camera.extend_from_slice(tail);
    fs::write(&path, &camera)?;

    let mut record = sample(EAST, CollectionSite::ExampleEast)?;
    record.photo = Some(Photo::from_path(&path)?);
    let export = prepare_export(
        std::slice::from_mut(&mut record),
        &Catalogue::bundled()?,
        request(Contact::WordPressOnly)?,
    )?;
    export.write(output.path(), None)?;
    verify_zip(output.path(), &record, Some(("jpg", &jpeg)))?;
    assert_eq!(fs::read(&path)?, camera);
    assert_eq!(Photo::from_path(&path)?.into_jpeg_bytes(), camera);
    Ok(())
}

#[test]
fn malformed_photo_metadata_and_trailing_data_are_rejected_without_rewriting() -> TestResult {
    let folder = tempdir()?;
    let path = folder.path().join("input.jpg");
    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg).encode_image(&RgbImage::from_pixel(3, 2, Rgb([70, 120, 180])))?;
    let tail = jpeg.strip_prefix(&[0xff, 0xd8]).ok_or("missing SOI")?;
    let mut malformed = vec![0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff];
    malformed.extend_from_slice(tail);
    jpeg.extend_from_slice(b"extra");

    for invalid in [malformed, jpeg, b"not a JPEG".to_vec()] {
        fs::write(&path, &invalid)?;
        assert!(Photo::from_path(&path).is_err());
        assert_eq!(fs::read(&path)?, invalid);
    }
    Ok(())
}

#[test]
fn bundled_vancouver_calendar_has_no_november_2026_rollback() -> TestResult {
    use jiff::{Timestamp, civil::DateTime, tz::TimeZone};

    let zone = TimeZone::get("America/Vancouver")?;
    for (local, utc) in [
        ("2026-11-01T00:00:00", "2026-11-01T07:00:00Z"),
        ("2026-11-01T01:59:59", "2026-11-01T08:59:59Z"),
        ("2026-11-01T02:00:00", "2026-11-01T09:00:00Z"),
        ("2026-11-02T00:00:00", "2026-11-02T07:00:00Z"),
        ("2026-11-19T00:00:00", "2026-11-19T07:00:00Z"),
    ] {
        let local = local.parse::<DateTime>()?;
        let expected = utc.parse::<Timestamp>()?;
        assert_eq!(
            zone.to_ambiguous_zoned(local).unambiguous()?.timestamp(),
            expected
        );
    }
    Ok(())
}

#[test]
fn photo_file_and_dimension_limits_still_apply() -> TestResult {
    let folder = tempdir()?;
    let path = folder.path().join("large.jpg");
    File::create(&path)?.set_len(4 * 1024 * 1024 + 1)?;
    assert!(matches!(
        Photo::from_path(&path),
        Err(ExportError::InvalidField {
            field: "photo",
            reason: "Choose a JPEG up to 4 MiB."
        })
    ));

    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg).encode_image(&RgbImage::from_pixel(
        4097,
        1,
        Rgb([70, 120, 180]),
    ))?;
    fs::write(&path, jpeg)?;
    assert!(matches!(
        Photo::from_path(&path),
        Err(ExportError::InvalidField {
            field: "photo",
            reason: "The JPEG exceeds the supported dimensions."
        })
    ));
    Ok(())
}

#[test]
fn incomplete_jpegs_are_rejected_without_replacing_the_source() -> TestResult {
    let folder = tempdir()?;
    let path = folder.path().join("synthetic.jpg");
    let pixels = RgbImage::from_fn(48, 32, |x, y| {
        Rgb([
            u8::try_from(x).unwrap_or_default(),
            u8::try_from(y).unwrap_or_default(),
            90,
        ])
    });
    let mut complete = Vec::new();
    JpegEncoder::new(&mut complete).encode_image(&pixels)?;
    let entropy = complete.strip_suffix(&[0xff, 0xd9]).ok_or("missing EOI")?;
    fs::write(&path, &complete)?;
    let _ = Photo::from_path(&path)?;

    fs::write(&path, entropy)?;
    assert!(Photo::from_path(&path).is_err());
    assert_eq!(fs::read(&path)?, entropy);

    let mut early_end = entropy
        .get(..entropy.len() - 20)
        .ok_or("short fixture")?
        .to_vec();
    early_end.extend_from_slice(&[0xff, 0xd9]);
    fs::write(&path, &early_end)?;
    assert!(Photo::from_path(&path).is_err());
    assert_eq!(fs::read(&path)?, early_end);
    Ok(())
}

fn read_member(archive: &mut ZipArchive<File>, name: &str) -> TestResult<Vec<u8>> {
    let mut data = Vec::new();
    archive.by_name(name)?.read_to_end(&mut data)?;
    Ok(data)
}

fn digest(value: &[u8]) -> String {
    let mut text = String::new();
    for byte in Sha256::digest(value) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn verify_zip(folder: &Path, sample: &Sample, photo: Option<(&str, &[u8])>) -> TestResult<Value> {
    let title = &sample.submission_number;
    let mut archive = ZipArchive::new(File::open(folder.join(format!("{title}.zip")))?)?;
    assert_eq!(archive.len(), 2 + usize::from(photo.is_some()));
    for index in 0..archive.len() {
        let member = archive.by_index(index)?;
        let name = member.name()?;
        assert_eq!(member.system(), System::Unix, "ZIP member {name}");
        assert_eq!(
            member.external_attributes(),
            0o100_600 << 16,
            "ZIP member {name}"
        );
    }
    let data = read_member(&mut archive, "test_data.json")?;
    let manifest: Value = serde_json::from_slice(&read_member(&mut archive, "manifest.json")?)?;
    let files = manifest.get("files").ok_or("missing files")?;
    assert_eq!(
        manifest
            .as_object()
            .ok_or("manifest is not an object")?
            .len(),
        5
    );
    assert_eq!(manifest.get("format_version"), Some(&Value::from(1)));
    assert_eq!(
        manifest.get("sample_id"),
        Some(&Value::from(sample.id.to_string()))
    );
    assert_eq!(manifest.get("exported_at"), Some(&Value::from(STAMP)));
    assert_eq!(
        manifest.get("catalogue_version"),
        Some(&Value::from(concat!(
            "sha256:",
            "501794d785f8719a385dda374e4a47d81792d19135f26e863f214d9d6c4b86f8"
        )))
    );
    verify_entry(files, "test_data.json", &data)?;
    if let Some((ext, source)) = photo {
        let name = format!("{title}.{ext}");
        let jpeg = read_member(&mut archive, &name)?;
        assert_eq!(jpeg, source);
        verify_entry(files, &name, source)?;
    }
    assert_eq!(
        files.as_object().ok_or("files is not an object")?.len(),
        archive.len() - 1
    );
    let payload: Value = serde_json::from_slice(&data)?;
    assert_eq!(
        payload.get("submission_number"),
        Some(&Value::from(title.as_str()))
    );
    assert_eq!(payload.get("visit"), Some(&Value::from(91)));
    assert_eq!(
        payload.get("analysis_date"),
        Some(&Value::from("2026-11-21"))
    );
    assert_eq!(
        payload.get("collection_date"),
        Some(&Value::from("2026-11-19"))
    );
    assert_eq!(
        payload.get("client"),
        Some(&Value::from(sample.code_word.clone()))
    );
    let keys: BTreeSet<_> = payload
        .as_object()
        .ok_or("payload is not an object")?
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = BTreeSet::from([
        "submission_number",
        "visit",
        "site",
        "analysis_date",
        "collection_date",
        "expected_substance",
        "sold_as_other",
        "results",
        "colour",
        "colour_other",
        "texture",
        "pre_post_consumption",
        "adverse_events",
        "advertse_events_yes_other",
        "fent_strip",
        "benzo",
        "medetomidine",
        "lysergamide_strip",
        "client",
        "notes",
    ]);
    if sample.pill_shape.is_some() {
        expected.extend(["pill_shape", "pill_shape_other"]);
    } else {
        expected.insert("texture_other");
    }
    assert_eq!(keys, expected);
    Ok(payload)
}

fn verify_entry(files: &Value, name: &str, bytes: &[u8]) -> TestResult {
    let entry = files
        .get(name)
        .and_then(Value::as_object)
        .ok_or("missing checksums")?;
    assert_eq!(entry.len(), 2);
    assert_eq!(entry.get("bytes"), Some(&Value::from(bytes.len())));
    assert_eq!(entry.get("sha256"), Some(&Value::from(digest(bytes))));
    Ok(())
}

fn pair(directory: &Path) -> TestResult<[Sample; 2]> {
    let mut first = sample(FIRST, CollectionSite::ExampleWest)?;
    attach_synthetic_photo(&mut first, directory)?;
    let mut second = sample(SECOND, CollectionSite::ExampleWest)?;
    second.texture = "other".into();
    second.texture_other = Some("synthetic texture".into());
    second.pill_shape = None;
    second.pill_shape_other = None;
    second.notes = Some(SECOND_NOTE.into());
    second.results = vec![DetectedResult {
        substance: Substance::Other,
        percentage: None,
        other: Some("synthetic detected label".into()),
    }];
    second.strips = Strips {
        fentanyl: StripResult::Negative,
        ..Strips::default()
    };
    second.code_word = STAFF_CODE.into();
    Ok([first, second])
}

fn verify_draft(body: &str, first_code: &str) -> TestResult {
    let (first, second) = body.split_once("Sample 2").ok_or("missing second sample")?;
    assert!(first.contains("Sample 1"));
    assert!(first.contains(first_code));
    assert!(first.contains("Synthetic compound A (12–18%), Unknown"));
    assert!(
        first.contains("Fentanyl: Positive, Medetomidine: Invalid, Lysergamide (LSD): Negative")
    );
    assert!(first.contains("Negative\nNotes: Alpha & synthetic note"));
    assert!(second.contains(STAFF_CODE));
    assert!(second.contains("Detected substances: Other (synthetic detected label)"));
    assert!(second.contains("Fentanyl: Negative\nNotes: Beta synthetic note"));
    assert!(!first.contains(SECOND_NOTE) && !second.contains(FIRST_NOTE));
    assert!(!first.contains("Fentanyl: Negative") && !second.contains("Fentanyl: Positive"));
    assert!(!body.contains(FIRST) && !body.contains(SECOND));
    assert!(!body.contains("Submission number:"));
    assert!(!body.contains("Benzodiazepines:") && !body.contains("Not Used"));
    assert!(!body.contains("{{") && !body.contains("----") && !body.contains("no evidence"));
    Ok(())
}

fn decoded_email(email: &str, subtype: &str) -> TestResult<String> {
    let part = email
        .split_once(&format!("Content-Type: text/{subtype}; charset=UTF-8\r\n"))
        .ok_or("missing MIME alternative")?
        .1;
    let encoded = part
        .split_once("\r\n\r\n")
        .ok_or("missing MIME part body")?
        .1;
    let encoded = encoded
        .split_once("\r\n--")
        .ok_or("missing MIME boundary")?
        .0;
    Ok(
        String::from_utf8(STANDARD.decode(encoded.lines().collect::<String>())?)?
            .replace("\r\n", "\n"),
    )
}

fn verify_approved_plain(body: &str) -> TestResult {
    const FOOTER: &str = "You also can find the test results on the webpage:";
    let approved = include_str!("../templates/results-email.eml").replace("\r\n", "\n");
    let (_, approved) = approved
        .split_once(FOOTER)
        .ok_or("missing approved text footer")?;
    let (approved, _) = approved
        .split_once("\n--")
        .ok_or("missing approved MIME end")?;
    let (_, footer) = body.split_once(FOOTER).ok_or("missing text footer")?;
    assert_eq!(footer.trim_end(), approved.trim_end());
    Ok(())
}

fn verify_approved_html(html: &str) -> TestResult {
    let raw = include_str!("../templates/results-email.eml");
    let (_, part) = raw
        .split_once("Content-Transfer-Encoding: quoted-printable")
        .ok_or("missing approved HTML")?;
    let (_, encoded) = part
        .split_once("\r\n\r\n")
        .ok_or("missing approved MIME text")?;
    let (encoded, _) = encoded
        .split_once("\r\n--")
        .ok_or("missing approved MIME end")?;
    let approved = String::from_utf8(quoted_printable::decode(
        encoded,
        quoted_printable::ParseMode::Strict,
    )?)?
    .replace("\r\n", "\n");
    let (_, expected_footer) = approved
        .split_once("</ol>")
        .ok_or("missing approved footer")?;
    let (samples, footer) = html.split_once("</ol>").ok_or("missing HTML sample list")?;
    assert_eq!(footer, expected_footer);
    assert_eq!(samples.matches("<li>").count(), 2);
    assert!(samples.starts_with("<div dir=\"ltr\">"));
    assert!(samples.contains("<li><font size=\"2\" face=\"arial, sans-serif\">"));
    assert!(!samples.contains("submission number ----"));
    assert!(footer.contains("Synthetic example only."));
    Ok(())
}

#[test]
fn two_samples_export_ordered_zips_and_separate_contact_drafts() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    assert_eq!(catalogue.records().len(), 3);
    assert_eq!(
        catalogue
            .records()
            .filter(|record| record.label == "Synthetic compound B")
            .count(),
        2
    );
    assert_eq!(
        catalogue
            .records()
            .map(|record| record.categories.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(DEFAULT_SITE_CODE, "901");
    let inputs = tempdir()?;
    let [mut first, mut second] = pair(inputs.path())?;
    let source_path = inputs.path().join("input.JpEg");
    let original = fs::read(&source_path)?;
    let email_dir = tempdir()?;
    let address = "synthetic@example.invalid";
    let export = prepare_export(
        std::slice::from_mut(&mut first),
        &catalogue,
        request(Contact::WordPressOnly)?,
    )?;
    assert_eq!(export.filenames().count(), 1);
    let generated = first.code_word.clone();
    let (word, digits) = generated
        .split_at_checked(generated.len().checked_sub(4).ok_or("code too short")?)
        .ok_or("invalid code stem")?;
    assert!(!word.is_empty() && word.bytes().all(|byte| byte.is_ascii_uppercase()));
    assert!(digits.bytes().all(|digit| digit.is_ascii_digit()));
    let mut samples = [first, second];
    let email_plan = prepare_export(&mut samples, &catalogue, request(Contact::Email(address))?)?;
    email_plan.write(email_dir.path(), None)?;
    assert_eq!(email_plan.filenames().count(), 3);
    [first, second] = samples;
    assert_eq!(first.code_word, generated);
    let a = verify_zip(email_dir.path(), &first, Some(("JpEg", &original)))?;
    assert_eq!(fs::read(&source_path)?, original);
    let b = verify_zip(email_dir.path(), &second, None)?;
    assert_eq!(a.get("site"), Some(&Value::from("fixture-west")));
    assert_eq!(a.get("fent_strip"), Some(&Value::from("Positive")));
    assert_eq!(a.get("lysergamide_strip"), Some(&Value::from("Negative")));
    assert_eq!(a.get("medetomidine"), Some(&Value::from("Invalid")));
    assert_eq!(a.get("benzo"), Some(&Value::from("Not Used")));
    assert_eq!(b.get("fent_strip"), Some(&Value::from("Negative")));
    assert_eq!(a.get("notes"), Some(&Value::from(FIRST_NOTE)));
    assert_eq!(b.get("notes"), Some(&Value::from(SECOND_NOTE)));
    assert_eq!(
        a.get("results"),
        Some(&serde_json::json!([
            {"substance":"910001","percentage":"12-18%"}, {"substance":"unknown","percentage":"blank"}
        ]))
    );
    assert_eq!(
        b.get("results"),
        Some(&serde_json::json!([
            {"substance":"other","percentage":"blank","other":"synthetic detected label"}
        ]))
    );
    assert!(!a.to_string().contains(address) && !b.to_string().contains(address));

    let email = fs::read_to_string(email_dir.path().join(format!("{FIRST}.eml")))?;
    assert!(
        email.contains("To: synthetic@example.invalid\r\nSubject: Synthetic sample report\r\n")
    );
    assert_eq!(email.matches("Content-Type: text/").count(), 2);
    assert!(!email.contains(FIRST) && !email.contains(SECOND));
    assert!(!email.contains("Content-Disposition: attachment") && !email.contains("image/"));
    let plain = decoded_email(&email, "plain")?;
    verify_draft(&plain, &generated)?;
    verify_approved_plain(&plain)?;
    let html = decoded_email(&email, "html")?;
    assert!(html.contains("Alpha &amp; synthetic note"));
    assert!(html.contains("Code word: CEDAR &nbsp;0002<br>"));
    verify_approved_html(&html)?;
    verify_draft(&visible_html_text(&html), &generated)?;

    let sms_dir = tempdir()?;
    let mut samples = [first, second];
    let sms = prepare_export(
        &mut samples,
        &catalogue,
        request(Contact::Text("6045550100"))?,
    )?;
    sms.write(sms_dir.path(), None)?;
    let [first, second] = &samples;
    verify_zip(sms_dir.path(), first, Some(("JpEg", &original)))?;
    verify_zip(sms_dir.path(), second, None)?;
    let sms = fs::read_to_string(sms_dir.path().join(format!("{FIRST}.txt")))?;
    let sms_body = sms
        .strip_prefix("To: 6045550100\n\n")
        .ok_or("Missing SMS header")?;
    verify_draft(sms_body, &generated)?;
    assert!(!sms_body.contains("6045550100"));
    verify_no_contact_in_zip(sms_dir.path())?;
    Ok(())
}

fn visible_html_text(html: &str) -> String {
    html.replace("<br>", "\n")
        .replace("</p><p>", "\n\n")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn verify_no_contact_in_zip(folder: &Path) -> TestResult {
    for sample in [FIRST, SECOND] {
        let mut archive = ZipArchive::new(fs::File::open(folder.join(format!("{sample}.zip")))?)?;
        for name in ["manifest.json", "test_data.json"] {
            let mut data = String::new();
            archive.by_name(name)?.read_to_string(&mut data)?;
            assert!(!data.contains("6045550100"));
        }
    }
    Ok(())
}

#[test]
fn single_email_and_sms_omit_only_the_report_heading() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let inputs = tempdir()?;
    let [mut first, _] = pair(inputs.path())?;
    let address = "synthetic@example.invalid";
    let one = tempdir()?;
    let phone = "+1 (604) 555-0100";
    let text = prepare_export(
        std::slice::from_mut(&mut first),
        &catalogue,
        request(Contact::Text(phone))?,
    )?;
    text.write(one.path(), None)?;
    let text = fs::read_to_string(one.path().join(format!("{FIRST}.txt")))?;
    let expected_header = format!("To: {phone}\n\nHello.");
    assert!(text.starts_with(&expected_header));
    assert!(!text.contains("Sample 1") && !text.contains(FIRST));
    let email_folder = tempdir()?;
    let single_email = prepare_export(
        std::slice::from_mut(&mut first),
        &catalogue,
        request(Contact::Email(address))?,
    )?;
    single_email.write(email_folder.path(), None)?;
    let eml = fs::read_to_string(email_folder.path().join(format!("{FIRST}.eml")))?;
    for part in ["plain", "html"] {
        let body = decoded_email(&eml, part)?;
        assert!(!body.contains("Sample 1") && !body.contains(FIRST) && !body.contains("<ol>"));
        assert!(body.contains("Code word: "));
    }
    assert!(
        prepare_export(
            std::slice::from_mut(&mut first),
            &catalogue,
            request(Contact::Text("6045550100\nTo: 6045550199"))?
        )
        .is_err()
    );

    Ok(())
}

#[test]
fn contact_drafts_require_one_testing_site_analysis_date_and_visit() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let directory = tempdir()?;
    for contact in [
        Contact::Email("synthetic@example.invalid"),
        Contact::Text("6045550100"),
    ] {
        for difference in ["visit", "site", "analysis date"] {
            let first = sample(FIRST, CollectionSite::ExampleWest)?;
            let mut second = sample(SECOND, CollectionSite::ExampleWest)?;
            match difference {
                "visit" => second.visit = first.visit + 1,
                "site" => second.site = CollectionSite::ExampleEast,
                _ => second.analysis_date = "2026-11-22".parse()?,
            }
            let mut samples = [first, second];
            let result = prepare_export(&mut samples, &catalogue, request(contact)?);
            assert!(
                matches!(result, Err(ExportError::MixedContactVisit)),
                "{difference}"
            );
            assert!(samples.iter().all(|sample| sample.code_word.is_empty()));
            assert_eq!(fs::read_dir(directory.path())?.count(), 0);
        }
    }

    let mut first = sample(FIRST, CollectionSite::ExampleWest)?;
    let second = sample(SECOND, CollectionSite::ExampleWest)?;
    first.collection_date = "2026-11-18".parse()?;
    let mut samples = [first, second];
    let plan = prepare_export(
        &mut samples,
        &catalogue,
        request(Contact::Text("6045550100"))?,
    )?;
    assert_eq!(plan.filenames().count(), 3);
    Ok(())
}

#[test]
fn wordpress_only_export_can_span_visits() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let first = sample(FIRST, CollectionSite::ExampleWest)?;
    let mut second = sample(SECOND, CollectionSite::ExampleEast)?;
    second.visit = first.visit + 1;
    second.analysis_date = "2026-11-22".parse()?;
    let destination = tempdir()?;
    let plan = prepare_export(
        &mut [first, second],
        &catalogue,
        request(Contact::WordPressOnly)?,
    )?;
    assert_eq!(plan.write(destination.path(), None)?.written.len(), 2);
    assert_eq!(
        plan.filenames().collect::<Vec<_>>(),
        [format!("{FIRST}.zip"), format!("{SECOND}.zip")]
    );
    Ok(())
}

#[test]
fn live_choices_and_missing_fields_reject_before_writing() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    assert!(
        !live_textures()
            .iter()
            .any(|texture| matches!(*texture, "Bar" | "Pill" | "Rock"))
    );
    let mut record = [sample(EAST, CollectionSite::ExampleEast)?];
    let sample = record.first_mut().ok_or("no test sample")?;
    sample.texture = "Pill".into();
    assert!(prepare_export(&mut record, &catalogue, request(Contact::WordPressOnly)?).is_err());
    let sample = record.first_mut().ok_or("no test sample")?;
    sample.texture = "Tablet".into();
    sample.pill_shape = Some("invalid shape".into());
    assert!(prepare_export(&mut record, &catalogue, request(Contact::WordPressOnly)?).is_err());
    let sample = record.first_mut().ok_or("no test sample")?;
    sample.texture = "Tablet".into();
    sample.results.clear();
    assert!(prepare_export(&mut record, &catalogue, request(Contact::WordPressOnly)?).is_err());
    let sample = record.first().ok_or("no test sample")?;
    assert_eq!(sample.code_word, "");
    Ok(())
}

#[test]
fn hidden_details_are_omitted_before_validation() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let destination = tempdir()?;
    let mut record = sample(EAST, CollectionSite::ExampleEast)?;
    let stale = "x".repeat(256);
    record.expected_substance = Substance::Unknown;
    record.sold_as_other = Some(stale.clone());
    record.colour = "Blue".into();
    record.colour_other = Some(stale.clone());
    record.texture = "Powder".into();
    record.texture_other = Some(stale.clone());
    record.pill_shape = Some("invalid shape".into());
    record.pill_shape_other = Some(stale.clone());
    record.adverse_events = AdverseEvents::No;
    record.adverse_events_other = Some(stale.clone());
    record.results.first_mut().ok_or("no test result")?.other = Some(stale);

    let plan = prepare_export(
        std::slice::from_mut(&mut record),
        &catalogue,
        request(Contact::WordPressOnly)?,
    )?;
    plan.write(destination.path(), None)?;
    let mut archive = ZipArchive::new(File::open(destination.path().join(format!("{EAST}.zip")))?)?;
    let payload: Value = serde_json::from_slice(&read_member(&mut archive, "test_data.json")?)?;
    for field in [
        "sold_as_other",
        "colour_other",
        "texture_other",
        "pill_shape",
        "pill_shape_other",
        "advertse_events_yes_other",
    ] {
        assert!(payload.get(field).is_none(), "hidden field {field}");
    }
    let first = payload
        .get("results")
        .and_then(|rows| rows.get(0))
        .ok_or("no exported row")?;
    assert!(first.get("other").is_none());

    record.colour = "other".into();
    let rejected = prepare_export(
        std::slice::from_mut(&mut record),
        &catalogue,
        request(Contact::WordPressOnly)?,
    );
    assert!(matches!(
        rejected,
        Err(ExportError::Sample { error, .. })
            if matches!(*error, ExportError::InvalidField { field: "colour details", .. })
    ));
    Ok(())
}

fn single(title: &str, catalogue: &Catalogue) -> TestResult<PreparedExport> {
    Ok(prepare_export(
        &mut [sample(title, CollectionSite::ExampleEast)?],
        catalogue,
        request(Contact::WordPressOnly)?,
    )?)
}

#[test]
fn conflicts_require_fresh_explicit_approval_and_safe_filenames() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let destination = tempdir()?;
    let plan = single("CON.report", &catalogue)?;
    let expected = format!("sample-{}.zip", digest(b"CON.report"));
    assert_eq!(plan.filenames().next(), Some(expected.as_str()));
    let path = destination.path().join(&expected);
    fs::write(&path, b"existing export")?;
    let Err(WriteError::Conflicts(approved)) = plan.write(destination.path(), None) else {
        return Err("missing overwrite conflict".into());
    };
    assert_eq!(
        approved.filenames().collect::<Vec<_>>(),
        [expected.as_str()]
    );
    assert_eq!(fs::read(&path)?, b"existing export");
    fs::write(&path, b"replaced export")?;
    assert!(matches!(
        plan.write(destination.path(), Some(&approved)),
        Err(WriteError::Conflicts(_))
    ));
    assert_eq!(fs::read(&path)?, b"replaced export");
    let approved = plan.inspect_folder(destination.path())?;
    assert_eq!(
        plan.write(destination.path(), Some(&approved))?.written,
        [path.canonicalize()?]
    );
    let mut archive = ZipArchive::new(File::open(&path)?)?;
    let payload: Value = serde_json::from_slice(&read_member(&mut archive, "test_data.json")?)?;
    assert_eq!(
        payload.get("submission_number"),
        Some(&Value::from("CON.report"))
    );
    assert_eq!(payload.get("site"), Some(&Value::from("fixture-east")));
    assert_eq!(fs::read_dir(destination.path())?.count(), 1);
    Ok(())
}

#[test]
fn abandoned_overwrite_blocks_inspection_and_retry_until_staff_reconcile_it() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let destination = tempdir()?;
    let folder = destination.path().canonicalize()?;
    let plan = single(EAST, &catalogue)?;
    let name = plan.filenames().next().ok_or("missing export filename")?;
    let target = folder.join(name);
    plan.write(&folder, None)?;
    let original_zip = fs::read(&target)?;
    let approval = plan.inspect_folder(&folder)?;

    // On-disk state left by an interruption immediately after moving the original.
    let staging_name = ".gydt-export-Qa1234";
    let staging = folder.join(staging_name);
    let builder = &mut fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        builder.mode(0o700);
    }
    builder.create(&staging)?;
    fs::create_dir(staging.join("originals"))?;
    let original = staging.join("originals").join(name);
    fs::rename(&target, &original)?;

    let Err(WriteError::RecoveryRequired(found)) = plan.inspect_folder(&folder) else {
        return Err("inspection missed the retained original".into());
    };
    assert_eq!(found.originals(), std::slice::from_ref(&original));
    assert!(found.to_string().contains(staging_name));
    assert!(!found.to_string().contains(name));
    assert!(!format!("{found:?}").contains(name));
    for approval in [None, Some(&approval)] {
        let Err(WriteError::RecoveryRequired(found)) = plan.write(&folder, approval) else {
            return Err("retry bypassed the retained original".into());
        };
        assert_eq!(found.originals(), std::slice::from_ref(&original));
        assert!(!target.try_exists()?);
        assert_eq!(fs::read(&original)?, original_zip);
    }
    assert_eq!(fs::read_dir(&folder)?.count(), 1);

    fs::write(&target, b"independently created export")?;
    assert!(matches!(
        plan.write(&folder, Some(&approval)),
        Err(WriteError::RecoveryRequired(_))
    ));
    assert_eq!(fs::read(&target)?, b"independently created export");
    assert_eq!(fs::read(&original)?, original_zip);
    let reconciliation = tempdir()?;
    let preserved = reconciliation.path().join(name);
    fs::rename(&original, &preserved)?;

    let approved = plan.inspect_folder(&folder)?;
    let completed = plan.write(&folder, Some(&approved))?;
    assert_eq!(completed.written, std::slice::from_ref(&target));
    assert!(completed.cleanup_warning.is_none());
    assert_eq!(fs::read(target)?, original_zip);
    assert_eq!(fs::read(preserved)?, original_zip);
    assert_eq!(fs::read_dir(folder)?.count(), 2);
    Ok(())
}

#[test]
fn a_new_contact_file_conflict_preserves_both_outputs() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let directory = tempdir()?;
    let folder = directory.path();
    let plan = prepare_export(
        &mut [sample(EAST, CollectionSite::ExampleEast)?],
        &catalogue,
        request(Contact::Text("6045550100"))?,
    )?;
    let zip = folder.join(format!("{EAST}.zip"));
    let draft = folder.join(format!("{EAST}.txt"));
    fs::write(&zip, "original ZIP")?;
    let approved_zip = plan.inspect_folder(folder)?;
    fs::write(&draft, "original draft")?;

    let Err(WriteError::Conflicts(conflicts)) = plan.write(folder, Some(&approved_zip)) else {
        return Err("expected updated conflicts".into());
    };
    assert_eq!(conflicts.filenames().count(), 2);
    assert!(matches!(
        plan.write(folder, None),
        Err(WriteError::Conflicts(_))
    ));
    assert_eq!(fs::read_to_string(&zip)?, "original ZIP");
    assert_eq!(fs::read_to_string(&draft)?, "original draft");
    assert_eq!(plan.write(folder, Some(&conflicts))?.written.len(), 2);
    assert!(fs::read_to_string(draft)?.contains("Code word: "));
    assert_eq!(ZipArchive::new(File::open(zip)?)?.len(), 2);
    assert_eq!(fs::read_dir(folder)?.count(), 2);
    Ok(())
}

#[test]
fn blank_contact_leaves_only_the_zip_and_header_breaks_are_rejected() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let mut record = [sample(EAST, CollectionSite::ExampleEast)?];
    let input = record.first_mut().ok_or("no test sample")?;
    input.notes = Some("  \t ".into());
    let rejected = prepare_export(
        &mut record,
        &catalogue,
        request(Contact::Email(
            "synthetic@example.invalid\r\nBcc: another@example.invalid",
        ))?,
    );
    assert!(matches!(
        rejected,
        Err(ExportError::InvalidField {
            field: "email address",
            ..
        })
    ));
    let plan = prepare_export(&mut record, &catalogue, request(Contact::Text(" "))?)?;
    let destination = tempdir()?;
    plan.write(destination.path(), None)?;
    assert_eq!(fs::read_dir(destination.path())?.count(), 1);
    let mut archive = ZipArchive::new(File::open(destination.path().join(format!("{EAST}.zip")))?)?;
    let payload: Value = serde_json::from_slice(&read_member(&mut archive, "test_data.json")?)?;
    assert!(payload.get("notes").is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn an_output_link_leaves_the_existing_target_untouched() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let folder = tempdir()?;
    let source = tempdir()?;
    let target = source.path().join("existing");
    fs::write(&target, "unrelated contents")?;
    let link = folder.path().join(format!("{EAST}.zip"));
    std::os::unix::fs::symlink(&target, &link)?;
    let plan = single(EAST, &catalogue)?;
    assert!(matches!(
        plan.write(folder.path(), None),
        Err(WriteError::Prepare { .. })
    ));
    assert!(link.symlink_metadata()?.is_symlink());
    assert_eq!(fs::read_to_string(target)?, "unrelated contents");
    Ok(())
}

#[test]
fn history_checks_identity_and_shared_standard_title() -> TestResult {
    let catalogue = Catalogue::bundled()?;
    let mut records = [sample(FIRST, CollectionSite::ExampleWest)?];
    let plan = prepare_export(&mut records, &catalogue, request(Contact::WordPressOnly)?)?;
    let mut request = request(Contact::WordPressOnly)?;
    request.history = plan.history();
    assert!(matches!(
        prepare_export(&mut records, &catalogue, request),
        Err(ExportError::ReexportConfirmation(_))
    ));
    let id = records.first().ok_or("no test sample")?.id;
    let confirmed = [id];
    request.confirmed_reexports = &confirmed;
    let repeat = prepare_export(&mut records, &catalogue, request)?;
    assert_eq!(repeat.history().first().ok_or("no history entry")?.id, id);
    let mut collision = [sample(FIRST, CollectionSite::ExampleEast)?];
    assert!(matches!(
        prepare_export(&mut collision, &catalogue, request),
        Err(ExportError::DuplicateSubmission(_))
    ));
    Ok(())
}

#[test]
#[ignore = "writes synthetic WordPress fixtures to an explicit scratch directory"]
fn stage_synthetic_wordpress_fixtures() -> TestResult {
    let dir = std::env::var("GYDT_SYNTHETIC_EXPORT_DIR")?;
    let path = Path::new(&dir);
    let catalogue = Catalogue::bundled()?;
    let inputs = tempdir()?;
    prepare_export(
        &mut pair(inputs.path())?,
        &catalogue,
        request(Contact::WordPressOnly)?,
    )?
    .write(path, None)?;
    single(EAST, &catalogue)?.write(path, None)?;
    Ok(())
}
