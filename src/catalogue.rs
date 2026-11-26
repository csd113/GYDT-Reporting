use std::collections::BTreeMap;

use roxmltree::Document;

use crate::{ExportError, sha256};

const XML: &str = include_str!("../data/substances-catalog.xml");
const WXR: &str = "http://wordpress.org/export/1.2/";
pub(crate) const VERSION: &str =
    "sha256:501794d785f8719a385dda374e4a47d81792d19135f26e863f214d9d6c4b86f8";

pub struct Catalogue {
    records: BTreeMap<u64, SubstanceRecord>,
    version: String,
}

pub struct SubstanceRecord {
    pub id: u64,
    pub label: String,
    pub slug: String,
    pub categories: Vec<Category>,
}

pub struct Category {
    pub domain: String,
    pub slug: String,
    pub label: String,
}

impl Catalogue {
    /// Loads the approved bundled catalogue, preserving IDs and category memberships.
    ///
    /// # Errors
    /// Returns an error if the snapshot, its fields, or its record count are invalid.
    pub fn bundled() -> Result<Self, ExportError> {
        let digest = sha256(XML.as_bytes());
        if VERSION.strip_prefix("sha256:") != Some(digest.as_str()) {
            return Err(ExportError::Catalogue);
        }
        let catalogue = Self::parse(XML, &digest)?;
        if catalogue.records.len() != 3 {
            return Err(ExportError::Catalogue);
        }
        Ok(catalogue)
    }

    pub(crate) fn parse(xml: &str, digest: &str) -> Result<Self, ExportError> {
        let doc = Document::parse(xml).map_err(|_| ExportError::Catalogue)?;
        let mut records = BTreeMap::new();
        for item in doc.descendants().filter(|node| node.has_tag_name("item")) {
            let field = |name| {
                item.children()
                    .find(|node| node.has_tag_name((WXR, name)))
                    .and_then(|node| node.text())
                    .ok_or(ExportError::Catalogue)
            };
            let id = field("post_id")?
                .parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(ExportError::Catalogue)?;
            if field("post_type")? != "substances" {
                return Err(ExportError::Catalogue);
            }
            let label = item
                .children()
                .find(|node| node.has_tag_name("title"))
                .and_then(|node| node.text())
                .ok_or(ExportError::Catalogue)?;
            let categories = item
                .children()
                .filter(|node| node.has_tag_name("category"))
                .map(|node| {
                    Ok(Category {
                        domain: node
                            .attribute("domain")
                            .ok_or(ExportError::Catalogue)?
                            .into(),
                        slug: node
                            .attribute("nicename")
                            .ok_or(ExportError::Catalogue)?
                            .into(),
                        label: node.text().ok_or(ExportError::Catalogue)?.into(),
                    })
                })
                .collect::<Result<Vec<_>, ExportError>>()?;
            let record = SubstanceRecord {
                id,
                label: label.into(),
                slug: field("post_name")?.into(),
                categories,
            };
            if records.insert(id, record).is_some() {
                return Err(ExportError::Catalogue);
            }
        }
        if records.is_empty() || records.len() > 10_000 {
            return Err(ExportError::Catalogue);
        }
        Ok(Self {
            records,
            version: format!("sha256:{digest}"),
        })
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub fn get(&self, id: u64) -> Option<&SubstanceRecord> {
        self.records.get(&id)
    }

    pub fn records(&self) -> impl ExactSizeIterator<Item = &SubstanceRecord> {
        self.records.values()
    }
}
