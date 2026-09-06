use std::collections::{HashMap, HashSet};

use crate::content::Operand;
use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

#[derive(Debug, Clone)]
pub struct ImageLocateOptions {
    /// Pages to search. None = all pages.
    pub pages: Option<Vec<usize>>,

    /// Minimum image width in pixels.
    pub min_width: u32,

    /// Minimum image height in pixels.
    pub min_height: u32,

    /// Include ImageMask images.
    pub include_masks: bool,

    /// Include soft-mask images.
    pub include_soft_masks: bool,

    /// Include inline images.
    pub include_inline: bool,
}

impl Default for ImageLocateOptions {
    fn default() -> Self {
        Self {
            pages: None,
            min_width: 1,
            min_height: 1,
            include_masks: false,
            include_soft_masks: false,
            include_inline: true,
        }
    }
}

/// Raw data captured for an inline image (BI/ID/EI) so it can be decoded and
/// exported without re-walking the page content stream.
#[derive(Debug, Clone)]
pub struct InlineImageData {
    /// The raw bytes between `ID` and `EI`, with any preceding (non-image)
    /// filters still applied — i.e. exactly the inline image stream payload.
    pub bytes: Vec<u8>,

    /// Bits per component, resolved from `/BPC` or `/BitsPerComponent`.
    /// Non-mask images must declare this unless the terminal JPX filter carries
    /// sample depth in the codestream.
    pub bits_per_component: u8,

    /// The filter chain (`/F` or `/Filter`), in application order. Both
    /// abbreviated (`Fl`, `AHx`, `CCF`, ...) and full names are preserved
    /// as-is; the decode path understands both.
    pub filters: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImageReference {
    /// Page number (1-indexed) where this image appears.
    pub page_number: usize,

    /// XObject resource name, or an auto-generated inline image name.
    pub xobject_name: String,

    /// PDF object number. 0 for inline images.
    pub object_number: u32,

    /// PDF generation number. 0 for inline images.
    pub generation_number: u16,

    /// Image width in pixels.
    pub width: u32,

    /// Image height in pixels.
    pub height: u32,

    /// Bits per component.
    pub bits_per_component: u8,

    /// Color space name.
    pub color_space: String,

    /// Filter(s) applied to the image stream.
    pub filter: Vec<String>,

    /// True if this is an inline image.
    pub is_inline: bool,

    /// True if /ImageMask is true.
    pub is_mask: bool,

    /// True if this image is referenced as /SMask by another image.
    pub is_smask: bool,

    /// For inline images, the captured raw data needed to decode/export them.
    /// `None` for XObject images (which are decoded via their object number).
    pub inline_data: Option<InlineImageData>,
}

impl ImageReference {
    /// Approximate uncompressed size in bytes.
    pub fn uncompressed_bytes(&self) -> usize {
        let channels = match self.color_space.as_str() {
            "DeviceGray" | "G" => 1usize,
            "DeviceCMYK" | "CMYK" => 4usize,
            _ => 3usize,
        };
        let bpp = (self.bits_per_component as usize * channels).div_ceil(8);
        self.width as usize * self.height as usize * bpp
    }
}

/// Scalar/array parameters parsed from an inline image's BI...ID dictionary.
#[derive(Debug, Default)]
struct InlineParams {
    values: HashMap<String, Operand>,
}

pub struct ImageLocator;

impl ImageLocator {
    /// Find all images in the document matching the given options.
    pub fn find_all_images(
        engine: &ContentEngine,
        options: &ImageLocateOptions,
    ) -> Result<Vec<ImageReference>> {
        let total_pages = engine.page_count()?;
        let pages: Vec<usize> = match &options.pages {
            Some(list) => list.clone(),
            None => (1..=total_pages).collect(),
        };
        let mut all_refs = Vec::new();
        for page_num in pages {
            let page_refs = Self::find_page_images(engine, page_num, options)?;
            all_refs.extend(page_refs);
        }

        all_refs.retain(|r| {
            if r.is_mask && !options.include_masks {
                return false;
            }
            if r.is_smask && !options.include_soft_masks {
                return false;
            }
            if r.width < options.min_width {
                return false;
            }
            if r.height < options.min_height {
                return false;
            }
            true
        });

        Ok(all_refs)
    }

    /// Find images on a single page.
    pub fn find_page_images(
        engine: &ContentEngine,
        page_number: usize,
        options: &ImageLocateOptions,
    ) -> Result<Vec<ImageReference>> {
        let resources = engine.get_page_resources(page_number)?;
        let reader = engine.document().reader();
        let mut refs = Vec::new();
        let mut visited = HashSet::new();
        let mut soft_mask_objects = HashSet::new();

        Self::walk_xobject_dict(
            &resources.xobjects,
            page_number,
            reader,
            &mut visited,
            &mut soft_mask_objects,
            options,
            &mut refs,
        )?;

        if options.include_inline {
            let inline_refs = Self::find_inline_images(engine, page_number)?;
            refs.extend(inline_refs);
        }

        Self::mark_soft_masks(&mut refs, &soft_mask_objects);

        Ok(refs)
    }

    /// Return unique images by object_number. Inline images are always unique.
    pub fn deduplicate(refs: Vec<ImageReference>) -> Vec<ImageReference> {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut result = Vec::new();

        for r in refs {
            if r.object_number == 0 || seen.insert(r.object_number) {
                result.push(r);
            }
        }

        result
    }

    /// Get the raw encoded stream bytes for an image reference.
    pub fn get_stream_bytes(image: &ImageReference, reader: &PdfReader) -> Result<Option<Vec<u8>>> {
        if image.is_inline || image.object_number == 0 {
            return Ok(None);
        }

        match reader.get_object(image.object_number, image.generation_number)? {
            PdfObject::Stream { raw, .. } => Ok(Some(raw)),
            _ => Err(WellfriendError::MalformedPdf(format!(
                "image object {} is not a stream",
                image.object_number
            ))),
        }
    }

    fn image_ref_from_dict(
        page_number: usize,
        xobject_name: String,
        object_number: u32,
        generation_number: u16,
        dict: &PdfDictionary,
    ) -> Result<ImageReference> {
        let label = format!("image XObject /{xobject_name}");
        let filters = Self::extract_filters(dict, &label)?;
        let width = Self::required_positive_u32(dict, "Width", "W", &label)?;
        let height = Self::required_positive_u32(dict, "Height", "H", &label)?;
        let is_mask = Self::get_image_mask(dict, &label)?;
        let bpc = Self::image_bits_per_component(dict, &filters, is_mask, &label)?;
        let color_space = Self::extract_color_space(dict, &filters, is_mask, &label)?;

        Ok(ImageReference {
            page_number,
            xobject_name,
            object_number,
            generation_number,
            width,
            height,
            bits_per_component: bpc,
            color_space,
            filter: filters,
            is_inline: false,
            is_mask,
            is_smask: false,
            inline_data: None,
        })
    }

    fn required_positive_u32(
        dict: &PdfDictionary,
        key: &str,
        short_key: &str,
        label: &str,
    ) -> Result<u32> {
        let value = dict
            .get_integer(key)
            .or_else(|| dict.get_integer(short_key));
        match value {
            Some(number) if number > 0 => u32::try_from(number).map_err(|_| {
                WellfriendError::MalformedPdf(format!("{label} /{key} exceeds dimension limit"))
            }),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /{key} must be a positive integer"
            ))),
            None if dict.contains_key(key) || dict.contains_key(short_key) => Err(
                WellfriendError::MalformedPdf(format!("{label} /{key} is not an integer")),
            ),
            None => Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /{key}"
            ))),
        }
    }

    fn image_bits_per_component(
        dict: &PdfDictionary,
        filters: &[String],
        is_mask: bool,
        label: &str,
    ) -> Result<u8> {
        let value = dict
            .get_integer("BitsPerComponent")
            .or_else(|| dict.get_integer("BPC"));
        if is_mask {
            return match value {
                Some(1) => Ok(1),
                Some(_) => Err(WellfriendError::MalformedPdf(format!(
                    "{label} /BitsPerComponent must be 1 for /ImageMask true"
                ))),
                None if dict.contains_key("BitsPerComponent") || dict.contains_key("BPC") => {
                    Err(WellfriendError::MalformedPdf(format!(
                        "{label} /BitsPerComponent is not an integer"
                    )))
                }
                None => Ok(1),
            };
        }

        match value {
            Some(number @ (1 | 2 | 4 | 8 | 16)) => Ok(number as u8),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent must be one of 1, 2, 4, 8, or 16"
            ))),
            None if !dict.contains_key("BitsPerComponent")
                && !dict.contains_key("BPC")
                && Self::terminal_filter_carries_sample_depth(filters) =>
            {
                Ok(8)
            }
            None if dict.contains_key("BitsPerComponent") || dict.contains_key("BPC") => {
                Err(WellfriendError::MalformedPdf(format!(
                    "{label} /BitsPerComponent is not an integer"
                )))
            }
            None => Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /BitsPerComponent"
            ))),
        }
    }

    fn extract_color_space(
        dict: &PdfDictionary,
        filters: &[String],
        is_mask: bool,
        label: &str,
    ) -> Result<String> {
        if is_mask {
            return Ok("DeviceGray".to_string());
        }

        match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
            Some(PdfObject::Name(name)) => Ok(Self::expand_color_space_name(name)),
            Some(PdfObject::Array(arr)) => arr
                .first()
                .and_then(PdfObject::as_name)
                .map(Self::expand_color_space_name)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!("{label} has malformed /ColorSpace"))
                }),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /ColorSpace is not a name or array"
            ))),
            None if Self::terminal_filter_carries_sample_depth(filters) => {
                Ok("JPXDecode".to_string())
            }
            None => Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /ColorSpace"
            ))),
        }
    }

    fn extract_filters(dict: &PdfDictionary, label: &str) -> Result<Vec<String>> {
        let value = dict.get("Filter").or_else(|| dict.get("F"));
        match value {
            Some(PdfObject::Name(name)) => Ok(vec![name.clone()]),
            Some(PdfObject::Array(arr)) => arr
                .iter()
                .map(|value| {
                    value.as_name().map(str::to_string).ok_or_else(|| {
                        WellfriendError::MalformedPdf(format!(
                            "{label} /Filter array contains a non-name entry"
                        ))
                    })
                })
                .collect(),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /Filter is not a name or name array"
            ))),
            _ => Ok(vec![]),
        }
    }

    fn get_image_mask(dict: &PdfDictionary, label: &str) -> Result<bool> {
        match dict.get("ImageMask").or_else(|| dict.get("IM")) {
            Some(PdfObject::Boolean(value)) => Ok(*value),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /ImageMask is not a boolean"
            ))),
            _ => Ok(false),
        }
    }

    fn terminal_filter_carries_sample_depth(filters: &[String]) -> bool {
        matches!(
            filters.last().map(String::as_str),
            Some("JPXDecode" | "JPX")
        )
    }

    fn expand_color_space_name(name: &str) -> String {
        match name {
            "G" => "DeviceGray".to_string(),
            "RGB" => "DeviceRGB".to_string(),
            "CMYK" => "DeviceCMYK".to_string(),
            other => other.to_string(),
        }
    }

    fn find_inline_images(
        engine: &ContentEngine,
        page_number: usize,
    ) -> Result<Vec<ImageReference>> {
        let ops = engine.get_page_content(page_number)?;
        let mut inline_refs = Vec::new();
        let mut inline_index = 0usize;
        let mut i = 0usize;

        while i < ops.len() {
            let op = &ops[i];
            if op.operator == "ID" {
                let params = Self::parse_inline_image_params(&op.operands);
                let pixel_bytes = if i + 1 < ops.len() && ops[i + 1].operator == "inline_image_data"
                {
                    ops[i + 1].string_bytes(0).map(|bytes| bytes.to_vec())
                } else {
                    None
                };

                inline_refs.push(Self::inline_ref_from_params(
                    page_number,
                    inline_index,
                    &params,
                    pixel_bytes,
                )?);
                inline_index += 1;
            }
            i += 1;
        }

        Ok(inline_refs)
    }

    fn inline_ref_from_params(
        page_number: usize,
        inline_index: usize,
        params: &InlineParams,
        pixel_bytes: Option<Vec<u8>>,
    ) -> Result<ImageReference> {
        let label = format!("inline image {page_number}:{inline_index}");
        let filter = Self::inline_filters_strict(params, &label)?;
        let filter_refs: Vec<&str> = filter.iter().map(String::as_str).collect();
        let is_mask = Self::inline_bool(params, "IM", "ImageMask", &label)?;
        let width = Self::inline_required_positive_u32(params, "W", "Width", &label)?;
        let height = Self::inline_required_positive_u32(params, "H", "Height", &label)?;
        let bits_per_component = if is_mask {
            Self::inline_mask_bits_per_component(params, &label)?
        } else {
            Self::inline_bits_per_component(params, &filter_refs, &label)?
        };
        let color_space = Self::inline_color_space(params, &filter_refs, is_mask, &label)?;

        let inline_data = pixel_bytes.map(|bytes| InlineImageData {
            bytes,
            bits_per_component,
            filters: filter.clone(),
        });

        Ok(ImageReference {
            page_number,
            xobject_name: format!("inline_{}_{}", page_number, inline_index),
            object_number: 0,
            generation_number: 0,
            width,
            height,
            bits_per_component,
            color_space,
            filter,
            is_inline: true,
            is_mask,
            is_smask: false,
            inline_data,
        })
    }

    fn parse_inline_image_params(operands: &[Operand]) -> InlineParams {
        let mut params = InlineParams::default();

        let mut iter = operands.iter();
        while let Some(op) = iter.next() {
            if let Some(key) = op.as_name() {
                if let Some(next) = iter.next() {
                    params.values.insert(key.to_string(), next.clone());
                }
            }
        }

        params
    }

    fn inline_value<'a>(
        params: &'a InlineParams,
        short_key: &str,
        key: &str,
    ) -> Option<&'a Operand> {
        params
            .values
            .get(short_key)
            .or_else(|| params.values.get(key))
    }

    fn inline_required_positive_u32(
        params: &InlineParams,
        short_key: &str,
        key: &str,
        label: &str,
    ) -> Result<u32> {
        let Some(value) = Self::inline_value(params, short_key, key) else {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /{key}"
            )));
        };
        let Some(number) = value.as_number() else {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /{key} is not numeric"
            )));
        };
        if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /{key} must be a positive integer"
            )));
        }
        if number > f64::from(u32::MAX) {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /{key} exceeds dimension limit"
            )));
        }
        Ok(number as u32)
    }

    fn inline_bits_per_component(
        params: &InlineParams,
        filters: &[&str],
        label: &str,
    ) -> Result<u8> {
        let Some(value) = Self::inline_value(params, "BPC", "BitsPerComponent") else {
            if Self::inline_terminal_filter_carries_sample_depth(filters) {
                return Ok(8);
            }
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /BitsPerComponent"
            )));
        };
        let Some(number) = value.as_number() else {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent is not numeric"
            )));
        };
        if !number.is_finite() || number.fract() != 0.0 {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent must be one of 1, 2, 4, 8, or 16"
            )));
        }
        match number as i64 {
            1 | 2 | 4 | 8 | 16 => Ok(number as u8),
            _ => Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent must be one of 1, 2, 4, 8, or 16"
            ))),
        }
    }

    fn inline_mask_bits_per_component(params: &InlineParams, label: &str) -> Result<u8> {
        let Some(value) = Self::inline_value(params, "BPC", "BitsPerComponent") else {
            return Ok(1);
        };
        let Some(number) = value.as_number() else {
            return Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent is not numeric"
            )));
        };
        if number == 1.0 {
            Ok(1)
        } else {
            Err(WellfriendError::MalformedPdf(format!(
                "{label} /BitsPerComponent must be 1 for /ImageMask true"
            )))
        }
    }

    fn inline_color_space(
        params: &InlineParams,
        filters: &[&str],
        is_mask: bool,
        label: &str,
    ) -> Result<String> {
        if is_mask {
            return Ok("DeviceGray".to_string());
        }

        match Self::inline_value(params, "CS", "ColorSpace") {
            Some(Operand::Name(name)) => Ok(Self::expand_color_space_name(name)),
            Some(Operand::Array(items)) => items
                .first()
                .and_then(Operand::as_name)
                .map(Self::expand_color_space_name)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!("{label} has malformed /ColorSpace"))
                }),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /ColorSpace is not a name or array"
            ))),
            None if Self::inline_terminal_filter_carries_sample_depth(filters) => {
                Ok("JPXDecode".to_string())
            }
            None => Err(WellfriendError::MalformedPdf(format!(
                "{label} missing /ColorSpace"
            ))),
        }
    }

    fn inline_bool(params: &InlineParams, short_key: &str, key: &str, label: &str) -> Result<bool> {
        match Self::inline_value(params, short_key, key) {
            Some(Operand::Boolean(value)) => Ok(*value),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /{key} is not a boolean"
            ))),
            None => Ok(false),
        }
    }

    fn inline_filters_strict(params: &InlineParams, label: &str) -> Result<Vec<String>> {
        match Self::inline_value(params, "F", "Filter") {
            Some(Operand::Name(name)) => Ok(vec![name.clone()]),
            Some(Operand::Array(items)) => items
                .iter()
                .map(|value| {
                    value.as_name().map(str::to_string).ok_or_else(|| {
                        WellfriendError::MalformedPdf(format!(
                            "{label} /Filter array contains a non-name entry"
                        ))
                    })
                })
                .collect(),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{label} /Filter is not a name or name array"
            ))),
            None => Ok(vec![]),
        }
    }

    fn inline_terminal_filter_carries_sample_depth(filters: &[&str]) -> bool {
        matches!(filters.last().copied(), Some("JPXDecode" | "JPX"))
    }

    fn walk_xobject_dict(
        xobjects: &HashMap<String, (u32, u16)>,
        page_number: usize,
        reader: &PdfReader,
        visited: &mut HashSet<u32>,
        soft_mask_objects: &mut HashSet<u32>,
        options: &ImageLocateOptions,
        results: &mut Vec<ImageReference>,
    ) -> Result<()> {
        let _ = options;
        for (name, &(obj_num, gen_num)) in xobjects {
            if !visited.insert(obj_num) {
                continue;
            }

            let obj = match reader.get_object(obj_num, gen_num) {
                Ok(obj) => obj,
                Err(err) => {
                    log::warn!(
                        "XObject '{}' (obj {}) failed to resolve: {}",
                        name,
                        obj_num,
                        err
                    );
                    continue;
                }
            };

            let dict = match &obj {
                PdfObject::Stream { dict, .. } => dict.clone(),
                _ => {
                    log::debug!("XObject '{}' is not a stream, skipping", name);
                    continue;
                }
            };

            match dict.get_name("Subtype") {
                Some("Image") => {
                    if let Some(PdfObject::Reference { number, .. }) = dict.get("SMask") {
                        soft_mask_objects.insert(*number);
                    }
                    results.push(Self::image_ref_from_dict(
                        page_number,
                        name.clone(),
                        obj_num,
                        gen_num,
                        &dict,
                    )?);
                }
                Some("Form") => {
                    log::debug!("XObject '{}' is a Form; walking nested images", name);
                    if let Some(res_dict) = Self::resolve_resource_dict(&dict, reader) {
                        if let Some(xobj_dict) = res_dict.get_dict("XObject") {
                            let nested: HashMap<String, (u32, u16)> = xobj_dict
                                .entries()
                                .filter_map(|(key, value)| {
                                    value
                                        .as_reference()
                                        .map(|reference| (key.clone(), reference))
                                })
                                .collect();
                            Self::walk_xobject_dict(
                                &nested,
                                page_number,
                                reader,
                                visited,
                                soft_mask_objects,
                                options,
                                results,
                            )?;
                        }
                    }
                }
                Some(other) => {
                    log::debug!(
                        "XObject '{}' has unsupported subtype '{}'; skipping",
                        name,
                        other
                    );
                }
                None => {
                    log::debug!("XObject '{}' has no /Subtype; skipping", name);
                }
            }
        }
        Ok(())
    }

    fn resolve_resource_dict(dict: &PdfDictionary, reader: &PdfReader) -> Option<PdfDictionary> {
        match dict.get("Resources") {
            Some(PdfObject::Dictionary(resources)) => Some(resources.clone()),
            Some(PdfObject::Reference { number, generation }) => reader
                .get_object(*number, *generation)
                .ok()
                .and_then(|obj| obj.as_dict().cloned()),
            _ => None,
        }
    }

    fn mark_soft_masks(refs: &mut [ImageReference], smask_objs: &HashSet<u32>) {
        for image_ref in refs.iter_mut() {
            if smask_objs.contains(&image_ref.object_number) {
                image_ref.is_smask = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_image_dict() -> PdfDictionary {
        let mut dict = PdfDictionary::empty();
        dict.insert("Width", PdfObject::Integer(10));
        dict.insert("Height", PdfObject::Integer(10));
        dict.insert("BitsPerComponent", PdfObject::Integer(8));
        dict.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
        dict
    }

    fn image_ref(dict: &PdfDictionary) -> ImageReference {
        ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, dict).unwrap()
    }

    fn inline_ref(operands: Vec<Operand>) -> Result<ImageReference> {
        let params = ImageLocator::parse_inline_image_params(&operands);
        ImageLocator::inline_ref_from_params(1, 0, &params, Some(vec![0]))
    }

    #[test]
    fn extract_color_space_handles_names() {
        let mut d = valid_image_dict();
        d.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
        let img = image_ref(&d);
        assert_eq!(img.color_space, "DeviceRGB");
    }

    #[test]
    fn extract_filters_handles_single_name() {
        let mut d = valid_image_dict();
        d.insert("Filter", PdfObject::Name("DCTDecode".to_string()));
        let img = image_ref(&d);
        assert_eq!(img.filter, vec!["DCTDecode"]);
    }

    #[test]
    fn extract_filters_handles_array() {
        let mut d = valid_image_dict();
        d.insert(
            "Filter",
            PdfObject::Array(vec![
                PdfObject::Name("FlateDecode".to_string()),
                PdfObject::Name("DCTDecode".to_string()),
            ]),
        );
        let img = image_ref(&d);
        assert_eq!(img.filter, vec!["FlateDecode", "DCTDecode"]);
    }

    #[test]
    fn is_mask_detected_correctly() {
        let mut d = PdfDictionary::empty();
        d.insert("ImageMask", PdfObject::Boolean(true));
        d.insert("Width", PdfObject::Integer(10));
        d.insert("Height", PdfObject::Integer(10));
        let img = image_ref(&d);
        assert!(img.is_mask);
        assert_eq!(img.bits_per_component, 1);
        assert_eq!(img.color_space, "DeviceGray");
    }

    #[test]
    fn image_mask_non_boolean_is_rejected() {
        let mut d = PdfDictionary::empty();
        d.insert("IM", PdfObject::Name("true".to_string()));
        d.insert("Width", PdfObject::Integer(10));
        d.insert("Height", PdfObject::Integer(10));
        let error = ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, &d).unwrap_err();
        assert!(format!("{error}").contains("/ImageMask is not a boolean"));
    }

    #[test]
    fn abbreviated_color_space_names_expanded() {
        let mut d = valid_image_dict();
        d.insert("ColorSpace", PdfObject::Name("G".to_string()));
        let img = image_ref(&d);
        assert_eq!(img.color_space, "DeviceGray");

        let mut d2 = valid_image_dict();
        d2.insert("ColorSpace", PdfObject::Name("RGB".to_string()));
        let img2 = ImageLocator::image_ref_from_dict(1, "Im2".to_string(), 6, 0, &d2).unwrap();
        assert_eq!(img2.color_space, "DeviceRGB");
    }

    #[test]
    fn uncompressed_bytes_calculation() {
        let mut r = ImageReference {
            page_number: 1,
            xobject_name: "Im1".to_string(),
            object_number: 5,
            generation_number: 0,
            width: 100,
            height: 100,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        assert_eq!(r.uncompressed_bytes(), 30000);

        r.color_space = "DeviceGray".to_string();
        assert_eq!(r.uncompressed_bytes(), 10000);
    }

    #[test]
    fn uncompressed_bytes_for_cmyk() {
        let r = ImageReference {
            page_number: 1,
            xobject_name: "Im1".to_string(),
            object_number: 5,
            generation_number: 0,
            width: 50,
            height: 50,
            bits_per_component: 8,
            color_space: "DeviceCMYK".to_string(),
            filter: vec!["DCTDecode".to_string()],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        assert_eq!(r.uncompressed_bytes(), 10000);
    }

    #[test]
    fn mark_soft_masks_uses_collected_primary_walk_refs() {
        let mut refs = vec![
            ImageReference {
                page_number: 1,
                xobject_name: "Image".to_string(),
                object_number: 10,
                generation_number: 0,
                width: 10,
                height: 10,
                bits_per_component: 8,
                color_space: "DeviceRGB".to_string(),
                filter: vec![],
                is_inline: false,
                is_mask: false,
                is_smask: false,
                inline_data: None,
            },
            ImageReference {
                page_number: 1,
                xobject_name: "SoftMask".to_string(),
                object_number: 11,
                generation_number: 0,
                width: 10,
                height: 10,
                bits_per_component: 8,
                color_space: "DeviceGray".to_string(),
                filter: vec![],
                is_inline: false,
                is_mask: false,
                is_smask: false,
                inline_data: None,
            },
        ];
        let mut soft_masks = HashSet::new();
        soft_masks.insert(11);

        ImageLocator::mark_soft_masks(&mut refs, &soft_masks);

        assert!(!refs[0].is_smask);
        assert!(refs[1].is_smask);
    }

    #[test]
    fn parse_inline_image_params_handles_mixed_types() {
        let operands = vec![
            Operand::Name("W".to_string()),
            Operand::Integer(200),
            Operand::Name("H".to_string()),
            Operand::Integer(150),
            Operand::Name("CS".to_string()),
            Operand::Name("RGB".to_string()),
            Operand::Name("BPC".to_string()),
            Operand::Integer(8),
            Operand::Name("IM".to_string()),
            Operand::Boolean(false),
        ];
        let params = ImageLocator::parse_inline_image_params(&operands);
        assert_eq!(params.values.get("W"), Some(&Operand::Integer(200)));
        assert_eq!(params.values.get("H"), Some(&Operand::Integer(150)));
        assert_eq!(
            params.values.get("CS"),
            Some(&Operand::Name("RGB".to_string()))
        );
        assert_eq!(params.values.get("IM"), Some(&Operand::Boolean(false)));
    }

    #[test]
    fn inline_filters_handles_single_abbreviated_name() {
        let operands = vec![
            Operand::Name("F".to_string()),
            Operand::Name("Fl".to_string()),
        ];
        let params = ImageLocator::parse_inline_image_params(&operands);
        assert_eq!(
            ImageLocator::inline_filters_strict(&params, "inline image").unwrap(),
            vec!["Fl".to_string()]
        );
    }

    #[test]
    fn inline_filters_handles_full_filter_key() {
        let operands = vec![
            Operand::Name("Filter".to_string()),
            Operand::Name("FlateDecode".to_string()),
        ];
        let params = ImageLocator::parse_inline_image_params(&operands);
        assert_eq!(
            ImageLocator::inline_filters_strict(&params, "inline image").unwrap(),
            vec!["FlateDecode".to_string()]
        );
    }

    #[test]
    fn inline_filters_handles_name_array_in_order() {
        // /F [/AHx /Fl]
        let operands = vec![
            Operand::Name("F".to_string()),
            Operand::Array(vec![
                Operand::Name("AHx".to_string()),
                Operand::Name("Fl".to_string()),
            ]),
        ];
        let params = ImageLocator::parse_inline_image_params(&operands);
        assert_eq!(
            ImageLocator::inline_filters_strict(&params, "inline image").unwrap(),
            vec!["AHx".to_string(), "Fl".to_string()]
        );
    }

    #[test]
    fn inline_filters_empty_when_absent() {
        let operands = vec![Operand::Name("W".to_string()), Operand::Integer(2)];
        let params = ImageLocator::parse_inline_image_params(&operands);
        assert!(ImageLocator::inline_filters_strict(&params, "inline image")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn extract_filters_handles_missing_filter_gracefully() {
        let d = valid_image_dict();
        let img = image_ref(&d);
        assert!(img.filter.is_empty());
    }

    #[test]
    fn default_options_do_not_include_masks() {
        let opts = ImageLocateOptions::default();
        assert!(!opts.include_masks);
        assert!(!opts.include_soft_masks);
        assert!(opts.include_inline);
        assert_eq!(opts.min_width, 1);
        assert_eq!(opts.min_height, 1);
    }

    #[test]
    fn image_ref_from_dict_rejects_malformed_metadata() {
        let mut d = PdfDictionary::empty();
        d.insert("Width", PdfObject::Integer(-10));
        d.insert("Height", PdfObject::Integer(-1));
        d.insert("BitsPerComponent", PdfObject::Integer(20));
        d.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
        let error = ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, &d).unwrap_err();
        assert!(format!("{error}").contains("/Width must be a positive integer"));

        let mut unsupported_bpc = valid_image_dict();
        unsupported_bpc.insert("BitsPerComponent", PdfObject::Integer(20));
        let error = ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, &unsupported_bpc)
            .unwrap_err();
        assert!(format!("{error}").contains("/BitsPerComponent must be one of"));

        let mut missing_color_space = valid_image_dict();
        missing_color_space.remove("ColorSpace");
        let error =
            ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, &missing_color_space)
                .unwrap_err();
        assert!(format!("{error}").contains("missing /ColorSpace"));
    }

    #[test]
    fn image_ref_from_dict_allows_jpx_to_carry_sample_depth() {
        let mut d = PdfDictionary::empty();
        d.insert("Width", PdfObject::Integer(10));
        d.insert("Height", PdfObject::Integer(10));
        d.insert("Filter", PdfObject::Name("JPXDecode".to_string()));
        let img = image_ref(&d);
        assert_eq!(img.bits_per_component, 8);
        assert_eq!(img.color_space, "JPXDecode");
    }

    #[test]
    fn image_ref_from_dict_rejects_malformed_filter_metadata() {
        let mut d = valid_image_dict();
        d.insert(
            "Filter",
            PdfObject::Array(vec![
                PdfObject::Name("FlateDecode".to_string()),
                PdfObject::Integer(7),
            ]),
        );
        let error = ImageLocator::image_ref_from_dict(1, "Im1".to_string(), 5, 0, &d).unwrap_err();
        assert!(format!("{error}").contains("/Filter array contains a non-name entry"));
    }

    #[test]
    fn inline_ref_from_params_rejects_defaulted_metadata() {
        let missing_bpc = inline_ref(vec![
            Operand::Name("W".to_string()),
            Operand::Integer(1),
            Operand::Name("H".to_string()),
            Operand::Integer(1),
            Operand::Name("CS".to_string()),
            Operand::Name("RGB".to_string()),
        ])
        .unwrap_err();
        assert!(format!("{missing_bpc}").contains("missing /BitsPerComponent"));

        let missing_color_space = inline_ref(vec![
            Operand::Name("W".to_string()),
            Operand::Integer(1),
            Operand::Name("H".to_string()),
            Operand::Integer(1),
            Operand::Name("BPC".to_string()),
            Operand::Integer(8),
        ])
        .unwrap_err();
        assert!(format!("{missing_color_space}").contains("missing /ColorSpace"));

        let malformed_filter = inline_ref(vec![
            Operand::Name("W".to_string()),
            Operand::Integer(1),
            Operand::Name("H".to_string()),
            Operand::Integer(1),
            Operand::Name("BPC".to_string()),
            Operand::Integer(8),
            Operand::Name("CS".to_string()),
            Operand::Name("RGB".to_string()),
            Operand::Name("F".to_string()),
            Operand::Array(vec![Operand::Name("Fl".to_string()), Operand::Integer(7)]),
        ])
        .unwrap_err();
        assert!(format!("{malformed_filter}").contains("/Filter array contains a non-name entry"));
    }

    #[test]
    fn inline_ref_from_params_allows_mask_defaults() {
        let img = inline_ref(vec![
            Operand::Name("W".to_string()),
            Operand::Integer(1),
            Operand::Name("H".to_string()),
            Operand::Integer(1),
            Operand::Name("IM".to_string()),
            Operand::Boolean(true),
        ])
        .unwrap();
        assert!(img.is_mask);
        assert_eq!(img.bits_per_component, 1);
        assert_eq!(img.color_space, "DeviceGray");
    }

    #[test]
    fn deduplicate_keeps_first_occurrence() {
        let refs = vec![
            ImageReference {
                page_number: 1,
                object_number: 5,
                xobject_name: "Im1".to_string(),
                generation_number: 0,
                width: 100,
                height: 100,
                bits_per_component: 8,
                color_space: "DeviceRGB".to_string(),
                filter: vec![],
                is_inline: false,
                is_mask: false,
                is_smask: false,
                inline_data: None,
            },
            ImageReference {
                page_number: 2,
                object_number: 5,
                xobject_name: "Im1".to_string(),
                generation_number: 0,
                width: 100,
                height: 100,
                bits_per_component: 8,
                color_space: "DeviceRGB".to_string(),
                filter: vec![],
                is_inline: false,
                is_mask: false,
                is_smask: false,
                inline_data: None,
            },
            ImageReference {
                page_number: 1,
                object_number: 7,
                xobject_name: "Im2".to_string(),
                generation_number: 0,
                width: 50,
                height: 50,
                bits_per_component: 8,
                color_space: "DeviceGray".to_string(),
                filter: vec![],
                is_inline: false,
                is_mask: false,
                is_smask: false,
                inline_data: None,
            },
        ];
        let deduped = ImageLocator::deduplicate(refs);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].page_number, 1);
        assert_eq!(deduped[1].object_number, 7);
    }
}
