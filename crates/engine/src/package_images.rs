//! Images for document packages (DOCX, EPUB).
//!
//! A package embeds each distinct image once, sized for the largest place it
//! appears. The cheapest faithful form is chosen from the image dictionary
//! before anything is decoded:
//!
//! - a baseline JPEG (`DCTDecode`) is copied byte for byte as `.jpg`;
//! - a Flate stream whose PNG predictor rows match the image (8-bit gray or
//!   RGB, `/Columns` = `/Width`) is already a PNG `IDAT` stream: it is
//!   wrapped in PNG chunks without being inflated;
//! - anything else is decoded, at reduced resolution when a JPEG or JPEG 2000
//!   is far larger than its placed size needs, fitted to the placed size at
//!   [`PACKAGE_IMAGE_DPI`], and written as JPEG (opaque) or PNG (alpha or
//!   bilevel).
//!
//! An image is only downscaled when it is more than [`FAR_LARGER`] times the
//! size its largest placement needs, so passthrough stays the common case.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::engine::{ContentEngine, PlacedImageReference};
use crate::error::Result;
use crate::images::decoder::{box_downscale_to_fit, ImageDecoder};
use crate::images::encoder::ImageEncoder;
use crate::images::locator::ImageReference;
use crate::object::{PdfDictionary, PdfObject};

/// Resolution an embedded image is fitted to, for its placed size.
pub(crate) const PACKAGE_IMAGE_DPI: f64 = 200.0;
/// How much larger than needed an image must be before it is downscaled.
const FAR_LARGER: u32 = 2;

/// One encoded image and its file extension.
#[derive(Debug, Clone)]
pub(crate) struct PackagedImage {
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) ext: &'static str,
    /// `crate::versioning::resource_digest` of `bytes`.
    pub(crate) digest: String,
}

/// What a package format accepts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PackagePolicy {
    /// Copy every baseline JPEG, whatever its colour space (Word shows CMYK
    /// and inverted JPEGs; EPUB reading systems do not).
    pub(crate) any_jpeg: bool,
}

/// Each page's placed images (masks and soft masks excluded), in page order.
pub(crate) fn page_placements(
    engine: &ContentEngine,
    pages: &[(u32, f64, f64)],
) -> Vec<Vec<PlacedImageReference>> {
    crate::page_parallel::map_ordered(pages, |&(number, width, height)| {
        let Ok(region) = crate::PageRegion::new(0.0, 0.0, width.max(1.0), height.max(1.0)) else {
            return Vec::new();
        };
        engine
            .find_page_images_in_region(number as usize, region)
            .map(|images| {
                images
                    .into_iter()
                    .filter(|placed| !placed.image.is_mask && !placed.image.is_smask)
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Encode every distinct image of `placements` once, in parallel, for the
/// largest size it is placed at. Images that cannot be read are absent.
pub(crate) fn package_distinct(
    engine: &ContentEngine,
    placements: &[Vec<PlacedImageReference>],
    policy: PackagePolicy,
) -> BTreeMap<(u32, u16), PackagedImage> {
    let mut largest: BTreeMap<(u32, u16), (ImageReference, f64, f64)> = BTreeMap::new();
    for placed in placements.iter().flatten() {
        let key = (placed.image.object_number, placed.image.generation_number);
        let width = (placed.bbox[2] - placed.bbox[0]).abs();
        let height = (placed.bbox[3] - placed.bbox[1]).abs();
        let entry = largest
            .entry(key)
            .or_insert_with(|| (placed.image.clone(), 0.0, 0.0));
        entry.1 = entry.1.max(width);
        entry.2 = entry.2.max(height);
    }
    let distinct: Vec<_> = largest.into_iter().collect();
    let encoded = crate::page_parallel::map_ordered(&distinct, |(_, (image, width, height))| {
        package_image(engine, image, *width, *height, policy).ok()
    });
    distinct
        .into_iter()
        .zip(encoded)
        .filter_map(|((key, _), image)| image.map(|image| (key, image)))
        .collect()
}

fn target_pixels(points: f64) -> u32 {
    ((points / 72.0 * PACKAGE_IMAGE_DPI).ceil()).clamp(16.0, 65_535.0) as u32
}

/// Encode one image for a package; see the module docs for the choice.
pub(crate) fn package_image(
    engine: &ContentEngine,
    image: &ImageReference,
    placed_width: f64,
    placed_height: f64,
    policy: PackagePolicy,
) -> Result<PackagedImage> {
    let target_width = target_pixels(placed_width);
    let target_height = target_pixels(placed_height);
    let far_larger =
        image.width > target_width * FAR_LARGER || image.height > target_height * FAR_LARGER;
    let reader = engine.document().reader();
    let stream = if image.is_inline || image.object_number == 0 {
        None
    } else {
        match reader.get_object(image.object_number, image.generation_number)? {
            PdfObject::Stream { dict, raw } => Some((dict, raw)),
            _ => None,
        }
    };
    if let Some((dict, raw)) = &stream {
        let traits = ImageTraits::of(dict, reader);
        if traits.single_filter(&["DCTDecode", "DCT"]) {
            // Word takes any baseline JPEG as it is (the DOCX writer always
            // did). Elsewhere only an unmasked gray or RGB JPEG that is not
            // far larger than needed is copied.
            let plain = traits.unmasked() && matches!(traits.channels(), Some(1 | 3));
            if policy.any_jpeg || (plain && !far_larger) {
                return Ok(packaged(raw.clone(), "jpg"));
            }
        }
        if !far_larger {
            if let Some(png) = flate_png_passthrough(&traits, image, raw) {
                return Ok(packaged(png, "png"));
            }
        }
        let decoded = if far_larger && traits.unmasked() && traits.channels().is_some() {
            let limits = crate::filters::DecodeLimits::default();
            if traits.single_filter(&["JPXDecode", "JPX"]) {
                ImageDecoder::decode_jpx_scaled_with_limits(
                    image,
                    reader,
                    None,
                    target_width,
                    target_height,
                    &limits,
                )
                .ok()
            } else if traits.single_filter(&["DCTDecode", "DCT"]) {
                ImageDecoder::decode_jpeg_scaled_with_limits_and_color_transform_options(
                    image,
                    reader,
                    None,
                    target_width,
                    target_height,
                    &limits,
                    crate::images::decoder::ImageColorOptions::default(),
                    None,
                )
                .ok()
            } else {
                None
            }
        } else {
            None
        };
        let raw_image = match decoded {
            Some(raw_image) => raw_image,
            None => engine.decode_image(image)?,
        };
        return encode_fitted(raw_image, target_width, target_height);
    }
    encode_fitted(engine.decode_image(image)?, target_width, target_height)
}

fn packaged(bytes: Vec<u8>, ext: &'static str) -> PackagedImage {
    PackagedImage {
        digest: crate::versioning::resource_digest(&bytes),
        bytes: Arc::new(bytes),
        ext,
    }
}

/// Fit to the target and write JPEG (opaque) or PNG (alpha or bilevel).
fn encode_fitted(
    raw: crate::images::decoder::RawImage,
    width: u32,
    height: u32,
) -> Result<PackagedImage> {
    let raw = box_downscale_to_fit(raw, width, height);
    let bilevel = raw.channels == 1 && raw.pixels.iter().all(|&v| v == 0 || v == 255);
    if raw.channels == 4 || raw.channels == 2 || bilevel {
        Ok(packaged(ImageEncoder::encode_png_compact(&raw)?, "png"))
    } else {
        Ok(packaged(ImageEncoder::encode_jpeg(&raw, 85)?, "jpg"))
    }
}

/// The dictionary facts the passthrough decisions read.
struct ImageTraits {
    filters: Vec<String>,
    color_components: Option<u8>,
    decode_parms: Option<PdfDictionary>,
    has_mask: bool,
    has_decode: bool,
    image_mask: bool,
    bits: Option<i64>,
}

impl ImageTraits {
    fn of(dict: &PdfDictionary, reader: &crate::reader::PdfReader) -> Self {
        let resolve = |object: &PdfObject| reader.resolve(object.clone()).ok();
        let filters = match dict.get("Filter").and_then(resolve) {
            Some(PdfObject::Name(name)) => vec![name],
            Some(PdfObject::Array(items)) => items
                .iter()
                .filter_map(|item| {
                    resolve(item).and_then(|item| item.as_name().map(str::to_string))
                })
                .collect(),
            _ => Vec::new(),
        };
        let decode_parms = match dict.get("DecodeParms").and_then(resolve) {
            Some(PdfObject::Dictionary(parms)) => Some(parms),
            Some(PdfObject::Array(items)) if items.len() == 1 => {
                resolve(&items[0]).and_then(|item| item.as_dict().cloned())
            }
            _ => None,
        };
        let color_components = match dict.get("ColorSpace").and_then(resolve) {
            Some(PdfObject::Name(name)) => match name.as_str() {
                "DeviceGray" | "G" => Some(1),
                "DeviceRGB" | "RGB" => Some(3),
                "DeviceCMYK" | "CMYK" => Some(4),
                _ => None,
            },
            Some(PdfObject::Array(items)) => {
                let family = items.first().and_then(resolve);
                match family.as_ref().and_then(PdfObject::as_name) {
                    Some("ICCBased") => items
                        .get(1)
                        .and_then(resolve)
                        .and_then(|profile| match profile {
                            PdfObject::Stream { dict, .. } => dict.get_integer("N"),
                            PdfObject::Dictionary(dict) => dict.get_integer("N"),
                            _ => None,
                        })
                        .and_then(|n| u8::try_from(n).ok())
                        .filter(|n| matches!(n, 1 | 3 | 4)),
                    Some("CalGray") => Some(1),
                    Some("CalRGB") => Some(3),
                    _ => None,
                }
            }
            _ => None,
        };
        let smask_in_data = dict.get_integer("SMaskInData").unwrap_or(0) != 0;
        Self {
            filters,
            color_components,
            decode_parms,
            has_mask: dict.contains_key("SMask") || dict.contains_key("Mask") || smask_in_data,
            has_decode: dict.contains_key("Decode"),
            image_mask: matches!(dict.get("ImageMask"), Some(PdfObject::Boolean(true))),
            bits: dict.get_integer("BitsPerComponent"),
        }
    }

    fn single_filter(&self, names: &[&str]) -> bool {
        matches!(self.filters.as_slice(), [only] if names.contains(&only.as_str()))
    }

    fn unmasked(&self) -> bool {
        !self.has_mask && !self.has_decode && !self.image_mask
    }

    fn channels(&self) -> Option<u8> {
        self.color_components
    }
}

/// A Flate image stored with PNG row predictors is already a PNG `IDAT`
/// zlib stream; wrap it in PNG chunks unchanged.
fn flate_png_passthrough(
    traits: &ImageTraits,
    image: &ImageReference,
    raw: &[u8],
) -> Option<Vec<u8>> {
    if !traits.single_filter(&["FlateDecode", "Fl"]) || !traits.unmasked() || traits.bits != Some(8)
    {
        return None;
    }
    let components = traits.channels().filter(|n| matches!(n, 1 | 3))?;
    let parms = traits.decode_parms.as_ref()?;
    if parms.get_integer("Predictor").unwrap_or(1) < 10
        || parms.get_integer("Colors").unwrap_or(1) != i64::from(components)
        || parms.get_integer("BitsPerComponent").unwrap_or(8) != 8
        || parms.get_integer("Columns").unwrap_or(1) != i64::from(image.width)
    {
        return None;
    }
    // The zlib stream must at least carry a valid header.
    if raw.len() < 2 || (u16::from(raw[0]) << 8 | u16::from(raw[1])) % 31 != 0 || raw[0] & 0x0f != 8
    {
        return None;
    }
    let color_type = if components == 1 { 0u8 } else { 2u8 };
    let mut png = Vec::with_capacity(raw.len() + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&image.width.to_be_bytes());
    ihdr.extend_from_slice(&image.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", raw);
    png_chunk(&mut png, b"IEND", &[]);
    Some(png)
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_chunk_crc_matches_the_png_specification() {
        // The CRC of an empty IEND chunk is fixed by the PNG specification.
        let mut out = Vec::new();
        png_chunk(&mut out, b"IEND", &[]);
        assert_eq!(out, b"\x00\x00\x00\x00IEND\xaeB`\x82");
    }
}
