use std::{io, path::Path, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use gydt_results_compiler::{
    AdverseEvents, CollectionSite, Consumption, Contact, DetectedResult, Percentage, Photo, Sample,
    StripResult, Strips, Substance,
};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

pub(super) const COLOURS: &[&str] = &[
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
pub(super) const SHAPES: &[(&str, &str)] = &[
    ("round", "Round"),
    ("rectangle", "Rectangle"),
    ("capsule", "Capsule"),
    ("teardrop", "Tear Drop"),
    ("oval", "Oval"),
    ("barrel", "Barrel"),
    ("cup", "Cup"),
    ("heart", "Heart"),
    ("star", "Star"),
    ("3-sided", "3-Sided"),
    ("4-sided", "4-Sided"),
    ("5-sided", "5-Sided"),
    ("6-sided", "6-Sided"),
    ("7-sided", "7-Sided"),
    ("8-sided", "8-Sided"),
    ("figure8", "Figure 8"),
    ("character", "Character"),
    ("unique", "Unique"),
    ("other", "Other"),
];
pub(super) const MAX_JPEG: u64 = 4 * 1024 * 1024;
pub(super) const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg"];
pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(super) enum Site {
    #[default]
    #[serde(rename = "fixture-east")]
    ExampleEast,
    #[serde(rename = "fixture-west")]
    ExampleWest,
}

impl Site {
    pub(super) const ALL: [Self; 2] = [Self::ExampleEast, Self::ExampleWest];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ExampleEast => "Example East",
            Self::ExampleWest => "Example West",
        }
    }

    pub(super) const fn slot(self) -> usize {
        match self {
            Self::ExampleEast => 0,
            Self::ExampleWest => 1,
        }
    }
}

impl From<Site> for CollectionSite {
    fn from(site: Site) -> Self {
        match site {
            Site::ExampleEast => Self::ExampleEast,
            Site::ExampleWest => Self::ExampleWest,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) enum Channel {
    #[default]
    None,
    Email,
    Text,
}

impl Channel {
    pub(super) const ALL: [Self; 3] = [Self::None, Self::Email, Self::Text];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Email => "Email",
            Self::Text => "Text",
        }
    }

    pub(super) fn contact(self, address: &str) -> Contact<'_> {
        let address = address.trim();
        match (self, address.is_empty()) {
            (Self::Email, false) => Contact::Email(address),
            (Self::Text, false) => Contact::Text(address),
            _ => Contact::WordPressOnly,
        }
    }

    pub(super) fn extension(self, address: &str) -> Option<&'static str> {
        match self.contact(address) {
            Contact::Email(_) => Some("eml"),
            Contact::Text(_) => Some("txt"),
            Contact::WordPressOnly => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) enum Strip {
    #[default]
    #[serde(rename = "Not Used")]
    Blank,
    Positive,
    Negative,
    Invalid,
}

impl Strip {
    pub(super) const ALL: [Self; 4] = [Self::Blank, Self::Positive, Self::Negative, Self::Invalid];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Blank => "",
            Self::Positive => "Positive",
            Self::Negative => "Negative",
            Self::Invalid => "Invalid",
        }
    }
}

impl From<Strip> for StripResult {
    fn from(choice: Strip) -> Self {
        match choice {
            Strip::Blank => Self::NotUsed,
            Strip::Positive => Self::Positive,
            Strip::Negative => Self::Negative,
            Strip::Invalid => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) enum Selection {
    Catalogue(u64),
    Unknown,
    Other,
}

impl Selection {
    pub(super) fn parse(token: &str) -> Option<Self> {
        match token {
            "unknown" => Some(Self::Unknown),
            "other" => Some(Self::Other),
            _ => {
                let id = token.parse::<u64>().ok()?;
                (id > 0 && id.to_string() == token).then_some(Self::Catalogue(id))
            }
        }
    }
}

impl From<Selection> for Substance {
    fn from(choice: Selection) -> Self {
        match choice {
            Selection::Catalogue(id) => Self::Catalogue(id),
            Selection::Unknown => Self::Unknown,
            Selection::Other => Self::Other,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ResultRow {
    pub(super) substance: Option<Selection>,
    pub(super) range: String,
    pub(super) custom: bool,
    pub(super) other: String,
}

pub(super) fn parse_range(range: &str) -> Option<Percentage> {
    let (low, high) = range.strip_suffix('%')?.split_once('-')?;
    let range = Percentage {
        low: low.parse().ok()?,
        high: high.parse().ok()?,
    };
    (range.low < range.high
        && range.high <= 100
        && range.low.to_string() == low
        && range.high.to_string() == high)
        .then_some(range)
}

impl ResultRow {
    fn result(&self) -> Result<Option<DetectedResult>> {
        let Some(substance) = self.substance else {
            return Ok(None);
        };
        let percentage = if self.range.is_empty() && !self.custom {
            None
        } else {
            Some(
                parse_range(&self.range)
                    .ok_or("Enter an increasing percentage range such as 10-17%.")?,
            )
        };
        Ok(Some(DetectedResult {
            substance: substance.into(),
            percentage,
            other: optional_text(&self.other),
        }))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct PhotoCopy {
    pub(super) name: String,
    pub(super) extension: String,
    data: Arc<str>,
}

impl PhotoCopy {
    pub(super) fn from_path(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Choose a JPEG with a readable filename.")?;
        let extension = photo_ext(name).ok_or("Choose a photo ending in .jpg or .jpeg.")?;
        let bytes = super::archive::read_file(path, MAX_JPEG, "JPG/JPEG photo")
            .map_err(|error| format!("Could not read {name}: {error}"))?;
        let validated = Photo::from_jpeg_bytes(extension, bytes)?;
        Ok(Self {
            name: name.into(),
            extension: extension.into(),
            data: STANDARD.encode(validated.into_jpeg_bytes()).into(),
        })
    }

    pub(super) fn new(name: &str, extension: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            extension: extension.into(),
            data: STANDARD.encode(bytes).into(),
        }
    }

    pub(super) fn bytes(&self) -> Result<Vec<u8>> {
        Ok(STANDARD.decode(self.data.as_bytes())?)
    }

    pub(super) fn prepare(&self) -> Result<Photo> {
        Ok(Photo::from_jpeg_bytes(&self.extension, self.bytes()?)?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Draft {
    pub(super) id: Uuid,
    pub(super) title: String,
    pub(super) suggested_title: Option<String>,
    pub(super) visit: String,
    pub(super) suggested_visit: Option<u32>,
    /// High-water marks for this sample's analysis day, by site.
    #[serde(default)]
    pub(super) visit_claims: [u32; 2],
    pub(super) site: Site,
    pub(super) analysis_date: String,
    pub(super) collection_date: String,
    pub(super) expected: Option<Selection>,
    pub(super) sold_as_other: String,
    pub(super) rows: Vec<ResultRow>,
    pub(super) colour: String,
    pub(super) colour_other: String,
    pub(super) texture: String,
    pub(super) texture_other: String,
    pub(super) shape: String,
    pub(super) shape_other: String,
    pub(super) used: Option<bool>,
    pub(super) effects: Option<bool>,
    pub(super) effects_other: String,
    pub(super) strips: [Strip; 4],
    pub(super) code_word: String,
    pub(super) notes: String,
    pub(super) photo: Option<PhotoCopy>,
    pub(super) channel: Channel,
    pub(super) address: String,
    pub(super) exported: bool,
}

impl Draft {
    pub(super) fn fresh(site: Site, day: Date) -> Result<Self> {
        Ok(Self {
            id: gydt_results_compiler::new_sample_id()?,
            title: String::new(),
            suggested_title: None,
            visit: String::new(),
            suggested_visit: None,
            visit_claims: [0; 2],
            site,
            analysis_date: day.to_string(),
            collection_date: day.to_string(),
            expected: None,
            sold_as_other: String::new(),
            rows: vec![ResultRow::default()],
            colour: String::new(),
            colour_other: String::new(),
            texture: String::new(),
            texture_other: String::new(),
            shape: String::new(),
            shape_other: String::new(),
            used: None,
            effects: None,
            effects_other: String::new(),
            strips: [Strip::Blank; 4],
            code_word: String::new(),
            notes: String::new(),
            photo: None,
            channel: Channel::None,
            address: String::new(),
            exported: false,
        })
    }

    pub(super) fn tab_label(&self) -> &str {
        if self.title.trim().is_empty() {
            "New sample"
        } else {
            &self.title
        }
    }

    pub(super) fn has_entry_data(&self) -> bool {
        (self.suggested_title.is_none() && !self.title.trim().is_empty())
            || (self.collection_date != self.analysis_date
                && valid_date(&self.collection_date).is_some())
            || self.expected.is_some()
            || self.rows.iter().any(|row| row.substance.is_some())
            || !self.colour.is_empty()
            || !self.texture.is_empty()
            || self.used.is_some()
            || self.effects.is_some()
            || self.strips.iter().any(|strip| *strip != Strip::Blank)
            || [&self.code_word, &self.notes, &self.address]
                .iter()
                .any(|value| !value.trim().is_empty())
            || self.photo.is_some()
    }

    pub(super) fn suggest_visit(&mut self, visit: u32) {
        self.visit = visit.to_string();
        self.suggested_visit = Some(visit);
    }

    pub(super) fn claimed_visit(&self, site: Site) -> Option<u32> {
        self.visit_claims
            .get(site.slot())
            .copied()
            .filter(|visit| *visit > 0)
    }

    pub(super) fn claim_visit(&mut self) {
        if let Some(visit) = parse_visit(&self.visit)
            && let Some(highest) = self.visit_claims.get_mut(self.site.slot())
        {
            *highest = (*highest).max(visit);
        }
    }

    pub(super) fn has_missing(&self) -> bool {
        self.title.trim().is_empty()
            || parse_visit(&self.visit).is_none()
            || valid_date(&self.collection_date).is_none()
            || self.expected.is_none()
            || self.colour.is_empty()
            || self.texture.is_empty()
            || !self.rows.iter().any(|row| row.substance.is_some())
    }

    pub(super) fn normalize(&mut self) {
        for value in [
            &mut self.title,
            &mut self.code_word,
            &mut self.notes,
            &mut self.sold_as_other,
            &mut self.colour_other,
            &mut self.texture_other,
            &mut self.shape_other,
            &mut self.effects_other,
        ] {
            *value = flat_text(value);
        }
        for row in &mut self.rows {
            row.other = flat_text(&row.other);
        }
    }

    pub(super) fn to_sample(&self) -> Result<Sample> {
        let [fentanyl, benzodiazepines, medetomidine, lysergamide] =
            self.strips.map(StripResult::from);
        Ok(Sample {
            id: self.id,
            submission_number: self.title.clone(),
            visit: parse_visit(&self.visit).ok_or("Enter a positive Visit number.")?,
            site: self.site.into(),
            analysis_date: valid_date(&self.analysis_date).ok_or("Invalid analysis date.")?,
            collection_date: valid_date(&self.collection_date)
                .ok_or("Enter a date as YYYY-MM-DD.")?,
            expected_substance: self.expected.ok_or("Choose an expected substance.")?.into(),
            sold_as_other: optional_text(&self.sold_as_other),
            results: self
                .rows
                .iter()
                .filter_map(|row| row.result().transpose())
                .collect::<Result<_>>()?,
            colour: self.colour.clone(),
            colour_other: optional_text(&self.colour_other),
            texture: self.texture.clone(),
            texture_other: optional_text(&self.texture_other),
            pill_shape: optional_text(&self.shape),
            pill_shape_other: optional_text(&self.shape_other),
            consumption: match self.used {
                None => Consumption::Unanswered,
                Some(false) => Consumption::Pre,
                Some(true) => Consumption::Post,
            },
            adverse_events: match self.effects {
                None => AdverseEvents::Unanswered,
                Some(false) => AdverseEvents::No,
                Some(true) => AdverseEvents::Yes,
            },
            adverse_events_other: optional_text(&self.effects_other),
            strips: Strips {
                fentanyl,
                benzodiazepines,
                medetomidine,
                lysergamide,
            },
            code_word: self.code_word.clone(),
            notes: optional_text(&self.notes),
            photo: self.photo.as_ref().map(PhotoCopy::prepare).transpose()?,
        })
    }
}

pub(super) fn parse_visit(value: &str) -> Option<u32> {
    let value = value.trim();
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|visit| (1..=2_147_483_647).contains(visit))
}

pub(super) fn valid_date(value: &str) -> Option<Date> {
    let date: Date = value.parse().ok()?;
    (date.year() >= 1000 && date.to_string() == value).then_some(date)
}

pub(super) fn photo_ext(name: &str) -> Option<&str> {
    let (_, extension) = name.rsplit_once('.')?;
    PHOTO_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        .then_some(extension)
}

pub(super) fn pill_texture(value: &str) -> bool {
    matches!(value, "Tablet" | "Pressed Pill")
}

pub(super) fn flat_text(value: &str) -> String {
    value
        .replace("\r\n", " ")
        .replace(['\r', '\n', '\t', '\u{2028}', '\u{2029}'], " ")
        .trim_matches(' ')
        .to_owned()
}

fn optional_text(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

pub(super) fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

pub(super) fn filename_stem(title: &str) -> String {
    let base = title
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix)
                .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
        });
    if title.len() <= 80
        && !title.ends_with(['.', ' '])
        && !reserved
        && title
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && title
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b" _.-".contains(&byte))
    {
        title.to_owned()
    } else {
        format!("sample-{}", digest(title.as_bytes()))
    }
}

pub(super) fn invalid(message: &'static str) -> Box<dyn std::error::Error + Send + Sync> {
    io::Error::new(io::ErrorKind::InvalidData, message).into()
}
