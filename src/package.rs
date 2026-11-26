use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Cursor, Write as _},
};

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use serde::Serialize;
use uuid::{Uuid, Variant, Version};
use zip::{CompressionMethod, System, ZipWriter, write::SimpleFileOptions};

use crate::{
    AdverseEvents, Catalogue, CollectionSite, Consumption, ExportError, Profile, Sample, Strips,
    Substance, draft, invalid, live_textures, sha256,
};

const MAX_MANIFEST: usize = 8 * 1024;
const MAX_DATA: usize = 64 * 1024;
const MAX_ZIP: usize = 5 * 1024 * 1024;
const COLOURS: &[&str] = &[
    "Black",
    "Blue",
    "Light Blue",
    "Dark Blue",
    "Brown",
    "Light Brown",
    "Dark Brown",
    "Colourless",
    "Green",
    "Light Green",
    "Dark Green",
    "Grey",
    "Light Grey",
    "Dark Grey",
    "Orange",
    "Light Orange",
    "Dark Orange",
    "Pink",
    "Light Pink",
    "Dark Pink",
    "Purple",
    "Light Purple",
    "Dark Purple",
    "Red",
    "Light Red",
    "Dark Red",
    "White",
    "Yellow",
    "Light Yellow",
    "Dark Yellow",
    "other",
];
const SHAPES: &[&str] = &[
    "round",
    "rectangle",
    "capsule",
    "teardrop",
    "oval",
    "barrel",
    "cup",
    "heart",
    "star",
    "3-sided",
    "4-sided",
    "5-sided",
    "6-sided",
    "7-sided",
    "8-sided",
    "figure8",
    "character",
    "unique",
    "other",
];

#[derive(Clone, Copy)]
pub enum Contact<'a> {
    WordPressOnly,
    Email(&'a str),
    Text(&'a str),
}

/// The identity and scope of a previously successful export. Persist these
/// entries alongside the desktop records across export-folder changes.
pub struct ExportHistoryEntry {
    pub id: Uuid,
    pub submission_number: String,
    pub site: CollectionSite,
    pub analysis_date: Date,
}

#[derive(Clone, Copy)]
pub struct ExportRequest<'a> {
    pub contact: Contact<'a>,
    pub exported_at: Timestamp,
    pub history: &'a [ExportHistoryEntry],
    pub confirmed_reexports: &'a [Uuid],
}

pub(crate) struct Artifact {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// A validated, immutable selection of output files.
pub struct PreparedExport {
    pub(crate) outputs: Vec<Artifact>,
    pub(crate) history: Vec<ExportHistoryEntry>,
}

impl PreparedExport {
    /// The exact filenames to show before writing or approving replacements.
    pub fn filenames(&self) -> impl ExactSizeIterator<Item = &str> {
        self.outputs.iter().map(|output| output.name.as_str())
    }

    /// Entries to persist after all outputs have been saved successfully.
    #[must_use]
    pub fn history(&self) -> &[ExportHistoryEntry] {
        &self.history
    }
}

#[derive(Serialize)]
pub(crate) struct Payload<'a> {
    pub(crate) submission_number: String,
    visit: u32,
    site: &'a str,
    analysis_date: String,
    collection_date: String,
    expected_substance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sold_as_other: Option<String>,
    pub(crate) results: Vec<Detection>,
    colour: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    colour_other: Option<String>,
    texture: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    texture_other: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pill_shape: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pill_shape_other: Option<String>,
    pre_post_consumption: Consumption,
    adverse_events: AdverseEvents,
    #[serde(skip_serializing_if = "Option::is_none")]
    advertse_events_yes_other: Option<String>,
    #[serde(flatten)]
    pub(crate) strips: Strips,
    pub(crate) client: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Detection {
    pub(crate) substance: String,
    pub(crate) percentage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) other: Option<String>,
}

#[derive(Serialize)]
struct Manifest<'a> {
    format_version: u8,
    sample_id: String,
    exported_at: &'a str,
    catalogue_version: &'a str,
    files: BTreeMap<&'a str, Member>,
}

#[derive(Serialize)]
struct Member {
    bytes: usize,
    sha256: String,
}

/// Validates the entire selection and prepares one ZIP per sample and at most
/// one contact draft. Newly generated code words are retained in `samples`.
///
/// # Errors
/// Returns an error when input, catalogue selections, history, or filenames
/// conflict with the format. Preparation does not write to the destination.
pub fn prepare_export(
    samples: &mut [Sample],
    catalogue: &Catalogue,
    request: ExportRequest<'_>,
) -> Result<PreparedExport, ExportError> {
    let templates = draft::Templates::fixture();
    prepare(samples, catalogue, &templates, fixture_site, request)
}

/// Prepares the same format-1 export with the selected private profile.
///
/// # Errors
/// Returns an error if the supplied sample or profile cannot form a ZIP.
pub fn prepare_profile_export(
    samples: &mut [Sample],
    profile: &Profile,
    request: ExportRequest<'_>,
) -> Result<PreparedExport, ExportError> {
    prepare(
        samples,
        profile.catalogue(),
        &profile.templates,
        |site| profile.site(site).token(),
        request,
    )
}

fn fixture_site(site: CollectionSite) -> &'static str {
    match site {
        CollectionSite::ExampleEast => "fixture-east",
        CollectionSite::ExampleWest => "fixture-west",
    }
}

fn prepare<'a>(
    samples: &mut [Sample],
    catalogue: &Catalogue,
    templates: &draft::Templates,
    site_token: impl Fn(CollectionSite) -> &'a str,
    request: ExportRequest<'_>,
) -> Result<PreparedExport, ExportError> {
    let contact = draft::validate_contact(request.contact)?;
    if !matches!(contact, Contact::WordPressOnly)
        && let Some((first, rest)) = samples.split_first()
        && rest.iter().any(|sample| {
            (sample.site, sample.analysis_date, sample.visit)
                != (first.site, first.analysis_date, first.visit)
        })
    {
        return Err(ExportError::MixedContactVisit);
    }
    let mut payloads = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            validate(sample, catalogue, site_token(sample.site)).map_err(|error| {
                ExportError::Sample {
                    number: index + 1,
                    error: Box::new(error),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first = payloads
        .first()
        .ok_or_else(|| invalid("selection", "Select at least one sample."))?;
    let first_stem = filename_stem(&first.submission_number);
    let history = collect_history(samples, &payloads, &request)?;
    if request.exported_at.to_zoned(TimeZone::UTC).year() < 1 {
        return Err(invalid(
            "export timestamp",
            "Use a UTC timestamp in years 0001–9999.",
        ));
    }
    let exported_at = request
        .exported_at
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let mut names = HashSet::new();
    let mut outputs = Vec::new();
    let visit_key = |sample: &Sample| (sample.site, sample.analysis_date, sample.visit);
    let mut visit_codes = HashMap::<_, HashSet<_>>::new();
    for (sample, payload) in samples.iter().zip(&payloads) {
        if !payload.client.is_empty() {
            visit_codes
                .entry(visit_key(sample))
                .or_default()
                .insert(payload.client.to_ascii_uppercase());
        }
    }
    for (sample, payload) in samples.iter().zip(&mut payloads) {
        let stem = filename_stem(&payload.submission_number);
        let name = format!("{stem}.zip");
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(ExportError::OutputNameConflict(name));
        }
        if payload.client.is_empty() {
            let reserved = visit_codes.entry(visit_key(sample)).or_default();
            payload.client = reserve_code(reserved, random_code_start()?)?;
        }
        let bytes = package(sample, payload, &stem, &exported_at, catalogue.version())?;
        outputs.push(Artifact { name, bytes });
    }
    if let Some(artifact) = draft::make(contact, catalogue, templates, &payloads, &first_stem)? {
        outputs.push(artifact);
    }
    let codes: Vec<_> = payloads.into_iter().map(|payload| payload.client).collect();
    for (sample, code) in samples.iter_mut().zip(codes) {
        if sample.code_word.trim().is_empty() {
            sample.code_word = code;
        }
    }
    Ok(PreparedExport { outputs, history })
}

fn validate<'a>(
    sample: &'a Sample,
    catalogue: &Catalogue,
    site: &'a str,
) -> Result<Payload<'a>, ExportError> {
    validate_identity(sample)?;
    let submission_number = required_text(&sample.submission_number, "submission number", 200)?;
    let expected_substance = substance(sample.expected_substance, catalogue, "expected substance")?;
    let sold_as_other = optional_text(
        sample.sold_as_other.as_deref(),
        "Sold As details",
        255,
        expected_substance == "other",
    )?;
    let results = detections(sample, catalogue)?;
    let colour = choice(&sample.colour, COLOURS, "sample colour")?;
    let texture = choice(&sample.texture, live_textures(), "sample texture")?;
    let pill_shape = sample
        .pill_shape
        .as_deref()
        .filter(|value| matches!(texture, "Pressed Pill" | "Tablet") && !value.is_empty())
        .map(|value| choice(value, SHAPES, "shape"))
        .transpose()?;
    if matches!(sample.consumption, Consumption::Unanswered)
        && !matches!(sample.adverse_events, AdverseEvents::Unanswered)
    {
        return Err(invalid(
            "unexpected effects",
            "Answer whether the sample was used first.",
        ));
    }

    Ok(Payload {
        submission_number,
        visit: sample.visit,
        site,
        analysis_date: sample.analysis_date.to_string(),
        collection_date: sample.collection_date.to_string(),
        expected_substance,
        sold_as_other,
        results,
        colour,
        texture,
        pill_shape,
        colour_other: optional_text(
            sample.colour_other.as_deref(),
            "colour details",
            255,
            colour == "other",
        )?,
        texture_other: optional_text(
            sample.texture_other.as_deref(),
            "texture details",
            255,
            texture == "other",
        )?,
        pill_shape_other: optional_text(
            sample.pill_shape_other.as_deref(),
            "shape details",
            255,
            pill_shape == Some("other"),
        )?,
        pre_post_consumption: sample.consumption,
        adverse_events: sample.adverse_events,
        advertse_events_yes_other: optional_text(
            sample.adverse_events_other.as_deref(),
            "unexpected effects details",
            255,
            matches!(sample.adverse_events, AdverseEvents::Yes),
        )?,
        strips: sample.strips,
        client: text(&sample.code_word, "code word", 255)?.unwrap_or_default(),
        notes: optional_text(sample.notes.as_deref(), "notes", 2000, true)?,
    })
}

fn validate_identity(sample: &Sample) -> Result<(), ExportError> {
    if sample.id.get_version() != Some(Version::Random)
        || sample.id.get_variant() != Variant::RFC4122
    {
        return Err(invalid(
            "sample ID",
            "Use the saved UUIDv4 for this sample.",
        ));
    }
    if !(1..=2_147_483_647).contains(&sample.visit) {
        return Err(invalid(
            "Visit number",
            "Enter a positive integer up to 2,147,483,647.",
        ));
    }
    if sample.analysis_date.year() < 1000 || sample.collection_date.year() < 1000 {
        return Err(invalid("date", "Use years 1000–9999."));
    }
    let zone = TimeZone::get("America/Vancouver")
        .map_err(|_| invalid("collection date", "The Vancouver time zone is unavailable."))?;
    zone.to_ambiguous_zoned(sample.collection_date.at(0, 0, 0, 0))
        .unambiguous()
        .map_err(|_| {
            invalid(
                "collection date",
                "The date has no unique Vancouver midnight.",
            )
        })?;

    Ok(())
}

fn choice<'a>(
    value: &'a str,
    allowed: &[&str],
    field: &'static str,
) -> Result<&'a str, ExportError> {
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(invalid(field, "Choose a supported value."))
    }
}

fn substance(
    value: Substance,
    catalogue: &Catalogue,
    field: &'static str,
) -> Result<String, ExportError> {
    match value {
        Substance::Catalogue(id) if catalogue.get(id).is_some() => Ok(id.to_string()),
        Substance::Catalogue(_) => Err(invalid(
            field,
            "Choose a substance from the bundled catalogue.",
        )),
        Substance::Unknown => Ok("unknown".into()),
        Substance::Other => Ok("other".into()),
    }
}

fn detections(sample: &Sample, catalogue: &Catalogue) -> Result<Vec<Detection>, ExportError> {
    if !(1..=10).contains(&sample.results.len()) {
        return Err(invalid(
            "detected substances",
            "Include one to ten results.",
        ));
    }
    sample
        .results
        .iter()
        .map(|row| {
            let substance = substance(row.substance, catalogue, "detected substance")?;
            let percentage = match row.percentage {
                None => "blank".into(),
                Some(range) if range.low < range.high && range.high <= 100 => {
                    format!("{}-{}%", range.low, range.high)
                }
                Some(_) => {
                    return Err(invalid(
                        "percentage",
                        "Enter an increasing whole-number range within 0–100%.",
                    ));
                }
            };
            let other = optional_text(
                row.other.as_deref(),
                "detected substance details",
                255,
                substance == "other",
            )?;
            Ok(Detection {
                substance,
                percentage,
                other,
            })
        })
        .collect()
}

fn text(value: &str, field: &'static str, max: usize) -> Result<Option<String>, ExportError> {
    let normalized = value
        .replace("\r\n", " ")
        .replace(['\r', '\n', '\t', '\u{2028}', '\u{2029}'], " ");
    let value = normalized.trim_matches(' ');
    if value.len() > max
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>'))
    {
        return Err(invalid(
            field,
            "Enter plain text within the allowed byte limit.",
        ));
    }
    Ok((!value.chars().all(char::is_whitespace)).then(|| value.to_owned()))
}

fn required_text(value: &str, field: &'static str, max: usize) -> Result<String, ExportError> {
    text(value, field, max)?.ok_or_else(|| invalid(field, "This field is required."))
}

fn optional_text(
    value: Option<&str>,
    field: &'static str,
    max: usize,
    allowed: bool,
) -> Result<Option<String>, ExportError> {
    if allowed {
        text(value.unwrap_or_default(), field, max)
    } else {
        Ok(None)
    }
}

const CODE_WORDS: [&str; 16] = [
    "ASPEN", "BIRCH", "CEDAR", "CLOVER", "FERN", "HAZEL", "LARCH", "MAPLE", "MOSS", "OAK",
    "ORCHID", "PINE", "REED", "SPRUCE", "VIOLET", "WILLOW",
];
const CODE_COUNT: u32 = 160_000;

fn random_code_start() -> Result<u32, ExportError> {
    loop {
        let number = getrandom::u32()?;
        if number < u32::MAX - u32::MAX % CODE_COUNT {
            return Ok(number % CODE_COUNT);
        }
    }
}

fn reserve_code(reserved: &mut HashSet<String>, start: u32) -> Result<String, ExportError> {
    for offset in 0..CODE_COUNT {
        let candidate = (start + offset) % CODE_COUNT;
        let word = CODE_WORDS
            .get((candidate / 10_000) as usize)
            .ok_or_else(|| invalid("code word", "The word list is unavailable."))?;
        let code = format!("{word}{:04}", candidate % 10_000);
        if reserved.insert(code.clone()) {
            return Ok(code);
        }
    }
    Err(invalid("code word", "Enter a code word for this sample."))
}

fn filename_stem(title: &str) -> String {
    let base = title
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && suffix.bytes().any(|digit| (b'1'..=b'9').contains(&digit))
            })
        });
    if title.len() <= 80
        && !title.ends_with(['.', ' '])
        && !device
        && title
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && title
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b" _.-".contains(&byte))
    {
        title.into()
    } else {
        format!("sample-{}", sha256(title.as_bytes()))
    }
}

fn standard_title(title: &str) -> bool {
    let Ok([city, site, year, month, day, serial]) =
        <[&str; 6]>::try_from(title.split('-').collect::<Vec<_>>())
    else {
        return false;
    };
    !city.is_empty()
        && site.len() == 3
        && year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && serial.len() >= 3
        && [city, site, year, month, day, serial]
            .into_iter()
            .all(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
        && serial.bytes().any(|digit| digit != b'0')
        && format!("{year}-{month}-{day}")
            .parse::<Date>()
            .is_ok_and(|date| date.year() >= 1000)
}

fn same_scope(a: &ExportHistoryEntry, b: &ExportHistoryEntry) -> bool {
    a.submission_number == b.submission_number
        && (standard_title(&a.submission_number)
            || (a.site == b.site && a.analysis_date == b.analysis_date))
}

fn collect_history(
    samples: &[Sample],
    payloads: &[Payload<'_>],
    request: &ExportRequest<'_>,
) -> Result<Vec<ExportHistoryEntry>, ExportError> {
    let mut history = Vec::new();
    for (sample, payload) in samples.iter().zip(payloads) {
        let entry = ExportHistoryEntry {
            id: sample.id,
            submission_number: payload.submission_number.clone(),
            site: sample.site,
            analysis_date: sample.analysis_date,
        };
        if history
            .iter()
            .any(|other: &ExportHistoryEntry| other.id == entry.id || same_scope(&entry, other))
            || request
                .history
                .iter()
                .any(|other| other.id != entry.id && same_scope(&entry, other))
        {
            return Err(ExportError::DuplicateSubmission(entry.submission_number));
        }
        if request.history.iter().any(|other| other.id == entry.id)
            && !request.confirmed_reexports.contains(&entry.id)
        {
            return Err(ExportError::ReexportConfirmation(entry.submission_number));
        }
        history.push(entry);
    }
    Ok(history)
}

fn package(
    sample: &Sample,
    payload: &Payload<'_>,
    stem: &str,
    exported_at: &str,
    catalogue_version: &str,
) -> Result<Vec<u8>, ExportError> {
    let data = serde_json::to_vec(payload)?;
    if data.len() > MAX_DATA {
        return Err(invalid(
            "sample data",
            "The ZIP sample JSON exceeds 64 KiB.",
        ));
    }
    let mut members = BTreeMap::from([("test_data.json", data.as_slice())]);
    let photo_name = sample
        .photo
        .as_ref()
        .map(|photo| format!("{stem}.{}", photo.extension));
    let photo_copy = sample
        .photo
        .as_ref()
        .map(crate::Photo::for_import)
        .transpose()?;
    if let (Some(name), Some(bytes)) = (photo_name.as_deref(), photo_copy.as_deref()) {
        members.insert(name, bytes);
    }
    let files = members
        .iter()
        .map(|(name, bytes)| {
            (
                *name,
                Member {
                    bytes: bytes.len(),
                    sha256: sha256(bytes),
                },
            )
        })
        .collect();
    let manifest = serde_json::to_vec(&Manifest {
        format_version: 1,
        sample_id: sample.id.to_string(),
        exported_at,
        catalogue_version,
        files,
    })?;
    if manifest.len() > MAX_MANIFEST {
        return Err(invalid("manifest", "The ZIP manifest exceeds 8 KiB."));
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    // Match the Unix regular-file mode with an explicit creator platform on every host.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .system(System::Unix)
        .unix_permissions(0o600);
    zip.start_file("manifest.json", options)?;
    zip.write_all(&manifest)?;
    for (name, bytes) in members {
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
    }
    let bytes = zip.finish()?.into_inner();
    if bytes.len() > MAX_ZIP {
        return Err(invalid("ZIP", "The WordPress package exceeds 5 MiB."));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generating_codes_skips_entered_and_newly_reserved_codes() -> Result<(), ExportError> {
        let entered = "aspen0000";
        let mut reserved = HashSet::from([entered.to_ascii_uppercase()]);
        assert_eq!(reserve_code(&mut reserved, 0)?, "ASPEN0001");
        assert_eq!(reserve_code(&mut reserved, 0)?, "ASPEN0002");
        assert_eq!(reserve_code(&mut HashSet::new(), 0)?, "ASPEN0000");
        Ok(())
    }
}
