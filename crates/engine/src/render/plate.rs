//! Native CMYK and DeviceN plate rendering (Poppler `splashModeCMYK8` /
//! `splashModeDeviceN8`).
//!
//! A plate pass renders the page through the ordinary compositor with every
//! paint colour replaced by the *inverted ink* of up to three output plates,
//! one per R/G/B channel: `255 - ink`, so paper (no ink) is 255 and the white
//! page background is already blank film. Coverage, clipping, constant alpha,
//! soft-mask alpha, isolated/knockout groups and the separable blend modes are
//! computed per channel, so each plate composites exactly as in a planar CMYK
//! or DeviceN compositor. Separable blend modes applied to complemented values
//! are the PDF rule for subtractive spaces (PDF 32000-1 11.3.5), so
//! `Multiply`, `Screen`, `Darken`, ... are exact as well.
//!
//! Colour → ink mapping:
//! - DeviceCMYK (and four-component ICCBased) components are used unchanged.
//! - DeviceGray is pure black ink (`K = 1 - gray`).
//! - DeviceRGB, three-component ICCBased (through the CMM to sRGB), CalRGB,
//!   Lab and anything else resolved to RGB use Poppler's uncalibrated
//!   conversion `c = 1 - r, ...; k = min(c, m, y); c -= k, ...`.
//! - Separation/DeviceN colorants named Cyan/Magenta/Yellow/Black paint their
//!   process plate directly; spot colorants paint their own plate when it is
//!   being separated and otherwise go through the alternate space and tint
//!   transform to CMYK; `/All` paints every plate, `/None` nothing.
//!
//! Overprint (`OP`/`op`, `OPM`) is honoured natively: with overprint on, a
//! paint leaves every plate its colour space does not name untouched. Process
//! spaces name the four process plates (with OPM 1 a DeviceCMYK component of
//! zero names nothing); a separated spot names its own plate; a spot mapped
//! through its alternate names the process plates its alternate colour at full
//! tint inks (Poppler's overprint mask). A CMYK render honouring overprint
//! separates the page's first [`POPPLER_SPOT_PLATES`] spots and adds their
//! alternate ink to the process plates afterwards ([`flatten_spot_inks`]), as
//! Poppler's DeviceN compositor does.
//!
//! Non-separable blend modes, transfer functions and colour-font glyphs are
//! native as well: see [`PlateTarget::check_blend_mode`],
//! [`PlateTarget::transfer_for_pass`] and [`PlateTarget::srgb_pixel`].
//!
//! Features a plate pass cannot reproduce exactly are reported through
//! [`PlateTarget::note_issue`]; callers then fall back to rendering RGB and
//! converting it, and surface the issue as a diagnostic.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::object::PdfObject;
use crate::reader::PdfReader;
use crate::render::buffer::PixelColor;
use crate::render::cmm::ColorTransformOptions;
use crate::render::color::RenderColor;
use crate::render::colorspace::NamedColor;
use crate::render::function::FunctionResources;

/// A page rendered to CMYK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmykRaster {
    pub width: u32,
    pub height: u32,
    /// Interleaved C, M, Y, K ink bytes (0 = no ink), rows without padding.
    pub pixels: Vec<u8>,
    /// True when composited natively in CMYK; false when the page needed a
    /// feature the plate compositor lacks and was rendered in RGB and
    /// converted (see `diagnostics`).
    pub native: bool,
    pub diagnostics: Vec<String>,
}

/// One 8-bit ink plate (0 = no ink).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparationPlate {
    /// Colorant name: Cyan, Magenta, Yellow, Black or the spot name.
    pub name: String,
    pub spot: bool,
    pub pixels: Vec<u8>,
}

/// A page rendered to one plate per colorant (DeviceN).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparationRaster {
    pub width: u32,
    pub height: u32,
    /// The four process plates, then the separated spot plates.
    pub plates: Vec<SeparationPlate>,
    pub native: bool,
    pub diagnostics: Vec<String>,
}

/// Process colorant names in plate order.
pub const PROCESS_COLORANTS: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];

/// Upper bound on separately rendered spot plates.
pub const MAX_SEPARATION_SPOTS: usize = 32;

/// Spot plates Poppler's `splashModeDeviceN8` keeps (`SPOT_NCOMPS`): a CMYK
/// render honouring overprint separates this many spots and flattens them
/// back (see [`flatten_spot_inks`]).
pub(crate) const POPPLER_SPOT_PLATES: usize = 4;

/// Process ink bytes (C, M, Y, K) of a separated spot at each tint byte.
pub(crate) type SpotInkTable = Arc<[[u8; 4]; 256]>;

/// Channel that renders no plate (stays paper).
pub(crate) const NO_PLATE: usize = usize::MAX;

/// Poppler `SplashBitmap::getCMYKLine`, which `pdftoppm -jpegcmyk` and
/// `-overprint` write: every spot plate with ink converts through its
/// alternate space and is *added* to the process plates, clamped
/// (`dblToByte(clip01(c + ...))`, truncating). So a spot overprinting a
/// backdrop that already carries ink in the same process plates adds to
/// it (magenta 102 + 61 = 163) instead of replacing it.
pub(crate) fn flatten_spot_inks<'a>(
    process: [u8; 4],
    spots: impl IntoIterator<Item = (u8, &'a [[u8; 4]; 256])>,
) -> [u8; 4] {
    let mut sum = process.map(|byte| f64::from(byte) / 255.0);
    let mut inked = false;
    for (tint, table) in spots {
        if tint == 0 {
            continue;
        }
        inked = true;
        for (acc, &byte) in sum.iter_mut().zip(&table[usize::from(tint)]) {
            *acc += f64::from(byte) / 255.0;
        }
    }
    if !inked {
        return process;
    }
    sum.map(|value| (value.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Process ink bytes of a resolved colour (`None` when it did not resolve).
fn process_ink_bytes(result: InkResult) -> Option<[u8; 4]> {
    match result {
        InkResult::Ink(ink) => Some([0, 1, 2, 3].map(|plate| unit_to_byte(ink.value(plate)))),
        InkResult::NoPaint => Some([0; 4]),
        _ => None,
    }
}

/// Issue reported when one pass renders several plates and an overprinting
/// paint names some of them but not others; the pass is then repeated with
/// one plate per pass.
pub(crate) const MIXED_OVERPRINT_ISSUE: &str =
    "overprint paint names only some of the plates rendered together";

/// Issue reported when a non-separable blend mode is used in a pass that
/// renders neither exactly the C/M/Y plates nor a single black or spot plate;
/// the page is then repeated with [`PassLayout::ProcessTogether`].
pub(crate) const NONSEPARABLE_LAYOUT_ISSUE: &str =
    "non-separable blend mode in a pass that does not render C, M and Y together";

/// Issue reported when the C/M/Y pass of [`PassLayout::ProcessTogether`]
/// cannot mask an overprinting paint per plate (several paints with
/// different plate masks in one operator, or a paint outside an operator).
pub(crate) const UNMASKED_OVERPRINT_ISSUE: &str =
    "overprint naming only some of C, M and Y could not be masked per plate";

const PROCESS_MASK: u64 = 0b1111;
const MAX_RESOLVE_DEPTH: u8 = 8;

/// Which plates one render pass produces and the shared issue log.
#[derive(Debug)]
pub(crate) struct PlateTarget {
    channels: [usize; 3],
    spots: Arc<[String]>,
    honour_overprint: bool,
    mask_overprint: bool,
    issues: Mutex<Vec<String>>,
    /// Record [`Self::spot_ink_tables`] for separated spots.
    flatten_spots: bool,
    spot_tables: Mutex<HashMap<usize, SpotInkTable>>,
}

impl PlateTarget {
    /// `channels` holds the plate index rendered into R, G and B (0..4 are
    /// the process plates, `4 + i` is `spots[i]`, [`NO_PLATE`] keeps paper).
    pub(crate) fn new(channels: [usize; 3], spots: Arc<[String]>, honour_overprint: bool) -> Self {
        Self {
            channels,
            spots,
            honour_overprint,
            mask_overprint: false,
            issues: Mutex::new(Vec::new()),
            flatten_spots: false,
            spot_tables: Mutex::new(HashMap::new()),
        }
    }

    /// Record, for every separated spot the pass resolves, its process ink
    /// at each tint (the alternate space through the tint transform), so
    /// the spot plates can be flattened into CMYK with
    /// [`flatten_spot_inks`].
    pub(crate) fn with_spot_flattening(mut self, flatten: bool) -> Self {
        self.flatten_spots = flatten;
        self
    }

    /// The recorded spot ink tables by plate index (`4 + spot`).
    pub(crate) fn spot_ink_tables(&self) -> HashMap<usize, SpotInkTable> {
        self.spot_tables
            .lock()
            .map(|tables| tables.clone())
            .unwrap_or_default()
    }

    /// Records the process ink of separated spot `plate` at every tint
    /// byte, as Poppler converts a plate byte (`byteToCol`, 16.16 fixed
    /// point) through the separation's `getCMYK`.
    fn record_spot_table(&self, plate: usize, ink_at: impl Fn(f64) -> Option<[u8; 4]>) {
        if !self.flatten_spots || plate < 4 {
            return;
        }
        let known = match self.spot_tables.lock() {
            Ok(tables) => tables.contains_key(&plate),
            Err(_) => true,
        };
        if known {
            return;
        }
        let mut table = [[0u8; 4]; 256];
        for (byte, entry) in table.iter_mut().enumerate() {
            let fixed = (byte << 8) + byte + (byte >> 7);
            match ink_at(fixed as f64 / 65536.0) {
                Some(ink) => *entry = ink,
                None => return,
            }
        }
        if let Ok(mut tables) = self.spot_tables.lock() {
            tables.entry(plate).or_insert_with(|| Arc::new(table));
        }
    }

    /// Mask overprinting paints per plate instead of reporting
    /// [`MIXED_OVERPRINT_ISSUE`]: the renderer saves the pixels before such
    /// a paint and restores the plates the paint does not name afterwards
    /// (Poppler's per-channel `overprintMask`). Used for the C/M/Y pass of
    /// [`PassLayout::ProcessTogether`], where C, M and Y must stay in one
    /// pass for the non-separable blend modes.
    pub(crate) fn with_overprint_masking(mut self, mask: bool) -> Self {
        self.mask_overprint = mask && !self.single_plate();
        self
    }

    pub(crate) fn masks_overprint(&self) -> bool {
        self.mask_overprint
    }

    /// The channels an overprinting `ink` leaves untouched, when it names
    /// some but not all plates of this pass and per-plate masking is on.
    pub(crate) fn overprint_keep_mask(&self, ink: &Ink) -> Option<[bool; 3]> {
        if !self.mask_overprint {
            return None;
        }
        let keep = self
            .channels
            .map(|plate| plate != NO_PLATE && !ink.names(plate));
        let plates = self.channels.iter().filter(|&&p| p != NO_PLATE).count();
        let kept = keep.iter().filter(|&&k| k).count();
        (kept > 0 && kept < plates).then_some(keep)
    }

    /// One plate in all three channels.
    pub(crate) fn single_plate(&self) -> bool {
        self.channels[0] == self.channels[1] && self.channels[1] == self.channels[2]
    }

    pub(crate) fn honours_overprint(&self) -> bool {
        self.honour_overprint
    }

    /// Suffix for render cache identities of this pass.
    pub(crate) fn cache_identity(&self) -> String {
        format!(
            "|plate={:?}|spots={}|op={}",
            self.channels,
            self.spots.join("\u{1f}"),
            self.honour_overprint
        )
    }

    pub(crate) fn note_issue(&self, issue: impl Into<String>) {
        let issue = issue.into();
        if let Ok(mut issues) = self.issues.lock() {
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
    }

    pub(crate) fn issues(&self) -> Vec<String> {
        self.issues
            .lock()
            .map(|issues| issues.clone())
            .unwrap_or_default()
    }

    /// Check a blend mode against this pass's plates.
    ///
    /// The non-separable modes (Hue, Saturation, Color, Luminosity) are
    /// computed in CMYK on the complements of C, M and Y, with K taken from
    /// the backdrop (Hue, Saturation, Color) or the source (Luminosity)
    /// (PDF 32000-2 11.3.5.3, Poppler `splashModeCMYK8`). A pass rendering
    /// exactly C/M/Y already holds those complements (`255 - ink`) in R/G/B,
    /// so the RGB formulas are that rule. A pass rendering one plate holds it
    /// as a grey triple, for which the formulas reduce to the backdrop (Hue,
    /// Saturation, Color) or the source (Luminosity): the K rule, which spot
    /// plates follow as well. Other passes cannot compute the modes and ask
    /// for [`PassLayout::ProcessTogether`]: a pass mixing K or spots, and a
    /// pass with C, M or Y alone (one plate per pass for overprint), whose
    /// overprint the C/M/Y pass then masks per plate (see
    /// [`Self::with_overprint_masking`]).
    pub(crate) fn check_blend_mode(&self, nonseparable: bool) {
        if !nonseparable || self.channels == [0, 1, 2] {
            return;
        }
        if self.single_plate() && self.channels[0] >= 3 {
            return;
        }
        self.note_issue(NONSEPARABLE_LAYOUT_ISSUE);
    }

    /// Transfer curves for this pass. `/TR` tables map additive values and a
    /// plate holds `255 - ink`, so C, M and Y take the red, green and blue
    /// curves and K (and spot plates) the fourth, grey curve, as Poppler's
    /// `cmykTransferC[i] = 255 - rgbTransferR[255 - i]`. Channels without a
    /// plate keep the identity.
    pub(crate) fn transfer_for_pass(
        &self,
        lut: &crate::content::state::TransferLut,
    ) -> crate::content::state::TransferLut {
        let mut identity = [0u8; 256];
        for (index, value) in identity.iter_mut().enumerate() {
            *value = index as u8;
        }
        let mut channels = lut.channels;
        for (slot, &plate) in channels.iter_mut().zip(self.channels.iter()) {
            *slot = match plate {
                NO_PLATE => identity,
                0..=2 => lut.channels[plate],
                _ => lut.channels[3],
            };
        }
        crate::content::state::TransferLut { channels }
    }

    /// Plate pixel of an sRGB colour (colour-font palette entries and
    /// bitmaps) through the DeviceRGB conversion; alpha is kept.
    pub(crate) fn srgb_pixel(&self, rgba: PixelColor) -> PixelColor {
        let cmyk = rgb_bytes_to_cmyk(rgba[0], rgba[1], rgba[2]);
        let mut out = [255u8, 255, 255, rgba[3]];
        for (byte, &plate) in out.iter_mut().zip(self.channels.iter()) {
            if plate < 4 {
                *byte = 255 - cmyk[plate];
            }
        }
        out
    }

    fn spot_index(&self, name: &str) -> Option<usize> {
        self.spots.iter().position(|spot| spot == name)
    }

    /// Plate index of a colorant this pass set can represent directly.
    fn direct_plate(&self, name: &str) -> Option<usize> {
        PROCESS_COLORANTS
            .iter()
            .position(|process| *process == name)
            .or_else(|| self.spot_index(name).map(|index| 4 + index))
    }

    /// The output pixel for an ink. With `overprint`, plates the colour does
    /// not name are left untouched: a fully transparent pixel when no plate of
    /// this pass is named.
    pub(crate) fn pixel(&self, ink: &Ink, alpha: f32, overprint: bool) -> PixelColor {
        let mut named_any = false;
        let mut unnamed_any = false;
        for &plate in &self.channels {
            if plate == NO_PLATE {
                continue;
            }
            if ink.names(plate) {
                named_any = true;
            } else {
                unnamed_any = true;
            }
        }
        if overprint {
            if !named_any {
                return [0, 0, 0, 0];
            }
            if unnamed_any && !self.mask_overprint {
                self.note_issue(MIXED_OVERPRINT_ISSUE);
            }
        }
        // Ink bytes round like Poppler's colToByte so DeviceCMYK paint lands
        // on exactly round(component * 255) in the output.
        let [r, g, b] = self.ink_bytes(ink);
        let alpha = RenderColor::new(0.0, 0.0, 0.0, alpha).to_pixel_color()[3];
        [r, g, b, alpha]
    }

    /// Whether an overprinting paint in this colour space leaves every plate
    /// of the pass untouched (image and shading paints, whose colour varies
    /// per pixel, are skipped as a whole), and the channels to keep when it
    /// names only some of them and the pass masks overprint per plate.
    pub(crate) fn overprint_skips_space(
        &self,
        space: &PdfObject,
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
    ) -> (bool, Option<[bool; 3]>) {
        let Some(count) = component_count(space, reader, 0) else {
            return (false, None);
        };
        let ones = vec![1.0; count];
        let ink = match self.resolve(space, None, &ones, reader, options, resources, false) {
            InkResult::Ink(ink) => ink,
            InkResult::NoPaint => return (true, None),
            _ => return (false, None),
        };
        if let Some(keep) = self.overprint_keep_mask(&ink) {
            return (false, Some(keep));
        }
        let mut named = 0usize;
        let mut plates = 0usize;
        for &plate in &self.channels {
            if plate == NO_PLATE {
                continue;
            }
            plates += 1;
            if ink.names(plate) {
                named += 1;
            }
        }
        if named > 0 && named < plates {
            self.note_issue(MIXED_OVERPRINT_ISSUE);
        }
        (plates > 0 && named == 0, None)
    }

    /// Ink of a device colour (`g`, `rg`, `k` and friends).
    pub(crate) fn device_ink(family: &str, components: &[f64], opm_nonzero: bool) -> Option<Ink> {
        let unit = |index: usize| -> Option<f32> {
            components
                .get(index)
                .copied()
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.0, 1.0) as f32)
        };
        match family {
            "DeviceGray" | "G" => Some(Ink::gray(unit(0)?)),
            "DeviceRGB" | "RGB" | "sRGB" => Some(Ink::rgb(unit(0)?, unit(1)?, unit(2)?)),
            "DeviceCMYK" | "CMYK" => Some(Ink::cmyk(
                [unit(0)?, unit(1)?, unit(2)?, unit(3)?],
                opm_nonzero,
            )),
            _ => None,
        }
    }

    /// Resolve components of a colour-space object to ink. `opm_nonzero`
    /// applies the OPM 1 rule to a DeviceCMYK space.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve(
        &self,
        space: &PdfObject,
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        opm_nonzero: bool,
    ) -> InkResult {
        self.resolve_at(
            space,
            source,
            components,
            reader,
            options,
            resources,
            opm_nonzero,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_at(
        &self,
        space: &PdfObject,
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        opm_nonzero: bool,
        depth: u8,
    ) -> InkResult {
        if depth > MAX_RESOLVE_DEPTH {
            return InkResult::Invalid("colour space nesting is too deep");
        }
        let resolved;
        let space = if matches!(space, PdfObject::Reference { .. }) {
            resolved = match reader.resolve(space.clone()) {
                Ok(value) => value,
                Err(_) => return InkResult::Invalid("colour space reference cannot be resolved"),
            };
            &resolved
        } else {
            space
        };
        let arr = match space {
            PdfObject::Name(name) => {
                return match Self::device_ink(name, components, opm_nonzero) {
                    Some(ink) => InkResult::Ink(ink),
                    None if matches!(
                        name.as_str(),
                        "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "DeviceCMYK" | "CMYK"
                    ) =>
                    {
                        InkResult::Invalid("malformed device colour components")
                    }
                    None => InkResult::Unhandled,
                };
            }
            PdfObject::Array(arr) => arr.as_slice(),
            _ => return InkResult::Unhandled,
        };
        let Some(family) = arr.first().and_then(PdfObject::as_name) else {
            return InkResult::Unhandled;
        };
        match family {
            "Separation" => {
                self.resolve_separation(arr, source, components, reader, options, resources, depth)
            }
            "DeviceN" => {
                self.resolve_device_n(arr, source, components, reader, options, resources, depth)
            }
            "ICCBased" => match icc_component_count(arr, reader) {
                Some(4) => match Self::device_ink("DeviceCMYK", components, false) {
                    Some(ink) => InkResult::Ink(ink),
                    None => InkResult::Invalid("malformed ICCBased components"),
                },
                Some(1) => match Self::device_ink("DeviceGray", components, false) {
                    Some(ink) => InkResult::Ink(ink),
                    None => InkResult::Invalid("malformed ICCBased components"),
                },
                _ => {
                    self.resolve_through_rgb(space, source, components, reader, options, resources)
                }
            },
            "Indexed" => self.resolve_indexed(
                arr, space, source, components, reader, options, resources, depth,
            ),
            "Pattern" => InkResult::Unhandled,
            _ => self.resolve_through_rgb(space, source, components, reader, options, resources),
        }
    }

    fn resolve_through_rgb(
        &self,
        space: &PdfObject,
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
    ) -> InkResult {
        match crate::render::colorspace::resolve_named_color_with_resources(
            space, source, components, 1.0, reader, options, resources,
        ) {
            NamedColor::Color(color) => InkResult::Ink(Ink::rgb(color.r, color.g, color.b)),
            NamedColor::NoPaint => InkResult::NoPaint,
            NamedColor::Invalid(reason) => InkResult::Invalid(reason),
            NamedColor::Unhandled => InkResult::Unhandled,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_separation(
        &self,
        arr: &[PdfObject],
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        depth: u8,
    ) -> InkResult {
        let Some(colorant) = arr.get(1).and_then(PdfObject::as_name) else {
            return InkResult::Unhandled;
        };
        let Some(tint) = components
            .first()
            .copied()
            .filter(|value| value.is_finite() && components.len() == 1)
        else {
            return InkResult::Invalid("malformed tint components");
        };
        let tint = tint.clamp(0.0, 1.0) as f32;
        match colorant {
            "None" => return InkResult::NoPaint,
            "All" => return InkResult::Ink(Ink::all(tint)),
            _ => {}
        }
        if let Some(plate) = self.direct_plate(colorant) {
            if let (Some(alt), Some(tint_fn)) = (arr.get(2), arr.get(3)) {
                self.record_spot_table(plate, |tint| {
                    process_ink_bytes(self.resolve_alternate(
                        alt,
                        tint_fn,
                        "Separation",
                        source,
                        &[tint],
                        reader,
                        options,
                        resources,
                        depth,
                    ))
                });
            }
            return InkResult::Ink(Ink::direct(&[(plate, tint)]));
        }
        let (Some(alt), Some(tint_fn)) = (arr.get(2), arr.get(3)) else {
            return InkResult::Unhandled;
        };
        self.resolve_alternate(
            alt,
            tint_fn,
            "Separation",
            source,
            &[f64::from(tint)],
            reader,
            options,
            resources,
            depth,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_device_n(
        &self,
        arr: &[PdfObject],
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        depth: u8,
    ) -> InkResult {
        let Some(names) = arr.get(1).and_then(PdfObject::as_array) else {
            return InkResult::Unhandled;
        };
        let names: Option<Vec<&str>> = names.iter().map(PdfObject::as_name).collect();
        let Some(names) = names else {
            return InkResult::Invalid("malformed DeviceN colorant names");
        };
        if names.is_empty() || names.len() > crate::render::colorspace::MAX_DEVICEN_COMPONENTS {
            return InkResult::Unhandled;
        }
        if components.len() != names.len() || components.iter().any(|value| !value.is_finite()) {
            return InkResult::Invalid("malformed tint components");
        }
        if names.iter().all(|name| *name == "None") {
            return InkResult::NoPaint;
        }
        let direct: Option<Vec<(usize, f32)>> = names
            .iter()
            .zip(components)
            .filter(|(name, _)| **name != "None")
            .map(|(name, value)| {
                self.direct_plate(name)
                    .map(|plate| (plate, value.clamp(0.0, 1.0) as f32))
            })
            .collect();
        if let Some(direct) = direct {
            if self.flatten_spots {
                self.record_device_n_tables(arr, &names, source, reader, options, resources, depth);
            }
            return InkResult::Ink(Ink::direct(&direct));
        }
        let (Some(alt), Some(tint_fn)) = (arr.get(2), arr.get(3)) else {
            return InkResult::Unhandled;
        };
        self.resolve_alternate(
            alt, tint_fn, "DeviceN", source, components, reader, options, resources, depth,
        )
    }

    /// Spot ink tables of the separated colorants of a DeviceN space: their
    /// Separation in the `/Colorants` attribute when present, else the
    /// DeviceN alternate with that colorant alone at the tint.
    #[allow(clippy::too_many_arguments)]
    fn record_device_n_tables(
        &self,
        arr: &[PdfObject],
        names: &[&str],
        source: Option<&PdfObject>,
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        depth: u8,
    ) {
        let resolve_dict = |object: &PdfObject| -> Option<crate::object::PdfDictionary> {
            reader.resolve(object.clone()).ok()?.as_dict().cloned()
        };
        let colorants = arr
            .get(4)
            .and_then(resolve_dict)
            .and_then(|attributes| attributes.get("Colorants").and_then(resolve_dict));
        for (index, name) in names.iter().enumerate() {
            let Some(plate) = self.direct_plate(name).filter(|plate| *plate >= 4) else {
                continue;
            };
            let separation = colorants
                .as_ref()
                .and_then(|colorants| colorants.get(name))
                .and_then(|object| reader.resolve(object.clone()).ok())
                .and_then(|object| object.as_array().map(|items| items.to_vec()))
                .filter(|items| items.len() >= 4);
            if let Some(separation) = separation {
                self.record_spot_table(plate, |tint| {
                    process_ink_bytes(self.resolve_alternate(
                        &separation[2],
                        &separation[3],
                        "Separation",
                        None,
                        &[tint],
                        reader,
                        options,
                        resources,
                        depth,
                    ))
                });
            } else if let (Some(alt), Some(tint_fn)) = (arr.get(2), arr.get(3)) {
                self.record_spot_table(plate, |tint| {
                    let mut components = vec![0.0; names.len()];
                    components[index] = tint;
                    process_ink_bytes(self.resolve_alternate(
                        alt,
                        tint_fn,
                        "DeviceN",
                        source,
                        &components,
                        reader,
                        options,
                        resources,
                        depth,
                    ))
                });
            }
        }
    }

    /// Spot colours this pass set does not separate: tint transform to the
    /// alternate space, then that space's ink. The overprint mask is what the
    /// alternate colour at full tint inks.
    #[allow(clippy::too_many_arguments)]
    fn resolve_alternate(
        &self,
        alt: &PdfObject,
        tint_fn: &PdfObject,
        family: &str,
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        depth: u8,
    ) -> InkResult {
        let source_alt = source
            .and_then(|source| reader.resolve(source.clone()).ok())
            .filter(|source| crate::render::default_colorspace::family(source) == Some(family))
            .and_then(|source| source.as_array().and_then(|items| items.get(2)).cloned());
        let evaluate = |inputs: &[f64]| -> Result<Vec<f64>, InkResult> {
            crate::render::colorspace::evaluate_tint_transform_with_resources(
                tint_fn, inputs, reader, resources,
            )
            .map_err(|invalid| match invalid {
                NamedColor::Invalid(reason) => InkResult::Invalid(reason),
                _ => InkResult::Invalid("malformed tint transform"),
            })
        };
        let alt_components = match evaluate(components) {
            Ok(values) => values,
            Err(result) => return result,
        };
        let mut ink = match self.resolve_at(
            alt,
            source_alt.as_ref(),
            &alt_components,
            reader,
            options,
            resources,
            false,
            depth + 1,
        ) {
            InkResult::Ink(ink) => ink,
            other => return other,
        };
        let full = vec![1.0; components.len()];
        let named = evaluate(&full).ok().and_then(|values| {
            match self.resolve_at(
                alt,
                source_alt.as_ref(),
                &values,
                reader,
                options,
                resources,
                false,
                depth + 1,
            ) {
                InkResult::Ink(full_ink) => Some(full_ink.inked_mask()),
                _ => None,
            }
        });
        ink.named = named.unwrap_or(PROCESS_MASK);
        ink.all = None;
        InkResult::Ink(ink)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_indexed(
        &self,
        arr: &[PdfObject],
        space: &PdfObject,
        source: Option<&PdfObject>,
        components: &[f64],
        reader: &PdfReader,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
        depth: u8,
    ) -> InkResult {
        let (Some(base), Some(hival), Some(lookup)) = (
            arr.get(1),
            arr.get(2).and_then(PdfObject::as_number),
            arr.get(3),
        ) else {
            return InkResult::Invalid("malformed Indexed color space");
        };
        let base_resolved = reader
            .resolve(base.clone())
            .unwrap_or_else(|_| base.clone());
        let base_family = crate::render::default_colorspace::family(&base_resolved);
        if matches!(base_family, Some("Lab" | "CalRGB" | "CalGray")) {
            return self.resolve_through_rgb(space, source, components, reader, options, resources);
        }
        let Some(count) = component_count(&base_resolved, reader, depth + 1) else {
            return InkResult::Invalid("malformed Indexed base color space");
        };
        let Some(index) = components
            .first()
            .copied()
            .filter(|value| value.is_finite())
        else {
            return InkResult::Invalid("malformed Indexed components");
        };
        let index = index.round().clamp(0.0, hival.max(0.0)) as usize;
        let bytes = match crate::render::colorspace::indexed_lookup_bytes(lookup, reader) {
            Ok(bytes) => bytes,
            Err(_) => return InkResult::Invalid("malformed Indexed lookup"),
        };
        let start = index * count;
        let Some(entry) = bytes.get(start..start + count) else {
            return InkResult::Invalid("Indexed lookup is too short");
        };
        let base_components: Vec<f64> = entry.iter().map(|&byte| f64::from(byte) / 255.0).collect();
        self.resolve_at(
            &base_resolved,
            None,
            &base_components,
            reader,
            options,
            resources,
            false,
            depth + 1,
        )
    }

    /// Convert decoded image samples (one byte per component, already in the
    /// colour space's unit domain with `/Decode` applied) to plate pixels.
    /// Returns `None` for spaces whose samples are not unit-domain bytes;
    /// callers convert those through RGB with [`Self::convert_device_image`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn convert_samples(
        &self,
        samples: &[u8],
        channels: usize,
        family: &str,
        space: Option<&PdfObject>,
        reader: Option<&PdfReader>,
        options: ColorTransformOptions,
        resources: FunctionResources<'_>,
    ) -> Option<(Vec<u8>, u8)> {
        if channels == 0 {
            return None;
        }
        let device = match (family, channels) {
            ("DeviceGray" | "G", 1) => Some(DeviceSamples::Gray),
            ("DeviceRGB" | "RGB" | "sRGB", 3) => Some(DeviceSamples::Rgb),
            ("DeviceCMYK" | "CMYK", 4) => Some(DeviceSamples::Cmyk),
            ("ICCBased", 1) => Some(DeviceSamples::Gray),
            ("ICCBased", 4) => Some(DeviceSamples::Cmyk),
            _ => None,
        };
        if let Some(device) = device {
            return Some(self.convert_device_samples(samples, device));
        }
        if !matches!(family, "Separation" | "DeviceN") {
            return None;
        }
        let (space, reader) = (space?, reader?);
        let mut memo: HashMap<u64, [u8; 3]> = HashMap::new();
        const MAX_MEMO: usize = 1 << 16;
        let single = self.single_plate();
        let out_channels = if single { 1 } else { 3 };
        let mut out = Vec::with_capacity(samples.len() / channels * out_channels);
        let mut values = vec![0.0f64; channels];
        for chunk in samples.chunks_exact(channels) {
            let key = (channels <= 8).then(|| {
                chunk
                    .iter()
                    .fold(0u64, |key, &byte| (key << 8) | u64::from(byte))
            });
            let pixel = match key.and_then(|key| memo.get(&key).copied()) {
                Some(pixel) => pixel,
                None => {
                    for (value, &byte) in values.iter_mut().zip(chunk) {
                        *value = f64::from(byte) / 255.0;
                    }
                    let pixel = match self
                        .resolve(space, None, &values, reader, options, resources, false)
                    {
                        InkResult::Ink(ink) => self.ink_bytes(&ink),
                        InkResult::NoPaint => [255, 255, 255],
                        _ => return None,
                    };
                    if let Some(key) = key {
                        if memo.len() < MAX_MEMO {
                            memo.insert(key, pixel);
                        }
                    }
                    pixel
                }
            };
            if single {
                out.push(pixel[0]);
            } else {
                out.extend_from_slice(&pixel);
            }
        }
        Some((out, out_channels as u8))
    }

    fn convert_device_samples(&self, samples: &[u8], device: DeviceSamples) -> (Vec<u8>, u8) {
        let step = match device {
            DeviceSamples::Gray => 1,
            DeviceSamples::Rgb => 3,
            DeviceSamples::Cmyk => 4,
        };
        let single = self.single_plate();
        let out_channels = if single { 1 } else { 3 };
        let mut out = Vec::with_capacity(samples.len() / step * out_channels);
        let channels = self.channels;
        let plate_byte = |chunk: &[u8], plate: usize| -> u8 {
            if plate == NO_PLATE || plate >= 4 {
                return 255;
            }
            match device {
                DeviceSamples::Gray => {
                    if plate == 3 {
                        chunk[0]
                    } else {
                        255
                    }
                }
                DeviceSamples::Cmyk => 255 - chunk[plate],
                DeviceSamples::Rgb => 255 - rgb_bytes_to_cmyk(chunk[0], chunk[1], chunk[2])[plate],
            }
        };
        for chunk in samples.chunks_exact(step) {
            if single {
                out.push(plate_byte(chunk, channels[0]));
            } else {
                for &plate in &channels {
                    out.push(plate_byte(chunk, plate));
                }
            }
        }
        (out, out_channels as u8)
    }

    /// Convert a device-referred decoded image (1 = gray, 2 = gray + alpha,
    /// 3 = RGB, 4 = RGBA) to plate pixels, keeping any alpha channel.
    pub(crate) fn convert_device_image(&self, pixels: &[u8], channels: u8) -> (Vec<u8>, u8) {
        let (device, color_step) = match channels {
            1 | 2 => (DeviceSamples::Gray, 1usize),
            _ => (DeviceSamples::Rgb, 3usize),
        };
        let step = usize::from(channels.max(1));
        let has_alpha = matches!(channels, 2 | 4);
        let single = self.single_plate();
        let color_out = if single { 1 } else { 3 };
        // Painters accept 1, 3 and 4 channels: alpha needs RGBA.
        let out_channels = match (has_alpha, single) {
            (false, true) => 1,
            (false, false) => 3,
            (true, _) => 4,
        };
        let mut out = Vec::with_capacity(pixels.len() / step * out_channels);
        for chunk in pixels.chunks_exact(step) {
            let (color, _) = self.convert_device_samples(&chunk[..color_step], device);
            if has_alpha {
                if color_out == 1 {
                    out.extend_from_slice(&[color[0], color[0], color[0]]);
                } else {
                    out.extend_from_slice(&color);
                }
                out.push(chunk[step - 1]);
            } else {
                out.extend_from_slice(&color);
            }
        }
        (out, out_channels as u8)
    }

    /// Inverted ink bytes of the three channels.
    pub(crate) fn ink_bytes(&self, ink: &Ink) -> [u8; 3] {
        let mut out = [255u8; 3];
        for (byte, &plate) in out.iter_mut().zip(self.channels.iter()) {
            if plate != NO_PLATE {
                *byte = 255 - unit_to_byte(ink.value(plate));
            }
        }
        out
    }
}

#[derive(Clone, Copy)]
enum DeviceSamples {
    Gray,
    Rgb,
    Cmyk,
}

/// Ink of one paint colour on every output plate.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Ink {
    cmyk: [f32; 4],
    /// Separated spot plates (`4 + index`) and directly named process plates.
    direct: Vec<(usize, f32)>,
    /// `/Separation /All`: every plate at this tint.
    all: Option<f32>,
    /// Plates named by the colour space (bit per plate) for overprint.
    named: u64,
}

impl Ink {
    fn gray(gray: f32) -> Self {
        Self {
            cmyk: [0.0, 0.0, 0.0, 1.0 - gray],
            named: PROCESS_MASK,
            ..Self::default()
        }
    }

    fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self {
            cmyk: rgb_to_cmyk(r, g, b),
            named: PROCESS_MASK,
            ..Self::default()
        }
    }

    fn cmyk(cmyk: [f32; 4], opm_nonzero: bool) -> Self {
        let named = if opm_nonzero {
            cmyk.iter()
                .enumerate()
                .filter(|(_, value)| **value > 0.0)
                .fold(0u64, |mask, (index, _)| mask | (1 << index))
        } else {
            PROCESS_MASK
        };
        Self {
            cmyk,
            named,
            ..Self::default()
        }
    }

    fn all(tint: f32) -> Self {
        Self {
            all: Some(tint),
            named: u64::MAX,
            ..Self::default()
        }
    }

    fn direct(entries: &[(usize, f32)]) -> Self {
        let mut ink = Self::default();
        for &(plate, value) in entries {
            if plate < 4 {
                ink.cmyk[plate] = value;
            } else {
                ink.direct.push((plate, value));
            }
            if plate < 64 {
                ink.named |= 1 << plate;
            }
        }
        ink
    }

    pub(crate) fn value(&self, plate: usize) -> f32 {
        if let Some(tint) = self.all {
            return tint;
        }
        if plate < 4 {
            return self.cmyk[plate];
        }
        self.direct
            .iter()
            .find(|(index, _)| *index == plate)
            .map_or(0.0, |(_, value)| *value)
    }

    pub(crate) fn names(&self, plate: usize) -> bool {
        plate < 64 && self.named & (1 << plate) != 0
    }

    /// Plates carrying any ink.
    fn inked_mask(&self) -> u64 {
        if self.all.is_some_and(|tint| tint > 0.0) {
            return u64::MAX;
        }
        let mut mask = self
            .cmyk
            .iter()
            .enumerate()
            .filter(|(_, value)| **value > 0.0)
            .fold(0u64, |mask, (index, _)| mask | (1 << index));
        for &(plate, value) in &self.direct {
            if value > 0.0 && plate < 64 {
                mask |= 1 << plate;
            }
        }
        mask
    }
}

/// Outcome of resolving a colour to ink.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InkResult {
    Ink(Ink),
    NoPaint,
    Invalid(&'static str),
    Unhandled,
}

/// Poppler's uncalibrated DeviceRGB to DeviceCMYK conversion
/// (`GfxDeviceRGBColorSpace::getCMYK`): full undercolour removal.
pub(crate) fn rgb_to_cmyk(r: f32, g: f32, b: f32) -> [f32; 4] {
    let c = (1.0 - r).clamp(0.0, 1.0);
    let m = (1.0 - g).clamp(0.0, 1.0);
    let y = (1.0 - b).clamp(0.0, 1.0);
    let k = c.min(m).min(y);
    [c - k, m - k, y - k, k]
}

/// Byte form of [`rgb_to_cmyk`], exact in integers.
pub(crate) fn rgb_bytes_to_cmyk(r: u8, g: u8, b: u8) -> [u8; 4] {
    let (c, m, y) = (255 - r, 255 - g, 255 - b);
    let k = c.min(m).min(y);
    [c - k, m - k, y - k, k]
}

fn unit_to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn icc_component_count(arr: &[PdfObject], reader: &PdfReader) -> Option<usize> {
    let stream = reader.resolve(arr.get(1)?.clone()).ok()?;
    let dict = match &stream {
        PdfObject::Stream { dict, .. } => dict,
        PdfObject::Dictionary(dict) => dict,
        _ => return None,
    };
    let n = match dict.get("N")? {
        PdfObject::Reference { .. } => reader.resolve(dict.get("N")?.clone()).ok()?.as_number()?,
        other => other.as_number()?,
    };
    (n >= 1.0 && n.fract() == 0.0).then_some(n as usize)
}

/// Number of colour components of a colour-space object.
pub(crate) fn component_count(space: &PdfObject, reader: &PdfReader, depth: u8) -> Option<usize> {
    if depth > MAX_RESOLVE_DEPTH {
        return None;
    }
    let space = reader.resolve(space.clone()).ok()?;
    match &space {
        PdfObject::Name(name) => match name.as_str() {
            "DeviceGray" | "G" | "CalGray" => Some(1),
            "DeviceRGB" | "RGB" | "CalRGB" | "Lab" => Some(3),
            "DeviceCMYK" | "CMYK" => Some(4),
            _ => None,
        },
        PdfObject::Array(arr) => match arr.first()?.as_name()? {
            "Separation" | "Indexed" | "CalGray" => Some(1),
            "CalRGB" | "Lab" => Some(3),
            "DeviceN" => arr.get(1)?.as_array().map(<[PdfObject]>::len),
            "ICCBased" => icc_component_count(arr, reader),
            "DeviceGray" => Some(1),
            "DeviceRGB" => Some(3),
            "DeviceCMYK" => Some(4),
            _ => None,
        },
        _ => None,
    }
}

/// How the plates of a page are grouped into render passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassLayout {
    /// Three plates per pass: C/M/Y, then K with the first two spots, ...
    Grouped,
    /// C/M/Y together, every other plate alone (non-separable blend modes
    /// with spot plates).
    ProcessTogether,
    /// One plate per pass (an overprinting paint names only some of the
    /// plates of a pass).
    Single,
}

/// Plate channel sets covering `plates` plates in `layout`. A plate alone in
/// its pass fills all three channels.
pub(crate) fn plate_pass_channels(plates: usize, layout: PassLayout) -> Vec<[usize; 3]> {
    match layout {
        PassLayout::Grouped => {}
        PassLayout::ProcessTogether => {
            let mut passes = vec![[0, 1, 2]];
            passes.extend((3..plates).map(|plate| [plate; 3]));
            return passes;
        }
        PassLayout::Single => return (0..plates).map(|plate| [plate; 3]).collect(),
    }
    (0..plates)
        .step_by(3)
        .map(|first| {
            if first + 1 >= plates {
                return [first; 3];
            }
            let at = |offset: usize| {
                if first + offset < plates {
                    first + offset
                } else {
                    NO_PLATE
                }
            };
            [at(0), at(1), at(2)]
        })
        .collect()
}

#[cfg(test)]
#[path = "plate_render_tests.rs"]
mod render_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn target(channels: [usize; 3], spots: &[&str]) -> PlateTarget {
        PlateTarget::new(
            channels,
            spots.iter().map(|spot| spot.to_string()).collect(),
            true,
        )
    }

    #[test]
    fn rgb_conversion_matches_poppler_undercolour_removal() {
        assert_eq!(rgb_bytes_to_cmyk(255, 255, 255), [0, 0, 0, 0]);
        assert_eq!(rgb_bytes_to_cmyk(0, 0, 0), [0, 0, 0, 255]);
        assert_eq!(rgb_bytes_to_cmyk(255, 0, 0), [0, 255, 255, 0]);
        assert_eq!(rgb_bytes_to_cmyk(128, 128, 128), [0, 0, 0, 127]);
        let float = rgb_to_cmyk(1.0, 0.0, 0.0);
        assert_eq!(float, [0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn device_cmyk_paints_exact_inverted_ink() {
        let plates = target([0, 1, 2], &[]);
        let ink = PlateTarget::device_ink("DeviceCMYK", &[0.2, 0.4, 0.0, 1.0], false).unwrap();
        let pixel = plates.pixel(&ink, 1.0, false);
        assert_eq!(pixel, [255 - 51, 255 - 102, 255, 255]);
        let black = target([3, 3, 3], &[]);
        assert_eq!(black.pixel(&ink, 0.5, false), [0, 0, 0, 128]);
    }

    #[test]
    fn gray_is_black_ink_only() {
        let ink = PlateTarget::device_ink("DeviceGray", &[0.25], false).unwrap();
        assert_eq!(ink.value(0), 0.0);
        assert_eq!(ink.value(3), 0.75);
    }

    #[test]
    fn overprint_mode_one_leaves_zero_components_untouched() {
        let cyan = target([0, 0, 0], &[]);
        let magenta = target([1, 1, 1], &[]);
        let ink = PlateTarget::device_ink("DeviceCMYK", &[0.0, 1.0, 0.0, 0.0], true).unwrap();
        assert_eq!(cyan.pixel(&ink, 1.0, true), [0, 0, 0, 0]);
        assert_eq!(magenta.pixel(&ink, 1.0, true), [0, 0, 0, 255]);
        // Without overprint the zero cyan knocks out.
        assert_eq!(cyan.pixel(&ink, 1.0, false), [255, 255, 255, 255]);
    }

    #[test]
    fn mixed_overprint_in_one_pass_is_reported() {
        let cmy = target([0, 1, 2], &[]);
        let ink = PlateTarget::device_ink("DeviceCMYK", &[0.0, 1.0, 0.0, 0.0], true).unwrap();
        let _ = cmy.pixel(&ink, 1.0, true);
        assert_eq!(cmy.issues(), vec![MIXED_OVERPRINT_ISSUE.to_string()]);
    }

    #[test]
    fn separated_spot_and_process_colorants_paint_their_plates() {
        let plates = target([0, 3, 4], &["PANTONE 123 C"]);
        let spot = Ink::direct(&[(4, 0.5)]);
        assert_eq!(plates.pixel(&spot, 1.0, false), [255, 255, 127, 255]);
        assert!(plates.issues().is_empty());
        // Overprinting a spot leaves the process plates of the pass untouched,
        // which one RGB pass cannot express: reported for a per-plate rerun.
        let _ = plates.pixel(&spot, 1.0, true);
        assert_eq!(plates.issues(), vec![MIXED_OVERPRINT_ISSUE.to_string()]);
        let spot_only = target([4, 4, 4], &["PANTONE 123 C"]);
        assert_eq!(spot_only.pixel(&spot, 1.0, true), [127, 127, 127, 255]);
        let cyan_only = target([0, 0, 0], &["PANTONE 123 C"]);
        assert_eq!(cyan_only.pixel(&spot, 1.0, true), [0, 0, 0, 0]);
        assert_eq!(cyan_only.pixel(&spot, 1.0, false), [255, 255, 255, 255]);
        assert_eq!(plates.direct_plate("Black"), Some(3));
        assert_eq!(plates.direct_plate("PANTONE 123 C"), Some(4));
        assert_eq!(plates.direct_plate("Orange"), None);
        let all = Ink::all(0.25);
        let cmy = target([0, 1, 2], &[]);
        assert_eq!(cmy.pixel(&all, 1.0, true), [191, 191, 191, 255]);
        assert!(cmy.issues().is_empty());
    }

    #[test]
    fn device_samples_convert_per_plate() {
        let k = target([3, 3, 3], &[]);
        let (gray, channels) = k.convert_device_image(&[0, 128, 255], 1);
        assert_eq!((gray, channels), (vec![0, 128, 255], 1));
        let cmy = target([0, 1, 2], &[]);
        let (rgb, channels) = cmy.convert_device_image(&[255, 0, 0], 3);
        assert_eq!((rgb, channels), (vec![255, 0, 0], 3));
        let (cmyk, channels) = cmy
            .convert_samples(
                &[10, 20, 30, 40],
                4,
                "DeviceCMYK",
                None,
                None,
                ColorTransformOptions::default(),
                FunctionResources::default(),
            )
            .unwrap();
        assert_eq!((cmyk, channels), (vec![245, 235, 225], 3));
        let (rgba, channels) = k.convert_device_image(&[0, 0, 0, 77], 4);
        assert_eq!((rgba, channels), (vec![0, 0, 0, 77], 4));
    }

    #[test]
    fn pass_channels_group_plates_three_at_a_time() {
        use PassLayout::{Grouped, ProcessTogether, Single};
        assert_eq!(plate_pass_channels(4, Grouped), vec![[0, 1, 2], [3, 3, 3]]);
        assert_eq!(plate_pass_channels(4, Single).len(), 4);
        assert_eq!(plate_pass_channels(6, Grouped), vec![[0, 1, 2], [3, 4, 5]]);
        assert_eq!(
            plate_pass_channels(5, Grouped),
            vec![[0, 1, 2], [3, 4, NO_PLATE]]
        );
        assert_eq!(
            plate_pass_channels(6, ProcessTogether),
            vec![[0, 1, 2], [3, 3, 3], [4, 4, 4], [5, 5, 5]]
        );
    }

    #[test]
    fn non_separable_blend_needs_process_plates_together() {
        let cmy = target([0, 1, 2], &[]);
        cmy.check_blend_mode(true);
        let black = target([3, 3, 3], &[]);
        black.check_blend_mode(true);
        let spot = target([4, 4, 4], &["Gold"]);
        spot.check_blend_mode(true);
        assert!(cmy.issues().is_empty());
        assert!(black.issues().is_empty());
        assert!(spot.issues().is_empty());
        let mixed = target([3, 4, 5], &["Gold", "Silver"]);
        mixed.check_blend_mode(false);
        assert!(mixed.issues().is_empty());
        mixed.check_blend_mode(true);
        assert_eq!(mixed.issues(), vec![NONSEPARABLE_LAYOUT_ISSUE.to_string()]);
        // One process plate per pass (overprint): C/M/Y go back together
        // with per-plate overprint masking.
        let cyan = target([0, 0, 0], &[]);
        cyan.check_blend_mode(true);
        assert_eq!(cyan.issues(), vec![NONSEPARABLE_LAYOUT_ISSUE.to_string()]);
    }

    #[test]
    fn overprint_masking_keeps_the_unnamed_process_plates() {
        // OPM 1 with a zero cyan component names magenta, yellow and black.
        let ink = PlateTarget::device_ink("DeviceCMYK", &[0.0, 1.0, 0.5, 0.0], true).unwrap();
        let plain = target([0, 1, 2], &[]);
        assert_eq!(plain.overprint_keep_mask(&ink), None);
        plain.pixel(&ink, 1.0, true);
        assert_eq!(plain.issues(), vec![MIXED_OVERPRINT_ISSUE.to_string()]);
        let masked = target([0, 1, 2], &[]).with_overprint_masking(true);
        assert_eq!(masked.overprint_keep_mask(&ink), Some([true, false, false]));
        assert_eq!(masked.pixel(&ink, 1.0, true), [255, 0, 127, 255]);
        assert!(masked.issues().is_empty());
        // Every plate named, or none: nothing to mask.
        let full = PlateTarget::device_ink("DeviceCMYK", &[0.1, 1.0, 0.5, 0.0], true).unwrap();
        assert_eq!(masked.overprint_keep_mask(&full), None);
        let black = PlateTarget::device_ink("DeviceCMYK", &[0.0, 0.0, 0.0, 1.0], true).unwrap();
        assert_eq!(masked.overprint_keep_mask(&black), None);
        // A single-plate pass never masks.
        let single = target([0, 0, 0], &[]).with_overprint_masking(true);
        assert_eq!(single.overprint_keep_mask(&ink), None);
    }

    #[test]
    fn spot_plates_flatten_additively_like_poppler() {
        let mut table = [[0u8; 4]; 256];
        table[255] = [0, 61, 240, 0];
        table[128] = [0, 31, 120, 0];
        assert_eq!(
            flatten_spot_inks([0, 102, 0, 0], [(255, &table)]),
            [0, 163, 240, 0]
        );
        // Clamped at full ink.
        assert_eq!(
            flatten_spot_inks([0, 250, 100, 0], [(255, &table), (128, &table)]),
            [0, 255, 255, 0]
        );
        // No spot ink keeps the process ink.
        assert_eq!(flatten_spot_inks([1, 2, 3, 4], [(0, &table)]), [1, 2, 3, 4]);
    }

    #[test]
    fn transfer_curves_map_to_plates() {
        let mut channels = [[0u8; 256]; 4];
        for (curve, table) in channels.iter_mut().enumerate() {
            for (index, value) in table.iter_mut().enumerate() {
                *value = (index as u8).saturating_sub(curve as u8 + 1);
            }
        }
        let lut = crate::content::state::TransferLut { channels };
        let cmy = target([0, 1, 2], &[]).transfer_for_pass(&lut);
        assert_eq!(cmy.apply_rgba([200, 200, 200, 255]), [199, 198, 197, 255]);
        let k_spot = target([3, 4, NO_PLATE], &["Gold"]).transfer_for_pass(&lut);
        assert_eq!(k_spot.apply_rgba([200, 200, 200, 9]), [196, 196, 200, 9]);
    }

    #[test]
    fn srgb_colours_convert_to_plate_ink() {
        let cmy = target([0, 1, 2], &[]);
        assert_eq!(cmy.srgb_pixel([255, 0, 0, 128]), [255, 0, 0, 128]);
        let black = target([3, 3, 3], &[]);
        assert_eq!(black.srgb_pixel([64, 64, 64, 255]), [64, 64, 64, 255]);
        let spot = target([4, 4, 4], &["Gold"]);
        assert_eq!(spot.srgb_pixel([0, 0, 0, 255]), [255, 255, 255, 255]);
    }
}
