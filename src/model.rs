use jiff::civil::Date;
use serde::Serialize;
use uuid::{Builder, Uuid};

use crate::{ExportError, Photo};

/// The current default location code for both sites. Store it as text.
pub const DEFAULT_SITE_CODE: &str = "901";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub enum CollectionSite {
    #[serde(rename = "fixture-east")]
    ExampleEast,
    #[serde(rename = "fixture-west")]
    ExampleWest,
}

#[derive(Clone, Copy)]
pub enum Substance {
    Catalogue(u64),
    Unknown,
    Other,
}

#[derive(Clone, Copy)]
pub struct Percentage {
    pub low: u8,
    pub high: u8,
}

#[derive(Clone)]
pub struct DetectedResult {
    pub substance: Substance,
    pub percentage: Option<Percentage>,
    pub other: Option<String>,
}

#[derive(Clone, Copy, Default, Serialize)]
pub enum StripResult {
    #[default]
    #[serde(rename = "Not Used")]
    NotUsed,
    Positive,
    Negative,
    Invalid,
}

#[derive(Clone, Copy, Default, Serialize)]
pub struct Strips {
    #[serde(rename = "fent_strip")]
    pub fentanyl: StripResult,
    #[serde(rename = "benzo")]
    pub benzodiazepines: StripResult,
    pub medetomidine: StripResult,
    #[serde(rename = "lysergamide_strip")]
    pub lysergamide: StripResult,
}

#[derive(Clone, Copy, Serialize)]
pub enum Consumption {
    #[serde(rename = "n/a")]
    Unanswered,
    #[serde(rename = "pre")]
    Pre,
    #[serde(rename = "post")]
    Post,
}

#[derive(Clone, Copy, Serialize)]
pub enum AdverseEvents {
    #[serde(rename = "na")]
    Unanswered,
    #[serde(rename = "no")]
    No,
    #[serde(rename = "yes")]
    Yes,
}

/// The selected sample and its exportable fields. Store the identity with the
/// sample when it is first created, and pass it back unchanged on re-export.
pub struct Sample {
    pub id: Uuid,
    pub submission_number: String,
    pub visit: u32,
    pub site: CollectionSite,
    pub analysis_date: Date,
    pub collection_date: Date,
    pub expected_substance: Substance,
    pub sold_as_other: Option<String>,
    pub results: Vec<DetectedResult>,
    pub colour: String,
    pub colour_other: Option<String>,
    pub texture: String,
    pub texture_other: Option<String>,
    pub pill_shape: Option<String>,
    pub pill_shape_other: Option<String>,
    pub consumption: Consumption,
    pub adverse_events: AdverseEvents,
    pub adverse_events_other: Option<String>,
    pub strips: Strips,
    pub code_word: String,
    pub notes: Option<String>,
    pub photo: Option<Photo>,
}

/// Creates a new sample's `UUIDv4`. Call once when creating the sample.
///
/// # Errors
/// Returns an error if the operating system cannot provide randomness.
pub fn new_sample_id() -> Result<Uuid, ExportError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes)?;
    Ok(Builder::from_random_bytes(bytes).into_uuid())
}

/// Supported new-entry texture tokens in the approved form's order.
#[must_use]
pub const fn live_textures() -> &'static [&'static str] {
    &[
        "Blotter",
        "Capsule",
        "Chunk",
        "Crystal",
        "Flake",
        "Granules",
        "Liquid",
        "Organic",
        "Paste",
        "Pebble",
        "Powder",
        "Pressed Pill",
        "Residue",
        "Tablet",
        "other",
    ]
}
