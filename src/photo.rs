use std::{
    borrow::Cow,
    fs::{self, File},
    io::Read as _,
    ops::Range,
    path::Path,
};

use gamut_core::{Cmyk8, DecodeImage, Gray8, Rgb8};

use crate::{ExportError, invalid};

pub(crate) const MAX_PHOTO: usize = 4 * 1024 * 1024;
const MAX_SIDE: u32 = 4096;
const MAX_PIXELS: u32 = 8_388_608;

pub struct Photo {
    pub(crate) bytes: Vec<u8>,
}

impl Photo {
    /// Reads and validates a JPEG for byte-for-byte inclusion in the export.
    ///
    /// # Errors
    /// Returns an error for unsupported extensions, unreadable images, or images
    /// outside the format's byte or pixel limits.
    pub fn from_path(path: &Path) -> Result<Self, ExportError> {
        let extension = path
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("jpg") && !extension.eq_ignore_ascii_case("jpeg") {
            return Err(invalid("photo", "Choose a .jpg or .jpeg file."));
        }
        let mut bytes = Vec::new();
        let read_error = |error| ExportError::PhotoRead {
            path: path.into(),
            error,
        };
        if !fs::symlink_metadata(path).map_err(read_error)?.is_file() {
            return Err(invalid("photo", "Choose a regular JPEG file."));
        }
        File::open(path)
            .map_err(read_error)?
            .take(MAX_PHOTO as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(read_error)?;
        Self::from_jpeg_bytes(extension, bytes)
    }

    /// Validates an owned .jpg or .jpeg snapshot without reopening a file.
    ///
    /// # Errors
    /// Returns an error for unsupported extensions, unreadable JPEG bytes, or
    /// images outside the format's byte or pixel limits.
    pub fn from_jpeg_bytes(extension: &str, bytes: Vec<u8>) -> Result<Self, ExportError> {
        if !extension.eq_ignore_ascii_case("jpg") && !extension.eq_ignore_ascii_case("jpeg") {
            return Err(invalid("photo", "Choose a .jpg or .jpeg file."));
        }
        if bytes.len() > MAX_PHOTO {
            return Err(invalid("photo", "Choose a JPEG up to 4 MiB."));
        }
        validate(&bytes, Metadata::Camera)?;
        Ok(Self { bytes })
    }

    /// Returns the original bytes without modifying or re-encoding them.
    #[must_use]
    pub fn into_jpeg_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Prepare only the ZIP member. Keep the source snapshot and its scan bytes intact.
    pub(crate) fn for_import(&self) -> Result<Cow<'_, [u8]>, ExportError> {
        let mut seen_jfif = false;
        let mut copy: Option<Vec<u8>> = None;
        let mut next = 0;
        check_segments(&self.bytes, |marker, payload, range| {
            if disallowed_metadata(marker, payload, &mut seen_jfif) {
                let output = copy.get_or_insert_with(|| Vec::with_capacity(self.bytes.len()));
                output.extend_from_slice(self.bytes.get(next..range.start)?);
                next = range.end;
            }
            Some(())
        })
        .ok_or_else(framing_error)?;

        let bytes = if let Some(mut copy) = copy {
            copy.extend_from_slice(self.bytes.get(next..).ok_or_else(framing_error)?);
            Cow::Owned(copy)
        } else {
            Cow::Borrowed(self.bytes.as_slice())
        };
        validate(&bytes, Metadata::Importer)?;
        Ok(bytes)
    }
}

#[derive(Clone, Copy)]
enum Metadata {
    Camera,
    Importer,
}

fn framing_error() -> ExportError {
    invalid(
        "photo",
        "The JPG/JPEG is incomplete, has invalid structure, or has bytes after its end marker.",
    )
}

fn validate(bytes: &[u8], metadata: Metadata) -> Result<(), ExportError> {
    let incomplete = || invalid("photo", "Choose a complete, readable JPG/JPEG.");
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(invalid(
            "photo",
            "The selected file is not a JPG/JPEG image.",
        ));
    }
    let mut seen_jfif = false;
    let frame = check_segments(bytes, |marker, data, _| match metadata {
        Metadata::Camera => Some(()),
        Metadata::Importer => (!disallowed_metadata(marker, data, &mut seen_jfif)).then_some(()),
    })
    .ok_or_else(framing_error)?;
    let (width, height) = (u32::from(frame.width), u32::from(frame.height));
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || width * height > MAX_PIXELS
    {
        return Err(invalid(
            "photo",
            "The JPEG exceeds the supported dimensions.",
        ));
    }
    if frame.arithmetic {
        // Gamut handles Huffman coding. WordPress still decodes and validates
        // arithmetic-coded JPEGs before it can create a draft.
        return Ok(());
    }
    // Decode for validation only. Retain the original bytes for packaging.
    let decoder = gamut_jpeg::JpegDecoder::new();
    match frame.components {
        1 => DecodeImage::<Gray8>::decode_image(&decoder, bytes).map(drop),
        3 => DecodeImage::<Rgb8>::decode_image(&decoder, bytes).map(drop),
        4 => DecodeImage::<Cmyk8>::decode_image(&decoder, bytes).map(drop),
        _ => return Err(incomplete()),
    }
    .map_err(|_| incomplete())
}

struct Frame {
    width: u16,
    height: u16,
    components: u8,
    arithmetic: bool,
}

fn read_frame(marker: u8, payload: &[u8]) -> Option<Frame> {
    let [8, yh, yl, xh, xl, components, sampling @ ..] = payload else {
        return None;
    };
    if !matches!(components, 1 | 3 | 4) || sampling.len() != usize::from(*components) * 3 {
        return None;
    }
    Some(Frame {
        width: u16::from_be_bytes([*xh, *xl]),
        height: u16::from_be_bytes([*yh, *yl]),
        components: *components,
        arithmetic: matches!(marker, 0xc9 | 0xca),
    })
}

// Walk each length-framed marker without decoding metadata or changing image
// data. The caller selects the camera or importer policy.
fn check_segments(
    mut bytes: &[u8],
    mut visit: impl FnMut(u8, &[u8], Range<usize>) -> Option<()>,
) -> Option<Frame> {
    let total = bytes.len();
    bytes = bytes.strip_prefix(&[0xff, 0xd8])?;
    let (mut in_scan, mut seen_scan) = (false, false);
    let mut frame = None;
    loop {
        if in_scan {
            let offset = bytes.iter().position(|byte| *byte == 0xff)?;
            bytes = bytes.get(offset..)?;
        }
        let start = total - bytes.len();
        let rest = bytes.strip_prefix(&[0xff])?;
        let offset = rest.iter().position(|byte| *byte != 0xff)?;
        let (&marker, rest) = rest.get(offset..)?.split_first()?;
        bytes = rest;
        match marker {
            0 | 0xd0..=0xd7 if in_scan => continue, // stuffed data or restart marker
            0xd9 if seen_scan && bytes.is_empty() => return frame,
            0..=1 | 0xc3 | 0xc5..=0xc7 | 0xcb | 0xcd..=0xcf | 0xd0..=0xd9 => return None,
            _ => {}
        }

        let [high, low, ..] = bytes else { return None };
        let length = usize::from(u16::from_be_bytes([*high, *low]));
        let (segment, rest) = bytes.split_at_checked(length)?;
        let payload = segment.get(2..)?;
        visit(marker, payload, start..total - rest.len())?;
        if matches!(marker, 0xc0..=0xc2 | 0xc9 | 0xca) {
            if frame.is_some() || seen_scan {
                return None;
            }
            frame = Some(read_frame(marker, payload)?);
        }
        in_scan = marker == 0xda;
        if in_scan && frame.is_none() {
            return None;
        }
        seen_scan |= in_scan;
        bytes = rest;
    }
}

fn disallowed_metadata(marker: u8, payload: &[u8], seen_jfif: &mut bool) -> bool {
    // Match the installed importer's APPn/COM rule without parsing camera metadata.
    if marker == 0xe0 && !*seen_jfif && thumbnail_free_jfif(payload) {
        *seen_jfif = true;
        return false;
    }
    matches!(marker, 0xe0..=0xef | 0xfe)
}

fn thumbnail_free_jfif(payload: &[u8]) -> bool {
    matches!(payload.strip_prefix(b"JFIF\0"), Some([1, minor, unit, xh, xl, yh, yl, 0, 0])
        if *minor <= 2 && *unit <= 2 && (*xh != 0 || *xl != 0) && (*yh != 0 || *yl != 0))
}
