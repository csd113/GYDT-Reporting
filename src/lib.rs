//! Offline v0.1 sample export for the desktop workflow.

mod catalogue;
mod draft;
mod files;
mod model;
mod package;
mod photo;
mod profile;

use std::path::PathBuf;

pub use catalogue::{Catalogue, Category, SubstanceRecord};
pub use files::{
    CleanupWarning, ConflictSnapshot, RecoveryRequired, WriteError, WriteFailure, WriteSuccess,
};
pub use model::{
    AdverseEvents, CollectionSite, Consumption, DEFAULT_SITE_CODE, DetectedResult, Percentage,
    Sample, StripResult, Strips, Substance, live_textures, new_sample_id,
};
pub use package::{
    Contact, ExportHistoryEntry, ExportRequest, PreparedExport, prepare_export,
    prepare_profile_export,
};
pub use photo::Photo;
pub use profile::{Profile, SiteDefinition};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("Invalid {field}. {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("Sample {number}. {error}")]
    Sample {
        number: usize,
        #[source]
        error: Box<Self>,
    },
    #[error("{0}")]
    Profile(&'static str),
    #[error(
        "The profile file {0} is missing, unreadable, or invalid. Use the privately supplied profile."
    )]
    ProfileFile(&'static str),
    #[error("The substance catalogue does not match the approved profile snapshot")]
    Catalogue,
    #[error("Sample number {0} has already been used")]
    DuplicateSubmission(String),
    #[error("Sample number {0} requires explicit re-export confirmation")]
    ReexportConfirmation(String),
    #[error(
        "Select samples from the same testing site, analysis date, and Visit number for one client draft. Use WordPress-only export for a mixed selection."
    )]
    MixedContactVisit,
    #[error("More than one output would use the filename {0}")]
    OutputNameConflict(String),
    #[error("Unable to read JPEG {path}")]
    PhotoRead {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("Unable to generate randomness")]
    Random(#[from] getrandom::Error),
    #[error("The client draft template has an unexpected format")]
    Template,
    #[error("Unable to serialize the export")]
    Json(#[from] serde_json::Error),
    #[error("Unable to build the WordPress archive")]
    Zip(#[from] zip::result::ZipError),
    #[error("Unable to write the WordPress archive")]
    ZipIo(#[from] std::io::Error),
}

fn invalid(field: &'static str, reason: &'static str) -> ExportError {
    ExportError::InvalidField { field, reason }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}
