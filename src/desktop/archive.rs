use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Cursor, Read as _},
    ops::Range,
    path::Path,
};

use gydt_results_compiler::{Catalogue, Profile, live_textures};
use jiff::{Timestamp, tz::TimeZone};
use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, Visitor},
};
use uuid::{Uuid, Variant, Version};
use zip::{CompressionMethod, ZipArchive, read::ZipFile};

use super::model::{
    COLOURS, Channel, Draft, MAX_JPEG, PhotoCopy, Result, ResultRow, SHAPES, Selection, Site,
    Strip, digest, filename_stem, flat_text, invalid, parse_range, photo_ext, pill_texture,
    valid_date,
};

const MANIFEST: &str = "manifest.json";
const DATA: &str = "test_data.json";
const MAX_ZIP: u64 = 5 * 1024 * 1024;
pub(super) fn read_file(path: &Path, max: u64, description: &str) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(format!("Choose a regular {description} file.").into());
    }
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    if size == 0 {
        return Err(format!("The {description} is empty.").into());
    }
    if size > max {
        return Err(format!("The {description} exceeds {} MiB.", max / (1024 * 1024)).into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Err(format!("The {description} changed while reading it. Wait for it to finish saving, then try again.").into());
    }
    Ok(bytes)
}

pub(super) fn reopen(path: &Path, profile: &Profile) -> Result<Draft> {
    let bytes = read_file(path, MAX_ZIP, "ZIP")?;
    let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice()))
        .map_err(|_| invalid("This file is an invalid or unsupported sample ZIP."))?;
    validate_archive(&mut archive, &bytes)?;
    let members = read_members(&mut archive)?;
    let manifest: Manifest = serde_json::from_slice(member(&members, MANIFEST)?)?;
    let id = manifest.validate(&members, profile.catalogue())?;
    let payload: Payload = serde_json::from_slice(member(&members, DATA)?)?;
    let stem = filename_stem(&payload.submission_number);
    let photo = members
        .iter()
        .find(|(name, _)| *name != MANIFEST && *name != DATA)
        .map(|(name, bytes)| {
            let extension = photo_ext(name).ok_or("The package photo must be a JPEG.")?;
            if name != &format!("{stem}.{extension}") {
                return Err("The sample photo name doesn't match the submission number.".into());
            }
            let photo = PhotoCopy::new(name, extension, bytes);
            photo.prepare()?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(photo)
        })
        .transpose()?;
    payload.draft(id, profile, photo)
}

fn read_members<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut members = BTreeMap::new();
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name()?.into_owned();
        if file.name_raw() != name.as_bytes() || !names.insert(name.to_ascii_lowercase()) {
            return Err("The ZIP includes a duplicate or non-ASCII member name.".into());
        }
        let limit = match name.as_str() {
            MANIFEST => 8 * 1024,
            DATA => 64 * 1024,
            _ if name.is_ascii() && !name.contains(['/', '\\']) && photo_ext(&name).is_some() => {
                MAX_JPEG
            }
            _ => return Err("The ZIP contains an unexpected file.".into()),
        };
        if limit == MAX_JPEG
            && members
                .keys()
                .any(|name: &String| photo_ext(name).is_some())
        {
            return Err("The ZIP contains more than one sample photo.".into());
        }
        let size = file.size();
        if size == 0 || size > limit {
            return Err(format!("{name} exceeds its size limit or is empty.").into());
        }
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size {
            return Err(format!("{name} does not match its declared size.").into());
        }
        members.insert(name, bytes);
    }
    Ok(members)
}

fn member<'a>(members: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a [u8]> {
    members
        .get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("The ZIP is missing {name}.").into())
}

fn validate_archive(archive: &mut ZipArchive<Cursor<&[u8]>>, bytes: &[u8]) -> Result<()> {
    if !(2..=3).contains(&archive.len())
        || archive.offset() != 0
        || archive.raw_zip64_extensible_data_sector().is_some()
        || archive.has_duplicate_names()
        || archive.has_overlapping_entries()?
    {
        return Err("Choose a format-1 sample ZIP with two or three distinct files.".into());
    }
    let central = usize::try_from(archive.central_directory_start())?;
    let footer_start = bytes
        .len()
        .checked_sub(22 + archive.comment().len())
        .ok_or_else(bad_headers)?;
    let footer = bytes.get(footer_start..).ok_or_else(bad_headers)?;
    let count = u16::try_from(archive.len())?;
    if !footer.starts_with(b"PK\x05\x06")
        || short(footer, 4)? != 0
        || short(footer, 6)? != 0
        || short(footer, 8)? != count
        || short(footer, 10)? != count
        || long(footer, 16)? as usize != central
        || central.checked_add(long(footer, 12)? as usize) != Some(footer_start)
        || usize::from(short(footer, 20)?) != archive.comment().len()
    {
        return Err(bad_headers());
    }
    let mut local_ranges = Vec::new();
    let mut central_ranges = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index)?;
        let (local, directory) = validate_headers(&file, bytes)?;
        local_ranges.push(local);
        central_ranges.push(directory);
    }
    if !covers(local_ranges, 0..central) || !covers(central_ranges, central..footer_start) {
        return Err(bad_headers());
    }
    Ok(())
}

fn covers(mut ranges: Vec<Range<usize>>, target: Range<usize>) -> bool {
    ranges.sort_by_key(|range| range.start);
    let mut position = target.start;
    for range in ranges {
        if range.start != position || range.end <= position {
            return false;
        }
        position = range.end;
    }
    position == target.end
}

fn validate_headers<R: std::io::Read>(
    file: &ZipFile<'_, R>,
    bytes: &[u8],
) -> Result<(Range<usize>, Range<usize>)> {
    if !file.is_file()
        || file
            .unix_mode()
            .is_some_and(|mode| !matches!(mode & 0o170_000, 0 | 0o100_000))
        || !matches!(
            file.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        )
    {
        return Err("The ZIP can only contain regular files using Stored or Deflate.".into());
    }
    let start = usize::try_from(file.header_start())?;
    let central_start = usize::try_from(file.central_header_start())?;
    let local = bytes.get(start..).ok_or_else(bad_headers)?;
    let central = bytes.get(central_start..).ok_or_else(bad_headers)?;
    let name = file.name_raw();
    let flags = short(local, 6)?;
    let method = short(local, 8)?;
    if !local.starts_with(b"PK\x03\x04")
        || !central.starts_with(b"PK\x01\x02")
        || flags & !0x080e != 0
        || flags != short(central, 8)?
        || method != short(central, 10)?
        || short(central, 34)? != 0
        || long(central, 42)? as usize != start
        || short(local, 4)? != short(central, 6)?
        || short(local, 4)? > 20
        || usize::from(short(local, 26)?) != name.len()
        || usize::from(short(central, 28)?) != name.len()
        || local.get(30..30 + name.len()) != Some(name)
        || central.get(46..46 + name.len()) != Some(name)
    {
        return Err(bad_headers());
    }
    let crc = file.crc32();
    let compressed = u32::try_from(file.compressed_size())?;
    let length = u32::try_from(file.size())?;
    let values = [crc, compressed, length];
    if [long(central, 16)?, long(central, 20)?, long(central, 24)?] != values {
        return Err(bad_headers());
    }
    let header_values = [long(local, 14)?, long(local, 18)?, long(local, 22)?];
    let local_header_end = extra_end(local, 30 + name.len(), 28)?;
    let data_end = start + local_header_end + compressed as usize;
    let end = if flags & 8 == 0 {
        if header_values != values {
            return Err(bad_headers());
        }
        data_end
    } else {
        if header_values != [0; 3] && header_values != values {
            return Err(bad_headers());
        }
        descriptor_end(bytes, data_end, values)?
    };
    let central_end =
        central_start + extra_end(central, 46 + name.len(), 30)? + usize::from(short(central, 32)?);
    Ok((start..end, central_start..central_end))
}

fn descriptor_end(bytes: &[u8], start: usize, values: [u32; 3]) -> Result<usize> {
    let descriptor = bytes.get(start..).ok_or_else(bad_headers)?;
    for offset in [0, 4] {
        if (offset == 0 || descriptor.starts_with(b"PK\x07\x08"))
            && [
                long(descriptor, offset)?,
                long(descriptor, offset + 4)?,
                long(descriptor, offset + 8)?,
            ] == values
        {
            return Ok(start + offset + 12);
        }
    }
    Err(bad_headers())
}

fn extra_end(header: &[u8], start: usize, length_at: usize) -> Result<usize> {
    let end = start + usize::from(short(header, length_at)?);
    let mut extra = header.get(start..end).ok_or_else(bad_headers)?;
    while !extra.is_empty() {
        if short(extra, 0)? == 1 {
            return Err(bad_headers());
        } // ZIP64 extended information.
        extra = extra
            .get(4 + usize::from(short(extra, 2)?)..)
            .ok_or_else(bad_headers)?;
    }
    Ok(end)
}

fn bad_headers() -> Box<dyn std::error::Error + Send + Sync> {
    invalid("The ZIP has invalid, inconsistent, or unsupported headers.")
}

fn short(bytes: &[u8], start: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(start..start + 2)
            .ok_or_else(bad_headers)?
            .try_into()?,
    ))
}

fn long(bytes: &[u8], start: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(start..start + 4)
            .ok_or_else(bad_headers)?
            .try_into()?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u8,
    sample_id: String,
    exported_at: String,
    catalogue_version: String,
    #[serde(deserialize_with = "unique_files")]
    files: BTreeMap<String, Member>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    bytes: usize,
    sha256: String,
}

fn unique_files<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, Member>, D::Error> {
    struct Files;
    impl<'de> Visitor<'de> for Files {
        type Value = BTreeMap<String, Member>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of distinct sample files")
        }

        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut files = BTreeMap::new();
            while let Some((name, member)) = map.next_entry()? {
                if files.insert(name, member).is_some() {
                    return Err(A::Error::custom("duplicate manifest file"));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(Files)
}

impl Manifest {
    fn validate(&self, members: &BTreeMap<String, Vec<u8>>, catalogue: &Catalogue) -> Result<Uuid> {
        if self.format_version != 1 {
            return Err("This ZIP format version is unsupported.".into());
        }
        if self.catalogue_version != catalogue.version() {
            return Err("This ZIP uses a different substance catalogue.".into());
        }
        let id: Uuid = self.sample_id.parse()?;
        if id.get_version() != Some(Version::Random)
            || id.get_variant() != Variant::RFC4122
            || id.to_string() != self.sample_id
        {
            return Err("The ZIP has an invalid sample ID.".into());
        }
        let timestamp: Timestamp = self.exported_at.parse()?;
        if timestamp.to_zoned(TimeZone::UTC).year() < 1
            || timestamp.strftime("%Y-%m-%dT%H:%M:%SZ").to_string() != self.exported_at
        {
            return Err("The ZIP has an invalid export timestamp.".into());
        }
        if self.files.len() + 1 != members.len() || self.files.contains_key(MANIFEST) {
            return Err("The ZIP files do not match the manifest.".into());
        }
        for (name, entry) in &self.files {
            let bytes = member(members, name)?;
            if entry.bytes != bytes.len() || entry.sha256 != digest(bytes) {
                return Err(format!("{name} does not match the manifest checksum or size.").into());
            }
        }
        Ok(id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    submission_number: String,
    visit: u32,
    site: String,
    analysis_date: String,
    collection_date: String,
    expected_substance: String,
    #[serde(default, deserialize_with = "optional_string")]
    sold_as_other: Option<String>,
    results: Vec<Detection>,
    colour: String,
    #[serde(default, deserialize_with = "optional_string")]
    colour_other: Option<String>,
    texture: String,
    #[serde(default, deserialize_with = "optional_string")]
    texture_other: Option<String>,
    #[serde(default, deserialize_with = "optional_string")]
    pill_shape: Option<String>,
    #[serde(default, deserialize_with = "optional_string")]
    pill_shape_other: Option<String>,
    pre_post_consumption: String,
    adverse_events: String,
    #[serde(default, deserialize_with = "optional_string")]
    advertse_events_yes_other: Option<String>,
    fent_strip: Strip,
    benzo: Strip,
    medetomidine: Strip,
    lysergamide_strip: Strip,
    client: String,
    #[serde(default, deserialize_with = "optional_string")]
    notes: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Detection {
    substance: String,
    percentage: String,
    #[serde(default, deserialize_with = "optional_string")]
    other: Option<String>,
}

fn optional_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl Payload {
    fn draft(self, id: Uuid, profile: &Profile, photo: Option<PhotoCopy>) -> Result<Draft> {
        let catalogue = profile.catalogue();
        let site = profile
            .collection_site(&self.site)
            .map(Site::from)
            .ok_or("This ZIP uses a collection location outside the current profile.")?;
        self.validate_details()?;
        let expected = checked_substance(&self.expected_substance, catalogue)?;
        let rows = self
            .results
            .into_iter()
            .map(|row| row.into_row(catalogue))
            .collect::<Result<_>>()?;
        let used = choice_answer(&self.pre_post_consumption, ["n/a", "pre", "post"])?;
        let effects = choice_answer(&self.adverse_events, ["na", "no", "yes"])?;
        if used.is_none() && effects.is_some() {
            return Err("The ZIP answers unexpected effects without answering sample use.".into());
        }
        Ok(Draft {
            id,
            title: self.submission_number,
            suggested_title: None,
            visit: self.visit.to_string(),
            suggested_visit: None,
            visit_claims: [0; 2],
            site,
            analysis_date: self.analysis_date,
            collection_date: self.collection_date,
            expected: Some(expected),
            sold_as_other: self.sold_as_other.unwrap_or_default(),
            rows,
            colour: self.colour,
            colour_other: self.colour_other.unwrap_or_default(),
            texture: self.texture,
            texture_other: self.texture_other.unwrap_or_default(),
            shape: self.pill_shape.unwrap_or_default(),
            shape_other: self.pill_shape_other.unwrap_or_default(),
            used,
            effects,
            effects_other: self.advertse_events_yes_other.unwrap_or_default(),
            strips: [
                self.fent_strip,
                self.benzo,
                self.medetomidine,
                self.lysergamide_strip,
            ],
            code_word: self.client,
            notes: self.notes.unwrap_or_default(),
            photo,
            channel: Channel::None,
            address: String::new(),
            exported: true,
        })
    }

    fn validate_details(&self) -> Result<()> {
        checked_text(&self.submission_number, 200, "submission number")?;
        checked_text(&self.client, 255, "code word")?;
        validate_optional(self.notes.as_deref(), true, 2000, "notes")?;
        let collection =
            valid_date(&self.collection_date).ok_or("The ZIP has an invalid collection date.")?;
        let zone = TimeZone::get("America/Vancouver")?;
        zone.to_ambiguous_zoned(collection.at(0, 0, 0, 0))
            .unambiguous()?;
        if valid_date(&self.analysis_date).is_none()
            || !(1..=2_147_483_647).contains(&self.visit)
            || !(1..=10).contains(&self.results.len())
        {
            return Err(
                "The ZIP has an invalid analysis date, Visit number, or detected result count."
                    .into(),
            );
        }
        if !COLOURS.contains(&self.colour.as_str())
            || !live_textures().contains(&self.texture.as_str())
        {
            return Err("The ZIP has an unsupported colour or texture.".into());
        }
        if let Some(shape) = &self.pill_shape
            && (!pill_texture(&self.texture) || !SHAPES.iter().any(|(choice, _)| choice == shape))
        {
            return Err("The ZIP has an invalid pill shape.".into());
        }
        for (value, allowed, field) in [
            (
                self.sold_as_other.as_deref(),
                self.expected_substance == "other",
                "Sold As details",
            ),
            (
                self.colour_other.as_deref(),
                self.colour == "other",
                "colour details",
            ),
            (
                self.texture_other.as_deref(),
                self.texture == "other",
                "texture details",
            ),
            (
                self.pill_shape_other.as_deref(),
                self.pill_shape.as_deref() == Some("other"),
                "shape details",
            ),
            (
                self.advertse_events_yes_other.as_deref(),
                self.adverse_events == "yes",
                "unexpected effects details",
            ),
        ] {
            validate_optional(value, allowed, 255, field)?;
        }
        Ok(())
    }
}

impl Detection {
    fn into_row(self, catalogue: &Catalogue) -> Result<ResultRow> {
        let substance = checked_substance(&self.substance, catalogue)?;
        validate_optional(
            self.other.as_deref(),
            substance == Selection::Other,
            255,
            "detected substance details",
        )?;
        if self.percentage == "blank" {
            return Ok(ResultRow {
                substance: Some(substance),
                other: self.other.unwrap_or_default(),
                ..ResultRow::default()
            });
        }
        let range =
            parse_range(&self.percentage).ok_or("The ZIP has an invalid detected percentage.")?;
        Ok(ResultRow {
            substance: Some(substance),
            custom: !(range.low >= 5 && range.low.is_multiple_of(5) && range.high - range.low == 5),
            range: self.percentage,
            other: self.other.unwrap_or_default(),
        })
    }
}

fn checked_substance(token: &str, catalogue: &Catalogue) -> Result<Selection> {
    let substance = Selection::parse(token).ok_or("The ZIP has an invalid substance.")?;
    if let Selection::Catalogue(id) = substance
        && catalogue.get(id).is_none()
    {
        return Err("The ZIP contains a substance absent from the bundled catalogue.".into());
    }
    Ok(substance)
}

fn checked_text(value: &str, limit: usize, field: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > limit
        || value != flat_text(value)
        || value
            .chars()
            .any(|char| char.is_control() || matches!(char, '<' | '>'))
    {
        return Err(format!("The ZIP contains invalid {field}.").into());
    }
    Ok(())
}

fn validate_optional(value: Option<&str>, allowed: bool, limit: usize, field: &str) -> Result<()> {
    if let Some(value) = value {
        if !allowed {
            return Err(format!("The ZIP includes {field} outside its allowed selection.").into());
        }
        checked_text(value, limit, field)?;
    }
    Ok(())
}

fn choice_answer(value: &str, [blank, no, yes]: [&str; 3]) -> Result<Option<bool>> {
    if value == blank {
        Ok(None)
    } else if value == no {
        Ok(Some(false))
    } else if value == yes {
        Ok(Some(true))
    } else {
        Err("The ZIP has an invalid sample-use or unexpected-effects answer.".into())
    }
}
