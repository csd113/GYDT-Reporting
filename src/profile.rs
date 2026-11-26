use std::{
    fs::{self, File},
    io::Read as _,
    path::Path,
};

use serde::Deserialize;

use crate::{Catalogue, CollectionSite, ExportError, draft, sha256};

/// Private, locally supplied configuration for one Results workflow.
pub struct Profile {
    catalogue: Catalogue,
    sites: [SiteDefinition; 2],
    pub(crate) templates: draft::Templates,
    storage_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    format_version: u8,
    profile_id: String,
    catalogue_sha256: String,
    sites: [SiteDefinition; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteDefinition {
    label: String,
    token: String,
    code: String,
}

impl Profile {
    /// Loads the four fixed profile files. No environment, network, or default
    /// staff directory is consulted.
    ///
    /// # Errors
    /// Returns an error for a missing or invalid configuration or input.
    pub fn load(directory: &Path) -> Result<Self, ExportError> {
        if !fs::symlink_metadata(directory).is_ok_and(|file| file.is_dir()) {
            return Err(ExportError::Profile(
                "Place the privately supplied GYDT Profile folder beside the app, then reopen it.",
            ));
        }
        let config: Configuration =
            serde_json::from_str(&read(directory, "profile.json", 16 * 1024)?)
                .map_err(|_| ExportError::Profile("The profile configuration is invalid."))?;
        let [first, second] = &config.sites;
        if config.format_version != 1
            || !slug(&config.profile_id)
            || !site(first)
            || !site(second)
            || first.token == second.token
            || first.label == second.label
        {
            return Err(ExportError::Profile(
                "The profile configuration is invalid.",
            ));
        }
        let xml = read(directory, "substances-catalog.xml", 8 * 1024 * 1024)?;
        if config.catalogue_sha256 != sha256(xml.as_bytes()) {
            return Err(ExportError::Catalogue);
        }
        let catalogue = Catalogue::parse(&xml, &config.catalogue_sha256)?;
        let templates = draft::Templates::load(
            &read(directory, "results-email.eml", 256 * 1024)?,
            &read(directory, "results-sms.txt", 64 * 1024)?,
        )?;
        let storage_key =
            sha256(format!("{}\0{}\0{}", config.profile_id, first.token, second.token).as_bytes());
        Ok(Self {
            catalogue,
            sites: config.sites,
            templates,
            storage_key,
        })
    }

    #[must_use]
    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogue
    }

    #[must_use]
    pub fn site(&self, site: CollectionSite) -> &SiteDefinition {
        let [first, second] = &self.sites;
        match site {
            CollectionSite::ExampleEast => first,
            CollectionSite::ExampleWest => second,
        }
    }

    #[must_use]
    pub fn collection_site(&self, token: &str) -> Option<CollectionSite> {
        [CollectionSite::ExampleEast, CollectionSite::ExampleWest]
            .into_iter()
            .find(|&site| self.site(site).token() == token)
    }

    #[must_use]
    pub fn storage_key(&self) -> &str {
        &self.storage_key
    }
}

impl SiteDefinition {
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn site(site: &SiteDefinition) -> bool {
    slug(&site.token)
        && !site.label.is_empty()
        && site.label.len() <= 120
        && site.label.trim() == site.label
        && !site.label.chars().any(char::is_control)
        && site.code.len() == 3
        && site.code.bytes().all(|b| b.is_ascii_digit())
}

fn read(directory: &Path, name: &'static str, limit: u64) -> Result<String, ExportError> {
    let path = directory.join(name);
    let error = || ExportError::ProfileFile(name);
    if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(error());
    }
    let file = File::open(path).map_err(|_| error())?;
    let meta = file.metadata().map_err(|_| error())?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > limit {
        return Err(error());
    }
    let mut value = String::new();
    file.take(limit + 1)
        .read_to_string(&mut value)
        .map_err(|_| error())?;
    if value.len() as u64 != meta.len() {
        return Err(error());
    }
    Ok(value)
}
