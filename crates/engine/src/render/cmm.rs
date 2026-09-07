//! Small, deterministic color-management helpers for PDF render output.
//!
//! Embedded ICC profiles are handled with the portable `qcms` fallback in
//! default builds. When the explicit `native-cmm-lcms2` feature is enabled,
//! ICCBased preview transforms use the safe `lcms2` wrapper around
//! LittleCMS/lcms2. Device spaces still need local fallbacks because PDF
//! DeviceCMYK/Cal/Lab often appear without an ICC profile.

use crate::filters::{decode_stream_lossless, StreamDecodeStatus};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::size_of;

const D50: [f32; 3] = [0.96422, 1.0, 0.82521];
pub(crate) const DEFAULT_MAX_ICC_PROFILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const DEFAULT_TRANSFORM_CACHE_ENTRIES: usize = 16;
pub(crate) const DEFAULT_TRANSFORM_CACHE_ENTRY_BYTES: usize =
    DEFAULT_MAX_ICC_PROFILE_BYTES + 64 * 1024;
pub(crate) const DEFAULT_TRANSFORM_CACHE_BYTES: usize =
    DEFAULT_TRANSFORM_CACHE_ENTRIES * DEFAULT_TRANSFORM_CACHE_ENTRY_BYTES;
pub(crate) const NATIVE_CMM_FEATURE_FLAG: &str = "native-cmm-lcms2";
pub(crate) const LCMS2_CRATE_VERSION: &str = "6.1.1";
pub(crate) const LCMS2_SYS_CRATE_VERSION: &str = "4.0.7";
const ICC_TRANSFORM_KIND_PROFILE_TO_SRGB: u8 = 0;
const ICC_TRANSFORM_KIND_BUILTIN_SRGB_PROOF: u8 = 1;
#[cfg(feature = "native-cmm-lcms2")]
const ICC_TRANSFORM_KIND_OUTPUT_INTENT_PROOF: u8 = 2;

thread_local! {
    static ICC_TRANSFORM_CACHE: RefCell<IccTransformCache> =
        RefCell::new(IccTransformCache::new(DEFAULT_TRANSFORM_CACHE_ENTRIES));
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum ColorIntent {
    #[default]
    Perceptual,
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

impl ColorIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Perceptual => "perceptual",
            Self::RelativeColorimetric => "relative_colorimetric",
            Self::Saturation => "saturation",
            Self::AbsoluteColorimetric => "absolute_colorimetric",
        }
    }

    pub(crate) fn from_pdf_name(name: &str) -> Self {
        let normalized = name
            .trim_start_matches('/')
            .chars()
            .map(|ch| if matches!(ch, '-' | ' ') { '_' } else { ch })
            .collect::<String>()
            .to_ascii_lowercase();
        match normalized.as_str() {
            "absolute_colorimetric" | "absolutecolorimetric" => Self::AbsoluteColorimetric,
            "perceptual" => Self::Perceptual,
            "saturation" => Self::Saturation,
            "relative_colorimetric" | "relativecolorimetric" => Self::RelativeColorimetric,
            _ => Self::RelativeColorimetric,
        }
    }

    fn to_qcms(self) -> qcms::Intent {
        match self {
            Self::Perceptual => qcms::Intent::Perceptual,
            Self::RelativeColorimetric => qcms::Intent::RelativeColorimetric,
            Self::Saturation => qcms::Intent::Saturation,
            Self::AbsoluteColorimetric => qcms::Intent::AbsoluteColorimetric,
        }
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn to_lcms2(self) -> lcms2::Intent {
        match self {
            Self::Perceptual => lcms2::Intent::Perceptual,
            Self::RelativeColorimetric => lcms2::Intent::RelativeColorimetric,
            Self::Saturation => lcms2::Intent::Saturation,
            Self::AbsoluteColorimetric => lcms2::Intent::AbsoluteColorimetric,
        }
    }
}

pub(crate) const SUPPORTED_QCMS_INTENTS: [ColorIntent; 4] = [
    ColorIntent::Perceptual,
    ColorIntent::RelativeColorimetric,
    ColorIntent::Saturation,
    ColorIntent::AbsoluteColorimetric,
];

pub(crate) const SUPPORTED_NATIVE_LCMS2_INTENTS: [ColorIntent; 4] = SUPPORTED_QCMS_INTENTS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct ColorTransformOptions {
    pub intent: ColorIntent,
    pub black_point_compensation: bool,
    pub backend: ColorTransformBackend,
    /// Render-contract/output-profile cache scope. Zero keeps standalone
    /// color helpers deterministic; page rendering salts this with the active
    /// schema-v1 render-contract fingerprint so display/print/proof transforms
    /// never share a cached ICC transform accidentally.
    pub cache_scope: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum ColorTransformBackend {
    #[default]
    PortableQcms,
    NativeLittleCms,
    DeterministicFallback,
}

impl ColorTransformBackend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PortableQcms => "portable-qcms",
            Self::NativeLittleCms => "native-littlecms",
            Self::DeterministicFallback => "deterministic-fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct IccTransformCacheMetrics {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub admissions: usize,
    pub rejections: usize,
    pub entries: usize,
    pub max_entries: usize,
    pub bytes_used: usize,
    pub max_bytes: usize,
    pub invalid_profiles: usize,
    pub unsupported_profiles: usize,
    pub native_lcms2_transforms: usize,
    pub native_lcms2_failures: usize,
    pub fallback_qcms_transforms: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCmmStatus {
    pub feature_flag: &'static str,
    pub compiled: bool,
    pub available: bool,
    pub backend_name: &'static str,
    pub selected_backend: &'static str,
    pub native_version: Option<String>,
    pub default_build_native_dependency: bool,
    pub wasm_native_unavailable: bool,
    pub unsafe_boundary: &'static str,
    pub linking_posture: &'static str,
}

pub(crate) fn native_cmm_status() -> NativeCmmStatus {
    let compiled = cfg!(feature = "native-cmm-lcms2");
    let available = compiled && !cfg!(target_arch = "wasm32");
    NativeCmmStatus {
        feature_flag: NATIVE_CMM_FEATURE_FLAG,
        compiled,
        available,
        backend_name: if available { "lcms2" } else { "qcms-fallback" },
        selected_backend: if available { "lcms2" } else { "fallback/qcms" },
        native_version: lcms2_version_string(),
        default_build_native_dependency: false,
        wasm_native_unavailable: cfg!(target_arch = "wasm32") || !compiled,
        unsafe_boundary: if compiled {
            "wellfriendpdf-engine remains forbid(unsafe_code); unsafe/native FFI is isolated in lcms2/lcms2-sys dependencies"
        } else {
            "no native CMM dependency compiled"
        },
        linking_posture: if compiled {
            "lcms2-sys dynamic discovery with static-fallback vendored LittleCMS when system lcms2 is unavailable"
        } else {
            "no lcms2 link in default build"
        },
    }
}

fn lcms2_version_string() -> Option<String> {
    #[cfg(feature = "native-cmm-lcms2")]
    {
        Some(format!(
            "lcms2 crate {LCMS2_CRATE_VERSION}; lcms2-sys {LCMS2_SYS_CRATE_VERSION}; encoded LittleCMS {}",
            lcms2::version()
        ))
    }
    #[cfg(not(feature = "native-cmm-lcms2"))]
    {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum IccBackend {
    FallbackQcms,
    #[cfg(feature = "native-cmm-lcms2")]
    NativeLcms2,
}

impl IccBackend {
    fn from_policy(policy: ColorTransformBackend) -> Option<Self> {
        match policy {
            ColorTransformBackend::PortableQcms => Some(Self::FallbackQcms),
            ColorTransformBackend::DeterministicFallback => None,
            ColorTransformBackend::NativeLittleCms => {
                #[cfg(all(feature = "native-cmm-lcms2", not(target_arch = "wasm32")))]
                {
                    return Some(Self::NativeLcms2);
                }
                #[allow(unreachable_code)]
                None
            }
        }
    }

    fn tag(self) -> u8 {
        match self {
            Self::FallbackQcms => 0,
            #[cfg(feature = "native-cmm-lcms2")]
            Self::NativeLcms2 => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IccFidelityProbe {
    pub name: &'static str,
    pub backend: &'static str,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    pub max_abs_error: u8,
    pub tolerance: u8,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IccTransformKey {
    kind: u8,
    backend: u8,
    cache_scope: u64,
    profile_digest: [u8; 32],
    profile_len: usize,
    components: u8,
    src_type: u8,
    dst_type: u8,
    intent: ColorIntent,
    black_point_compensation: bool,
}

enum CachedIccTransform {
    Qcms(qcms::Transform),
    #[cfg(feature = "native-cmm-lcms2")]
    Lcms2(lcms2::Transform<u8, u8>),
}

impl CachedIccTransform {
    fn convert(&self, src: &[u8], dst: &mut [u8]) {
        match self {
            Self::Qcms(transform) => transform.convert(src, dst),
            #[cfg(feature = "native-cmm-lcms2")]
            Self::Lcms2(transform) => transform.transform_pixels(src, dst),
        }
    }
}

fn icc_transform_entry_bytes(key: &IccTransformKey) -> usize {
    key.profile_len
        .saturating_add(DEFAULT_TRANSFORM_CACHE_ENTRY_BYTES - DEFAULT_MAX_ICC_PROFILE_BYTES)
        .saturating_add(size_of::<IccTransformKey>())
}

fn icc_profile_digest(profile_bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(profile_bytes).into()
}

struct IccTransformCacheEntry {
    key: IccTransformKey,
    transform: CachedIccTransform,
    byte_cost: usize,
}

pub(crate) struct IccTransformCache {
    max_entries: usize,
    max_bytes: usize,
    current_bytes: usize,
    entries: Vec<IccTransformCacheEntry>,
    metrics: IccTransformCacheMetrics,
}

impl IccTransformCache {
    pub(crate) fn new(max_entries: usize) -> Self {
        let max_entries = max_entries.max(1);
        let max_bytes = if max_entries == DEFAULT_TRANSFORM_CACHE_ENTRIES {
            DEFAULT_TRANSFORM_CACHE_BYTES
        } else {
            max_entries.saturating_mul(DEFAULT_TRANSFORM_CACHE_ENTRY_BYTES)
        };
        Self::new_with_budget(max_entries, max_bytes)
    }

    fn new_with_budget(max_entries: usize, max_bytes: usize) -> Self {
        let max_entries = max_entries.max(1);
        let max_bytes = max_bytes.max(1);
        Self {
            max_entries,
            max_bytes,
            current_bytes: 0,
            entries: Vec::new(),
            metrics: IccTransformCacheMetrics {
                max_entries,
                max_bytes,
                ..IccTransformCacheMetrics::default()
            },
        }
    }

    pub(crate) fn metrics(&self) -> IccTransformCacheMetrics {
        IccTransformCacheMetrics {
            entries: self.entries.len(),
            max_entries: self.max_entries,
            bytes_used: self.current_bytes,
            max_bytes: self.max_bytes,
            ..self.metrics
        }
    }

    fn admit(
        &mut self,
        key: IccTransformKey,
        transform: CachedIccTransform,
        byte_cost: usize,
    ) -> Result<usize, CachedIccTransform> {
        if byte_cost == 0 || byte_cost > self.max_bytes {
            self.metrics.rejections += 1;
            return Err(transform);
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.current_bytes.saturating_add(byte_cost) > self.max_bytes)
        {
            let evicted = self.entries.remove(0);
            self.current_bytes = self.current_bytes.saturating_sub(evicted.byte_cost);
            self.metrics.evictions += 1;
        }
        if self.current_bytes.saturating_add(byte_cost) > self.max_bytes {
            self.metrics.rejections += 1;
            return Err(transform);
        }
        self.current_bytes = self.current_bytes.saturating_add(byte_cost);
        self.entries.push(IccTransformCacheEntry {
            key,
            transform,
            byte_cost,
        });
        self.metrics.admissions += 1;
        Ok(self.entries.len() - 1)
    }

    fn touch(&mut self, idx: usize) -> usize {
        if idx + 1 == self.entries.len() {
            idx
        } else {
            let entry = self.entries.remove(idx);
            self.entries.push(entry);
            self.entries.len() - 1
        }
    }

    pub(crate) fn transform_profile_to_srgb(
        &mut self,
        profile_bytes: &[u8],
        components: u8,
        pixels: &[u8],
        options: ColorTransformOptions,
    ) -> Option<(Vec<u8>, u8)> {
        let backend = IccBackend::from_policy(options.backend)?;
        self.transform_profile_to_srgb_with_backend(
            backend,
            profile_bytes,
            components,
            pixels,
            options,
        )
    }

    fn transform_profile_to_srgb_with_backend(
        &mut self,
        backend: IccBackend,
        profile_bytes: &[u8],
        components: u8,
        pixels: &[u8],
        options: ColorTransformOptions,
    ) -> Option<(Vec<u8>, u8)> {
        if profile_bytes.len() > DEFAULT_MAX_ICC_PROFILE_BYTES {
            self.metrics.unsupported_profiles += 1;
            return None;
        }
        let bytes_per_pixel = usize::from(components);
        if bytes_per_pixel == 0 || !pixels.len().is_multiple_of(bytes_per_pixel) {
            return None;
        }
        let key = IccTransformKey {
            kind: ICC_TRANSFORM_KIND_PROFILE_TO_SRGB,
            backend: backend.tag(),
            cache_scope: options.cache_scope,
            profile_digest: icc_profile_digest(profile_bytes),
            profile_len: profile_bytes.len(),
            components,
            src_type: qcms_data_type_for_components(components)
                .map(qcms_data_type_tag)
                .or_else(|| native_data_type_tag(components))?,
            dst_type: qcms_data_type_tag(qcms::DataType::RGB8),
            intent: options.intent,
            black_point_compensation: options.black_point_compensation,
        };
        let idx = match self.entries.iter().position(|entry| entry.key == key) {
            Some(idx) => {
                self.metrics.hits += 1;
                self.touch(idx)
            }
            None => {
                self.metrics.misses += 1;
                let transform = match backend {
                    IccBackend::FallbackQcms => {
                        self.create_qcms_transform(profile_bytes, components, options)?
                    }
                    #[cfg(feature = "native-cmm-lcms2")]
                    IccBackend::NativeLcms2 => {
                        self.create_lcms2_transform(profile_bytes, components, options)?
                    }
                };
                let byte_cost = icc_transform_entry_bytes(&key);
                match self.admit(key, transform, byte_cost) {
                    Ok(idx) => idx,
                    Err(transform) => {
                        let pixel_count = pixels.len() / bytes_per_pixel;
                        let mut rgb = vec![0u8; pixel_count * 3];
                        transform.convert(pixels, &mut rgb);
                        match backend {
                            IccBackend::FallbackQcms => self.metrics.fallback_qcms_transforms += 1,
                            #[cfg(feature = "native-cmm-lcms2")]
                            IccBackend::NativeLcms2 => self.metrics.native_lcms2_transforms += 1,
                        }
                        return Some((rgb, 3));
                    }
                }
            }
        };
        let pixel_count = pixels.len() / bytes_per_pixel;
        let mut rgb = vec![0u8; pixel_count * 3];
        self.entries[idx].transform.convert(pixels, &mut rgb);
        match backend {
            IccBackend::FallbackQcms => self.metrics.fallback_qcms_transforms += 1,
            #[cfg(feature = "native-cmm-lcms2")]
            IccBackend::NativeLcms2 => self.metrics.native_lcms2_transforms += 1,
        }
        Some((rgb, 3))
    }

    fn create_qcms_transform(
        &mut self,
        profile_bytes: &[u8],
        components: u8,
        options: ColorTransformOptions,
    ) -> Option<CachedIccTransform> {
        let src_type = qcms_data_type_for_components(components)?;
        if !qcms_profile_shape_matches_components(profile_bytes, components) {
            self.metrics.unsupported_profiles += 1;
            return None;
        }
        let input = match qcms::Profile::new_from_slice(profile_bytes, false) {
            Some(profile) => profile,
            None => {
                self.metrics.invalid_profiles += 1;
                return None;
            }
        };
        let mut output = qcms::Profile::new_sRGB();
        output.precache_output_transform();
        let transform = match qcms::Transform::new_to(
            &input,
            &output,
            src_type,
            qcms::DataType::RGB8,
            options.intent.to_qcms(),
        ) {
            Some(transform) => transform,
            None => {
                self.metrics.unsupported_profiles += 1;
                return None;
            }
        };
        Some(CachedIccTransform::Qcms(transform))
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn create_lcms2_transform(
        &mut self,
        profile_bytes: &[u8],
        components: u8,
        options: ColorTransformOptions,
    ) -> Option<CachedIccTransform> {
        let input_format = lcms2_pixel_format_for_components(components)?;
        let input = match lcms2::Profile::new_icc(profile_bytes) {
            Ok(profile) => profile,
            Err(_) => {
                self.metrics.invalid_profiles += 1;
                self.metrics.native_lcms2_failures += 1;
                return None;
            }
        };
        if !lcms2_profile_components_match(&input, components) {
            self.metrics.unsupported_profiles += 1;
            self.metrics.native_lcms2_failures += 1;
            return None;
        }
        let output = lcms2::Profile::new_srgb();
        let flags = if options.black_point_compensation {
            lcms2::Flags::BLACKPOINT_COMPENSATION
        } else {
            lcms2::Flags::default()
        };
        let transform = match lcms2::Transform::new_flags(
            &input,
            input_format,
            &output,
            lcms2::PixelFormat::RGB_8,
            options.intent.to_lcms2(),
            flags,
        ) {
            Ok(transform) => transform,
            Err(_) if input.device_class() == lcms2::ProfileClassSignature::LinkClass => {
                match lcms2::Transform::new_multiprofile(
                    &[&input, &output],
                    input_format,
                    lcms2::PixelFormat::RGB_8,
                    options.intent.to_lcms2(),
                    flags,
                ) {
                    Ok(transform) => transform,
                    Err(_) => {
                        self.metrics.native_lcms2_failures += 1;
                        return None;
                    }
                }
            }
            Err(_) => {
                self.metrics.native_lcms2_failures += 1;
                return None;
            }
        };
        Some(CachedIccTransform::Lcms2(transform))
    }

    pub(crate) fn transform_builtin_srgb_to_srgb_for_proof(
        &mut self,
        pixels: &[u8],
        options: ColorTransformOptions,
    ) -> Option<Vec<u8>> {
        if !pixels.len().is_multiple_of(3) {
            return None;
        }
        let backend = IccBackend::from_policy(options.backend)?;
        let key = IccTransformKey {
            kind: ICC_TRANSFORM_KIND_BUILTIN_SRGB_PROOF,
            backend: backend.tag(),
            cache_scope: options.cache_scope,
            profile_digest: icc_profile_digest(b"wellfriendpdf:builtin-srgb-proof"),
            profile_len: 0,
            components: 3,
            src_type: qcms_data_type_tag(qcms::DataType::RGB8),
            dst_type: qcms_data_type_tag(qcms::DataType::RGB8),
            intent: options.intent,
            black_point_compensation: options.black_point_compensation,
        };
        let idx = match self.entries.iter().position(|entry| entry.key == key) {
            Some(idx) => {
                self.metrics.hits += 1;
                self.touch(idx)
            }
            None => {
                self.metrics.misses += 1;
                let transform = match backend {
                    IccBackend::FallbackQcms => self.create_builtin_srgb_qcms_transform(options)?,
                    #[cfg(feature = "native-cmm-lcms2")]
                    IccBackend::NativeLcms2 => self.create_builtin_srgb_lcms2_transform(options)?,
                };
                let byte_cost = icc_transform_entry_bytes(&key);
                match self.admit(key, transform, byte_cost) {
                    Ok(idx) => idx,
                    Err(transform) => {
                        let mut out = vec![0u8; pixels.len()];
                        transform.convert(pixels, &mut out);
                        match backend {
                            IccBackend::FallbackQcms => self.metrics.fallback_qcms_transforms += 1,
                            #[cfg(feature = "native-cmm-lcms2")]
                            IccBackend::NativeLcms2 => self.metrics.native_lcms2_transforms += 1,
                        }
                        return Some(out);
                    }
                }
            }
        };
        let mut out = vec![0u8; pixels.len()];
        self.entries[idx].transform.convert(pixels, &mut out);
        match backend {
            IccBackend::FallbackQcms => self.metrics.fallback_qcms_transforms += 1,
            #[cfg(feature = "native-cmm-lcms2")]
            IccBackend::NativeLcms2 => self.metrics.native_lcms2_transforms += 1,
        }
        Some(out)
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn proof_srgb_via_output_intent(
        &mut self,
        output_intent_profile: &[u8],
        pixels: &[u8],
        options: ColorTransformOptions,
    ) -> Option<Vec<u8>> {
        if output_intent_profile.len() > DEFAULT_MAX_ICC_PROFILE_BYTES
            || !pixels.len().is_multiple_of(3)
        {
            self.metrics.unsupported_profiles += 1;
            return None;
        }
        let backend = IccBackend::from_policy(options.backend)?;
        if !matches!(backend, IccBackend::NativeLcms2) {
            return None;
        }
        let key = IccTransformKey {
            kind: ICC_TRANSFORM_KIND_OUTPUT_INTENT_PROOF,
            backend: backend.tag(),
            cache_scope: options.cache_scope,
            profile_digest: icc_profile_digest(output_intent_profile),
            profile_len: output_intent_profile.len(),
            components: 3,
            src_type: qcms_data_type_tag(qcms::DataType::RGB8),
            dst_type: qcms_data_type_tag(qcms::DataType::RGB8),
            intent: options.intent,
            black_point_compensation: options.black_point_compensation,
        };
        let idx = match self.entries.iter().position(|entry| entry.key == key) {
            Some(idx) => {
                self.metrics.hits += 1;
                self.touch(idx)
            }
            None => {
                self.metrics.misses += 1;
                let transform =
                    self.create_lcms2_proof_transform(output_intent_profile, options)?;
                let byte_cost = icc_transform_entry_bytes(&key);
                match self.admit(key, transform, byte_cost) {
                    Ok(idx) => idx,
                    Err(transform) => {
                        let mut out = vec![0u8; pixels.len()];
                        transform.convert(pixels, &mut out);
                        self.metrics.native_lcms2_transforms += 1;
                        return Some(out);
                    }
                }
            }
        };
        let mut out = vec![0u8; pixels.len()];
        self.entries[idx].transform.convert(pixels, &mut out);
        self.metrics.native_lcms2_transforms += 1;
        Some(out)
    }

    fn create_builtin_srgb_qcms_transform(
        &mut self,
        options: ColorTransformOptions,
    ) -> Option<CachedIccTransform> {
        let input = qcms::Profile::new_sRGB();
        let mut output = qcms::Profile::new_sRGB();
        output.precache_output_transform();
        let transform = qcms::Transform::new_to(
            &input,
            &output,
            qcms::DataType::RGB8,
            qcms::DataType::RGB8,
            options.intent.to_qcms(),
        )?;
        Some(CachedIccTransform::Qcms(transform))
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn create_builtin_srgb_lcms2_transform(
        &mut self,
        options: ColorTransformOptions,
    ) -> Option<CachedIccTransform> {
        let input = lcms2::Profile::new_srgb();
        let output = lcms2::Profile::new_srgb();
        let flags = if options.black_point_compensation {
            lcms2::Flags::BLACKPOINT_COMPENSATION
        } else {
            lcms2::Flags::default()
        };
        let transform = match lcms2::Transform::new_flags(
            &input,
            lcms2::PixelFormat::RGB_8,
            &output,
            lcms2::PixelFormat::RGB_8,
            options.intent.to_lcms2(),
            flags,
        ) {
            Ok(transform) => transform,
            Err(_) => {
                self.metrics.native_lcms2_failures += 1;
                return None;
            }
        };
        Some(CachedIccTransform::Lcms2(transform))
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn create_lcms2_proof_transform(
        &mut self,
        output_intent_profile: &[u8],
        options: ColorTransformOptions,
    ) -> Option<CachedIccTransform> {
        let proofing_profile = match lcms2::Profile::new_icc(output_intent_profile) {
            Ok(profile) => profile,
            Err(_) => {
                self.metrics.invalid_profiles += 1;
                self.metrics.native_lcms2_failures += 1;
                return None;
            }
        };
        let input = lcms2::Profile::new_srgb();
        let output = lcms2::Profile::new_srgb();
        let mut flags = lcms2::Flags::SOFT_PROOFING;
        if options.black_point_compensation {
            flags = flags | lcms2::Flags::BLACKPOINT_COMPENSATION;
        }
        let transform = match lcms2::Transform::new_proofing(
            &input,
            lcms2::PixelFormat::RGB_8,
            &output,
            lcms2::PixelFormat::RGB_8,
            &proofing_profile,
            options.intent.to_lcms2(),
            options.intent.to_lcms2(),
            flags,
        ) {
            Ok(transform) => transform,
            Err(_) => {
                self.metrics.native_lcms2_failures += 1;
                return None;
            }
        };
        Some(CachedIccTransform::Lcms2(transform))
    }
}

pub(crate) fn icc_transform_cache_metrics() -> IccTransformCacheMetrics {
    ICC_TRANSFORM_CACHE.with(|cache| cache.borrow().metrics())
}

pub(crate) fn color_transform_cache_scope(render_contract_fingerprint: &str) -> u64 {
    stable_hash(render_contract_fingerprint.as_bytes())
}

pub(crate) fn native_lcms2_profile_valid_for_components(
    profile_bytes: &[u8],
    components: Option<u8>,
) -> Option<bool> {
    #[cfg(feature = "native-cmm-lcms2")]
    {
        if profile_bytes.len() > DEFAULT_MAX_ICC_PROFILE_BYTES {
            return Some(false);
        }
        let profile = match lcms2::Profile::new_icc(profile_bytes) {
            Ok(profile) => profile,
            Err(_) => return Some(false),
        };
        Some(
            components
                .map(|n| lcms2_profile_components_match(&profile, n))
                .unwrap_or(true),
        )
    }
    #[cfg(not(feature = "native-cmm-lcms2"))]
    {
        let _ = (profile_bytes, components);
        None
    }
}

pub(crate) fn proof_srgb_via_output_intent(
    output_intent_profile: &[u8],
    pixels: &[u8],
    options: ColorTransformOptions,
) -> Option<Vec<u8>> {
    #[cfg(feature = "native-cmm-lcms2")]
    {
        ICC_TRANSFORM_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .proof_srgb_via_output_intent(output_intent_profile, pixels, options)
        })
    }
    #[cfg(not(feature = "native-cmm-lcms2"))]
    {
        let _ = (output_intent_profile, pixels, options);
        None
    }
}

pub(crate) fn srgb_identity_fidelity_probes() -> Vec<IccFidelityProbe> {
    let vectors: [(&str, &[u8]); 3] = [
        (
            "srgb_identity_primary_steps",
            &[0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255],
        ),
        (
            "srgb_identity_midpoints",
            &[16, 32, 64, 96, 128, 160, 192, 224, 240],
        ),
        (
            "srgb_identity_mixed",
            &[12, 200, 40, 50, 90, 180, 240, 120, 18],
        ),
    ];
    let mut cache = IccTransformCache::new(4);
    vectors
        .into_iter()
        .map(|(name, input)| {
            let output = cache
                .transform_builtin_srgb_to_srgb_for_proof(input, ColorTransformOptions::default())
                .unwrap_or_default();
            let max_abs_error = input
                .iter()
                .zip(output.iter())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            let passed = input.len() == output.len() && max_abs_error <= 1;
            IccFidelityProbe {
                name,
                backend: "qcms-builtin-srgb",
                input: input.to_vec(),
                output,
                max_abs_error,
                tolerance: 1,
                passed,
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn reset_icc_transform_cache_for_tests() {
    ICC_TRANSFORM_CACHE.with(|cache| {
        *cache.borrow_mut() = IccTransformCache::new(DEFAULT_TRANSFORM_CACHE_ENTRIES);
    });
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LabParams {
    pub white_point: [f32; 3],
    pub range: [f32; 4],
}

impl Default for LabParams {
    fn default() -> Self {
        Self {
            white_point: D50,
            range: [-100.0, 100.0, -100.0, 100.0],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CalGrayParams {
    pub white_point: [f32; 3],
    pub gamma: f32,
}

impl Default for CalGrayParams {
    fn default() -> Self {
        Self {
            white_point: D50,
            gamma: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CalRgbParams {
    pub white_point: [f32; 3],
    pub gamma: [f32; 3],
    /// PDF matrix order: [XA YA ZA XB YB ZB XC YC ZC].
    pub matrix: [f32; 9],
}

impl Default for CalRgbParams {
    fn default() -> Self {
        Self {
            white_point: D50,
            gamma: [1.0, 1.0, 1.0],
            matrix: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Convert unprofiled DeviceCMYK through a Poppler/Splash-like deterministic
/// process-color fallback.
///
/// DeviceCMYK is device dependent. Without an OutputIntent or ICCBased stream
/// there is no single correct transform, but Poppler's Windows build renders
/// default process inks close to these sRGB anchors:
/// C=(0,173,239), M=(236,0,140), Y=(255,242,0), R=(237,28,36),
/// G=(0,166,80), B=(46,49,146), CMY=(54,54,57), K=(35,31,32).
/// This fallback uses trilinear interpolation over that CMY process-ink cube,
/// then blends toward the measured process black. It is much closer to Poppler
/// than the old `(1-c)*(1-k)` algebraic conversion while remaining deterministic
/// and profile-license-free.
pub(crate) fn device_cmyk_to_srgb(c: f32, m: f32, y: f32, k: f32) -> [f32; 3] {
    let c = c.clamp(0.0, 1.0);
    let m = m.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);
    let k = k.clamp(0.0, 1.0);

    const W: [f32; 3] = [255.0, 255.0, 255.0];
    const C: [f32; 3] = [0.0, 173.0, 239.0];
    const M: [f32; 3] = [236.0, 0.0, 140.0];
    const Y: [f32; 3] = [255.0, 242.0, 0.0];
    const CM: [f32; 3] = [46.0, 49.0, 146.0];
    const CY: [f32; 3] = [0.0, 166.0, 80.0];
    const MY: [f32; 3] = [237.0, 28.0, 36.0];
    const CMY: [f32; 3] = [54.0, 54.0, 57.0];
    const BLACK: [f32; 3] = [35.0, 31.0, 32.0];

    let mut out = [0.0; 3];
    for i in 0..3 {
        let c00 = lerp(W[i], C[i], c);
        let c10 = lerp(M[i], CM[i], c);
        let c01 = lerp(Y[i], CY[i], c);
        let c11 = lerp(MY[i], CMY[i], c);
        let c0 = lerp(c00, c10, m);
        let c1 = lerp(c01, c11, m);
        out[i] = lerp(lerp(c0, c1, y), BLACK[i], k).clamp(0.0, 255.0) / 255.0;
    }
    out
}

pub(crate) fn device_cmyk_bytes_to_rgb(pixels: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
    for chunk in pixels.chunks_exact(4) {
        let out = device_cmyk_to_srgb(
            chunk[0] as f32 / 255.0,
            chunk[1] as f32 / 255.0,
            chunk[2] as f32 / 255.0,
            chunk[3] as f32 / 255.0,
        );
        rgb.extend(out.map(unit_to_u8));
    }
    rgb
}

pub(crate) fn device_cmyk_overprint_preview_srgb(
    dst_rgb: [f32; 3],
    src_cmyk: [f32; 4],
    overprint_mode_one: bool,
) -> [f32; 3] {
    if !overprint_mode_one {
        return device_cmyk_to_srgb(src_cmyk[0], src_cmyk[1], src_cmyk[2], src_cmyk[3]);
    }
    let mut dst_cmyk = srgb_to_device_cmyk_preview(dst_rgb);
    for (dst, src) in dst_cmyk.iter_mut().zip(src_cmyk) {
        let src = src.clamp(0.0, 1.0);
        if src > 1e-6 {
            *dst = src;
        }
    }
    device_cmyk_to_srgb(dst_cmyk[0], dst_cmyk[1], dst_cmyk[2], dst_cmyk[3])
}

pub(crate) fn srgb_to_device_cmyk_preview(rgb: [f32; 3]) -> [f32; 4] {
    let r = rgb[0].clamp(0.0, 1.0);
    let g = rgb[1].clamp(0.0, 1.0);
    let b = rgb[2].clamp(0.0, 1.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 - 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let denom = 1.0 - k;
    [
        ((1.0 - r - k) / denom).clamp(0.0, 1.0),
        ((1.0 - g - k) / denom).clamp(0.0, 1.0),
        ((1.0 - b - k) / denom).clamp(0.0, 1.0),
        k.clamp(0.0, 1.0),
    ]
}

pub(crate) fn lab_to_srgb(l: f32, a: f32, b: f32, params: LabParams) -> [f32; 3] {
    let l = l.clamp(0.0, 100.0);
    let a = a.clamp(params.range[0], params.range[1]);
    let b = b.clamp(params.range[2], params.range[3]);
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let xyz = [
        params.white_point[0] * lab_f_inv(fx),
        params.white_point[1] * lab_f_inv(fy),
        params.white_point[2] * lab_f_inv(fz),
    ];
    xyz_d50_to_srgb(adapt_xyz_to_d50(xyz, params.white_point))
}

pub(crate) fn lab_bytes_to_rgb(pixels: &[u8], params: LabParams) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks_exact(3) {
        let l = decode_range(chunk[0], 0.0, 100.0);
        let a = decode_range(chunk[1], params.range[0], params.range[1]);
        let b = decode_range(chunk[2], params.range[2], params.range[3]);
        rgb.extend(lab_to_srgb(l, a, b, params).map(unit_to_u8));
    }
    rgb
}

pub(crate) fn cal_gray_to_srgb(gray: f32, params: CalGrayParams) -> [f32; 3] {
    let g = gray.clamp(0.0, 1.0).powf(params.gamma.max(0.01));
    xyz_d50_to_srgb(adapt_xyz_to_d50(
        [
            params.white_point[0] * g,
            params.white_point[1] * g,
            params.white_point[2] * g,
        ],
        params.white_point,
    ))
}

pub(crate) fn cal_gray_bytes_to_rgb(pixels: &[u8], params: CalGrayParams) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for &gray in pixels {
        rgb.extend(cal_gray_to_srgb(gray as f32 / 255.0, params).map(unit_to_u8));
    }
    rgb
}

pub(crate) fn cal_rgb_to_srgb(components: [f32; 3], params: CalRgbParams) -> [f32; 3] {
    let a = components[0]
        .clamp(0.0, 1.0)
        .powf(params.gamma[0].max(0.01));
    let b = components[1]
        .clamp(0.0, 1.0)
        .powf(params.gamma[1].max(0.01));
    let c = components[2]
        .clamp(0.0, 1.0)
        .powf(params.gamma[2].max(0.01));
    let m = params.matrix;
    let xyz = [
        m[0] * a + m[3] * b + m[6] * c,
        m[1] * a + m[4] * b + m[7] * c,
        m[2] * a + m[5] * b + m[8] * c,
    ];
    xyz_d50_to_srgb(adapt_xyz_to_d50(xyz, params.white_point))
}

pub(crate) fn cal_rgb_bytes_to_rgb(pixels: &[u8], params: CalRgbParams) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks_exact(3) {
        let comps = [
            chunk[0] as f32 / 255.0,
            chunk[1] as f32 / 255.0,
            chunk[2] as f32 / 255.0,
        ];
        rgb.extend(cal_rgb_to_srgb(comps, params).map(unit_to_u8));
    }
    rgb
}

pub(crate) fn icc_bytes_to_rgb_with_options(
    pixels: &[u8],
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: ColorTransformOptions,
) -> Option<(Vec<u8>, u8)> {
    let (profile_dict, profile_bytes) = icc_profile_stream(dict, reader)?;
    let n = icc_profile_component_count(&profile_dict)?;
    ICC_TRANSFORM_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .transform_profile_to_srgb(&profile_bytes, n, pixels, options)
    })
}

pub(crate) fn icc_components_to_srgb_with_options(
    space_obj: &PdfObject,
    components: &[f64],
    reader: &PdfReader,
    options: ColorTransformOptions,
) -> Option<[f32; 3]> {
    let (profile_dict, profile_bytes) = icc_profile_stream_from_space(space_obj, reader)?;
    let n = icc_profile_component_count(&profile_dict)?;
    let component_count = usize::from(n);
    if components.len() != component_count
        || components.iter().any(|component| !component.is_finite())
    {
        return None;
    }
    let mut src = vec![0u8; component_count];
    for (i, byte) in src.iter_mut().enumerate() {
        *byte = unit_to_u8(components[i] as f32);
    }
    let (dst, _) = ICC_TRANSFORM_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .transform_profile_to_srgb(&profile_bytes, n, &src, options)
    })?;
    Some([
        dst[0] as f32 / 255.0,
        dst[1] as f32 / 255.0,
        dst[2] as f32 / 255.0,
    ])
}

fn qcms_data_type_for_components(components: u8) -> Option<qcms::DataType> {
    match components {
        1 => Some(qcms::DataType::Gray8),
        3 => Some(qcms::DataType::RGB8),
        4 => Some(qcms::DataType::CMYK),
        _ => None,
    }
}

fn qcms_profile_shape_matches_components(profile_bytes: &[u8], components: u8) -> bool {
    let Some(color_space) = icc_header_signature(profile_bytes, 16) else {
        return false;
    };
    match components {
        1 => color_space == *b"GRAY",
        3 => color_space == *b"RGB ",
        4 => false,
        _ => false,
    }
}

fn icc_header_signature(profile_bytes: &[u8], offset: usize) -> Option<[u8; 4]> {
    let end = offset.checked_add(4)?;
    let bytes = profile_bytes.get(offset..end)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn qcms_data_type_tag(data_type: qcms::DataType) -> u8 {
    match data_type {
        qcms::DataType::RGB8 => 0,
        qcms::DataType::RGBA8 => 1,
        qcms::DataType::BGRA8 => 2,
        qcms::DataType::Gray8 => 3,
        qcms::DataType::GrayA8 => 4,
        qcms::DataType::CMYK => 5,
    }
}

fn native_data_type_tag(components: u8) -> Option<u8> {
    match components {
        1 => Some(10),
        3 => Some(11),
        4 => Some(12),
        _ => None,
    }
}

#[cfg(feature = "native-cmm-lcms2")]
fn lcms2_pixel_format_for_components(components: u8) -> Option<lcms2::PixelFormat> {
    match components {
        1 => Some(lcms2::PixelFormat::GRAY_8),
        3 => Some(lcms2::PixelFormat::RGB_8),
        4 => Some(lcms2::PixelFormat::CMYK_8),
        _ => None,
    }
}

#[cfg(feature = "native-cmm-lcms2")]
fn lcms2_profile_components_match(profile: &lcms2::Profile, components: u8) -> bool {
    matches!(
        (profile.color_space(), components),
        (lcms2::ColorSpaceSignature::GrayData, 1)
            | (lcms2::ColorSpaceSignature::RgbData, 3)
            | (lcms2::ColorSpaceSignature::CmykData, 4)
    )
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn icc_channel_count(dict: &PdfDictionary, reader: &PdfReader) -> Option<u8> {
    let (profile_dict, _) = icc_profile_object(dict, reader)?;
    icc_profile_component_count(&profile_dict)
}

fn icc_profile_component_count(profile_dict: &PdfDictionary) -> Option<u8> {
    let n = profile_dict.get_integer("N")?;
    if (1..=4).contains(&n) {
        Some(n as u8)
    } else {
        None
    }
}

pub(crate) fn try_lab_params_from_image_dict(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<LabParams, String> {
    let color_space = image_color_space_object(dict, "Lab")?;
    let param_dict = strict_calibrated_param_dict(color_space, reader, "Lab")?;
    let white_point = required_xyz(&param_dict, "WhitePoint", "Lab")?;
    optional_xyz(&param_dict, "BlackPoint", "Lab")?;
    let range = optional_lab_range(&param_dict)?;
    Ok(LabParams { white_point, range })
}

pub(crate) fn try_cal_gray_params_from_image_dict(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<CalGrayParams, String> {
    let color_space = image_color_space_object(dict, "CalGray")?;
    let param_dict = strict_calibrated_param_dict(color_space, reader, "CalGray")?;
    let white_point = required_xyz(&param_dict, "WhitePoint", "CalGray")?;
    optional_xyz(&param_dict, "BlackPoint", "CalGray")?;
    let gamma = optional_positive_number(&param_dict, "Gamma", "CalGray")?.unwrap_or(1.0);
    Ok(CalGrayParams { white_point, gamma })
}

pub(crate) fn try_cal_rgb_params_from_image_dict(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
) -> std::result::Result<CalRgbParams, String> {
    let color_space = image_color_space_object(dict, "CalRGB")?;
    let param_dict = strict_calibrated_param_dict(color_space, reader, "CalRGB")?;
    let white_point = required_xyz(&param_dict, "WhitePoint", "CalRGB")?;
    optional_xyz(&param_dict, "BlackPoint", "CalRGB")?;
    let gamma = optional_positive_number_array(&param_dict, "Gamma", "CalRGB")?.unwrap_or([1.0; 3]);
    let matrix = optional_number_array9(&param_dict, "Matrix", "CalRGB")?
        .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    Ok(CalRgbParams {
        white_point,
        gamma,
        matrix,
    })
}

fn image_color_space_object<'a>(
    dict: &'a PdfDictionary,
    family: &str,
) -> std::result::Result<&'a PdfObject, String> {
    dict.get("ColorSpace")
        .or_else(|| dict.get("CS"))
        .ok_or_else(|| format!("{family} image ColorSpace is missing parameter array"))
}

fn strict_calibrated_param_dict(
    space: &PdfObject,
    reader: Option<&PdfReader>,
    family: &str,
) -> std::result::Result<PdfDictionary, String> {
    let arr = resolve_space_array_strict(space, reader, family)?;
    match arr.first().and_then(PdfObject::as_name) {
        Some(name) if name == family => {}
        Some(name) => {
            return Err(format!(
                "{family} image ColorSpace resolved to /{name}, expected /{family}"
            ))
        }
        None => {
            return Err(format!(
                "{family} image ColorSpace array has no family name"
            ))
        }
    }
    if arr.len() != 2 {
        return Err(format!(
            "{family} image ColorSpace has {} entries, expected 2",
            arr.len()
        ));
    }
    let params = arr
        .get(1)
        .ok_or_else(|| format!("{family} image ColorSpace missing parameter dictionary"))?;
    resolve_to_dict_strict(params, reader, family)
}

fn resolve_space_array_strict(
    space: &PdfObject,
    reader: Option<&PdfReader>,
    family: &str,
) -> std::result::Result<Vec<PdfObject>, String> {
    let resolved = match (space, reader) {
        (PdfObject::Reference { .. }, Some(reader)) => reader
            .resolve(space.clone())
            .map_err(|err| format!("{family} image ColorSpace reference failed: {err}"))?,
        (PdfObject::Reference { .. }, None) => {
            return Err(format!(
                "{family} image ColorSpace reference requires a PdfReader"
            ))
        }
        _ => space.clone(),
    };
    resolved.as_array().map(|arr| arr.to_vec()).ok_or_else(|| {
        format!("{family} image ColorSpace must be an array with a parameter dictionary")
    })
}

fn resolve_to_dict_strict(
    obj: &PdfObject,
    reader: Option<&PdfReader>,
    family: &str,
) -> std::result::Result<PdfDictionary, String> {
    let resolved = match (obj, reader) {
        (PdfObject::Reference { .. }, Some(reader)) => {
            reader.resolve(obj.clone()).map_err(|err| {
                format!("{family} image ColorSpace parameter dictionary reference failed: {err}")
            })?
        }
        (PdfObject::Reference { .. }, None) => {
            return Err(format!(
                "{family} image ColorSpace parameter dictionary reference requires a PdfReader"
            ))
        }
        _ => obj.clone(),
    };
    resolved.as_dict().cloned().ok_or_else(|| {
        format!("{family} image ColorSpace parameter object must resolve to a dictionary")
    })
}

fn required_xyz(
    dict: &PdfDictionary,
    key: &str,
    family: &str,
) -> std::result::Result<[f32; 3], String> {
    let obj = dict
        .get(key)
        .ok_or_else(|| format!("{family} image ColorSpace missing /{key}"))?;
    let values = numeric_array_exact(obj, 3)
        .ok_or_else(|| format!("{family} image ColorSpace /{key} must have 3 numeric values"))?;
    if !valid_xyz(&values) {
        return Err(format!(
            "{family} image ColorSpace /{key} contains invalid XYZ values"
        ));
    }
    Ok([values[0] as f32, values[1] as f32, values[2] as f32])
}

fn optional_xyz(
    dict: &PdfDictionary,
    key: &str,
    family: &str,
) -> std::result::Result<Option<[f32; 3]>, String> {
    let Some(obj) = dict.get(key) else {
        return Ok(None);
    };
    let values = numeric_array_exact(obj, 3)
        .ok_or_else(|| format!("{family} image ColorSpace /{key} must have 3 numeric values"))?;
    if !valid_xyz(&values) {
        return Err(format!(
            "{family} image ColorSpace /{key} contains invalid XYZ values"
        ));
    }
    Ok(Some([values[0] as f32, values[1] as f32, values[2] as f32]))
}

fn optional_positive_number(
    dict: &PdfDictionary,
    key: &str,
    family: &str,
) -> std::result::Result<Option<f32>, String> {
    let Some(obj) = dict.get(key) else {
        return Ok(None);
    };
    let value = obj
        .as_number()
        .ok_or_else(|| format!("{family} image ColorSpace /{key} must be numeric"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!(
            "{family} image ColorSpace /{key} must be finite and positive"
        ));
    }
    Ok(Some(value as f32))
}

fn optional_positive_number_array(
    dict: &PdfDictionary,
    key: &str,
    family: &str,
) -> std::result::Result<Option<[f32; 3]>, String> {
    let Some(obj) = dict.get(key) else {
        return Ok(None);
    };
    let values = numeric_array_exact(obj, 3)
        .ok_or_else(|| format!("{family} image ColorSpace /{key} must have 3 numeric values"))?;
    if values
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(format!(
            "{family} image ColorSpace /{key} values must be finite and positive"
        ));
    }
    Ok(Some([values[0] as f32, values[1] as f32, values[2] as f32]))
}

fn optional_number_array9(
    dict: &PdfDictionary,
    key: &str,
    family: &str,
) -> std::result::Result<Option<[f32; 9]>, String> {
    let Some(obj) = dict.get(key) else {
        return Ok(None);
    };
    let values = numeric_array_exact(obj, 9)
        .ok_or_else(|| format!("{family} image ColorSpace /{key} must have 9 numeric values"))?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "{family} image ColorSpace /{key} values must be finite"
        ));
    }
    let mut out = [0.0; 9];
    for (idx, dst) in out.iter_mut().enumerate() {
        *dst = values[idx] as f32;
    }
    Ok(Some(out))
}

fn optional_lab_range(dict: &PdfDictionary) -> std::result::Result<[f32; 4], String> {
    let Some(obj) = dict.get("Range") else {
        return Ok([-100.0, 100.0, -100.0, 100.0]);
    };
    let values = numeric_array_exact(obj, 4)
        .ok_or_else(|| "Lab image ColorSpace /Range must have 4 numeric values".to_string())?;
    if values.iter().any(|value| !value.is_finite())
        || values[0] > values[1]
        || values[2] > values[3]
    {
        return Err("Lab image ColorSpace /Range contains invalid bounds".to_string());
    }
    Ok([
        values[0] as f32,
        values[1] as f32,
        values[2] as f32,
        values[3] as f32,
    ])
}

fn numeric_array_exact(obj: &PdfObject, expected_len: usize) -> Option<Vec<f64>> {
    let arr = obj.as_array()?;
    if arr.len() != expected_len {
        return None;
    }
    arr.iter().map(PdfObject::as_number).collect()
}

fn valid_xyz(values: &[f64]) -> bool {
    values.len() == 3
        && values.iter().all(|value| value.is_finite())
        && values[0] >= 0.0
        && values[1] > 0.0
        && values[2] >= 0.0
}

fn icc_profile_stream(
    dict: &PdfDictionary,
    reader: &PdfReader,
) -> Option<(PdfDictionary, Vec<u8>)> {
    let (profile_dict, stream_obj) = icc_profile_object(dict, reader)?;
    let decoded = decode_stream_lossless(&stream_obj, reader).ok()?;
    match decoded.status {
        StreamDecodeStatus::Complete if decoded.data.len() <= DEFAULT_MAX_ICC_PROFILE_BYTES => {
            Some((profile_dict, decoded.data))
        }
        StreamDecodeStatus::Complete => None,
        StreamDecodeStatus::StoppedAtImageFilter(_) => None,
    }
}

fn icc_profile_stream_from_space(
    space_obj: &PdfObject,
    reader: &PdfReader,
) -> Option<(PdfDictionary, Vec<u8>)> {
    let (profile_dict, stream_obj) = icc_profile_object_from_space(space_obj, reader)?;
    let decoded = decode_stream_lossless(&stream_obj, reader).ok()?;
    match decoded.status {
        StreamDecodeStatus::Complete if decoded.data.len() <= DEFAULT_MAX_ICC_PROFILE_BYTES => {
            Some((profile_dict, decoded.data))
        }
        StreamDecodeStatus::Complete => None,
        StreamDecodeStatus::StoppedAtImageFilter(_) => None,
    }
}

fn icc_profile_object(
    dict: &PdfDictionary,
    reader: &PdfReader,
) -> Option<(PdfDictionary, PdfObject)> {
    let arr = dict.get("ColorSpace")?.as_array()?;
    if arr.first().and_then(PdfObject::as_name) != Some("ICCBased") {
        return None;
    }
    let obj = reader.resolve(arr.get(1)?.clone()).ok()?;
    match obj {
        PdfObject::Stream { dict, raw } => {
            let profile_dict = dict.clone();
            Some((profile_dict, PdfObject::Stream { dict, raw }))
        }
        _ => None,
    }
}

fn icc_profile_object_from_space(
    space_obj: &PdfObject,
    reader: &PdfReader,
) -> Option<(PdfDictionary, PdfObject)> {
    let resolved = match space_obj {
        PdfObject::Reference { .. } => reader.resolve(space_obj.clone()).ok()?,
        other => other.clone(),
    };
    let arr = resolved.as_array()?;
    if arr.first().and_then(PdfObject::as_name) != Some("ICCBased") {
        return None;
    }
    let obj = reader.resolve(arr.get(1)?.clone()).ok()?;
    match obj {
        PdfObject::Stream { dict, raw } => {
            let profile_dict = dict.clone();
            Some((profile_dict, PdfObject::Stream { dict, raw }))
        }
        _ => None,
    }
}

pub(crate) fn lab_params_from_space(
    space: &PdfObject,
    reader: Option<&PdfReader>,
) -> Option<LabParams> {
    let arr = resolve_space_array(space, reader)?;
    if arr.first().and_then(PdfObject::as_name) != Some("Lab") {
        return None;
    }
    let dict = arr.get(1).and_then(|obj| resolve_to_dict(obj, reader))?;
    Some(LabParams {
        white_point: read_xyz(&dict, "WhitePoint").unwrap_or(D50),
        range: read_range4(&dict, "Range").unwrap_or([-100.0, 100.0, -100.0, 100.0]),
    })
}

pub(crate) fn cal_gray_params_from_space(
    space: &PdfObject,
    reader: Option<&PdfReader>,
) -> Option<CalGrayParams> {
    let arr = resolve_space_array(space, reader)?;
    if arr.first().and_then(PdfObject::as_name) != Some("CalGray") {
        return None;
    }
    let dict = arr.get(1).and_then(|obj| resolve_to_dict(obj, reader))?;
    Some(CalGrayParams {
        white_point: read_xyz(&dict, "WhitePoint").unwrap_or(D50),
        gamma: dict
            .get("Gamma")
            .and_then(PdfObject::as_number)
            .unwrap_or(1.0) as f32,
    })
}

pub(crate) fn cal_rgb_params_from_space(
    space: &PdfObject,
    reader: Option<&PdfReader>,
) -> Option<CalRgbParams> {
    let arr = resolve_space_array(space, reader)?;
    if arr.first().and_then(PdfObject::as_name) != Some("CalRGB") {
        return None;
    }
    let dict = arr.get(1).and_then(|obj| resolve_to_dict(obj, reader))?;
    Some(CalRgbParams {
        white_point: read_xyz(&dict, "WhitePoint").unwrap_or(D50),
        gamma: read_array3(&dict, "Gamma").unwrap_or([1.0, 1.0, 1.0]),
        matrix: read_array9(&dict, "Matrix")
            .unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
    })
}

fn resolve_space_array(space: &PdfObject, reader: Option<&PdfReader>) -> Option<Vec<PdfObject>> {
    let resolved = match (space, reader) {
        (PdfObject::Reference { .. }, Some(reader)) => reader.resolve(space.clone()).ok()?,
        _ => space.clone(),
    };
    resolved.as_array().map(|arr| arr.to_vec())
}

fn resolve_to_dict(obj: &PdfObject, reader: Option<&PdfReader>) -> Option<PdfDictionary> {
    let resolved = match (obj, reader) {
        (PdfObject::Reference { .. }, Some(reader)) => reader.resolve(obj.clone()).ok()?,
        _ => obj.clone(),
    };
    resolved.as_dict().cloned()
}

fn read_xyz(dict: &PdfDictionary, key: &str) -> Option<[f32; 3]> {
    let arr = dict.get_array(key)?;
    if arr.len() < 3 {
        return None;
    }
    Some([
        arr[0].as_number()? as f32,
        arr[1].as_number()? as f32,
        arr[2].as_number()? as f32,
    ])
}

fn read_range4(dict: &PdfDictionary, key: &str) -> Option<[f32; 4]> {
    let arr = dict.get_array(key)?;
    if arr.len() < 4 {
        return None;
    }
    Some([
        arr[0].as_number()? as f32,
        arr[1].as_number()? as f32,
        arr[2].as_number()? as f32,
        arr[3].as_number()? as f32,
    ])
}

fn read_array3(dict: &PdfDictionary, key: &str) -> Option<[f32; 3]> {
    let arr = dict.get_array(key)?;
    if arr.len() < 3 {
        return None;
    }
    Some([
        arr[0].as_number()? as f32,
        arr[1].as_number()? as f32,
        arr[2].as_number()? as f32,
    ])
}

fn read_array9(dict: &PdfDictionary, key: &str) -> Option<[f32; 9]> {
    let arr = dict.get_array(key)?;
    if arr.len() < 9 {
        return None;
    }
    let mut out = [0.0; 9];
    for (i, dst) in out.iter_mut().enumerate() {
        *dst = arr[i].as_number()? as f32;
    }
    Some(out)
}

fn adapt_xyz_to_d50(xyz: [f32; 3], source_white: [f32; 3]) -> [f32; 3] {
    if close3(source_white, D50) {
        return xyz;
    }
    const BRADFORD: [[f32; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    const BRADFORD_INV: [[f32; 3]; 3] = [
        [0.9869929, -0.1470543, 0.1599627],
        [0.4323053, 0.5183603, 0.0492912],
        [-0.0085287, 0.0400428, 0.9684867],
    ];
    let src_lms = mat3_mul_vec(BRADFORD, source_white);
    let dst_lms = mat3_mul_vec(BRADFORD, D50);
    let xyz_lms = mat3_mul_vec(BRADFORD, xyz);
    let adapted_lms = [
        xyz_lms[0] * safe_ratio(dst_lms[0], src_lms[0]),
        xyz_lms[1] * safe_ratio(dst_lms[1], src_lms[1]),
        xyz_lms[2] * safe_ratio(dst_lms[2], src_lms[2]),
    ];
    mat3_mul_vec(BRADFORD_INV, adapted_lms)
}

fn xyz_d50_to_srgb(xyz: [f32; 3]) -> [f32; 3] {
    // D50-adapted sRGB matrix.
    let r = 3.133_856 * xyz[0] - 1.616_866_7 * xyz[1] - 0.490_614_6 * xyz[2];
    let g = -0.978_768_4 * xyz[0] + 1.916_141_5 * xyz[1] + 0.033_454 * xyz[2];
    let b = 0.071_945_3 * xyz[0] - 0.228_991_4 * xyz[1] + 1.405_242_7 * xyz[2];
    [srgb_encode(r), srgb_encode(g), srgb_encode(b)]
}

fn lab_f_inv(t: f32) -> f32 {
    const DELTA: f32 = 6.0 / 29.0;
    if t > DELTA {
        t * t * t
    } else {
        3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
    }
}

fn srgb_encode(linear: f32) -> f32 {
    let linear = linear.clamp(0.0, 1.0);
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

fn decode_range(byte: u8, lo: f32, hi: f32) -> f32 {
    lo + (byte as f32 / 255.0) * (hi - lo)
}

fn unit_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

fn mat3_mul_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn safe_ratio(num: f32, den: f32) -> f32 {
    if den.abs() < 1e-6 {
        1.0
    } else {
        num / den
    }
}

fn close3(a: [f32; 3], b: [f32; 3]) -> bool {
    (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4 && (a[2] - b[2]).abs() < 1e-4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(rgb: [f32; 3]) -> [u8; 3] {
        rgb.map(unit_to_u8)
    }

    #[test]
    fn device_cmyk_matches_poppler_process_anchors() {
        assert_eq!(
            bytes(device_cmyk_to_srgb(0.0, 0.0, 0.0, 0.0)),
            [255, 255, 255]
        );
        assert_eq!(
            bytes(device_cmyk_to_srgb(1.0, 0.0, 0.0, 0.0)),
            [0, 173, 239]
        );
        assert_eq!(
            bytes(device_cmyk_to_srgb(0.0, 1.0, 0.0, 0.0)),
            [236, 0, 140]
        );
        assert_eq!(
            bytes(device_cmyk_to_srgb(0.0, 0.0, 1.0, 0.0)),
            [255, 242, 0]
        );
        assert_eq!(
            bytes(device_cmyk_to_srgb(0.0, 1.0, 1.0, 0.0)),
            [237, 28, 36]
        );
        assert_eq!(bytes(device_cmyk_to_srgb(1.0, 0.0, 1.0, 0.0)), [0, 166, 80]);
        assert_eq!(
            bytes(device_cmyk_to_srgb(1.0, 1.0, 0.0, 0.0)),
            [46, 49, 146]
        );
        assert_eq!(bytes(device_cmyk_to_srgb(1.0, 1.0, 1.0, 0.0)), [54, 54, 57]);
        assert_eq!(bytes(device_cmyk_to_srgb(0.0, 0.0, 0.0, 1.0)), [35, 31, 32]);
        assert_eq!(
            bytes(device_cmyk_to_srgb(0.0, 0.0, 0.0, 0.5)),
            [145, 143, 144]
        );
    }

    #[test]
    fn device_cmyk_mixed_color_is_near_poppler_probe() {
        let out = bytes(device_cmyk_to_srgb(0.5, 0.25, 0.0, 0.2));
        assert!(
            (out[0] as i16 - 108).abs() <= 8,
            "R should be near Poppler probe, got {out:?}"
        );
        assert!(
            (out[1] as i16 - 137).abs() <= 12,
            "G should be near Poppler probe, got {out:?}"
        );
        assert!(
            (out[2] as i16 - 182).abs() <= 8,
            "B should be near Poppler probe, got {out:?}"
        );
    }

    #[test]
    fn lab_white_and_black_are_correct() {
        assert_eq!(
            bytes(lab_to_srgb(100.0, 0.0, 0.0, LabParams::default())),
            [255, 255, 255]
        );
        assert_eq!(
            bytes(lab_to_srgb(0.0, 0.0, 0.0, LabParams::default())),
            [0, 0, 0]
        );
    }

    #[test]
    fn lab_mid_gray_is_neutral() {
        let out = bytes(lab_to_srgb(50.0, 0.0, 0.0, LabParams::default()));
        assert!((out[0] as i16 - out[1] as i16).abs() <= 2, "{out:?}");
        assert!((out[1] as i16 - out[2] as i16).abs() <= 2, "{out:?}");
        assert!((115..=125).contains(&out[0]), "{out:?}");
    }

    #[test]
    fn cal_gray_gamma_is_applied() {
        let params = CalGrayParams {
            gamma: 2.0,
            ..CalGrayParams::default()
        };
        let out = bytes(cal_gray_to_srgb(0.5, params));
        assert!(out[0] < 140, "{out:?}");
        assert!((out[0] as i16 - out[1] as i16).abs() <= 2, "{out:?}");
    }

    #[test]
    fn builtin_srgb_transform_cache_is_deterministic() {
        reset_icc_transform_cache_for_tests();
        let pixels = [0u8, 64, 128, 255, 128, 0];
        let mut cache = IccTransformCache::new(4);
        let first = cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, ColorTransformOptions::default())
            .unwrap();
        let second = cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, ColorTransformOptions::default())
            .unwrap();
        assert_eq!(first, pixels);
        assert_eq!(second, pixels);
        let metrics = cache.metrics();
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.entries, 1);
        let global_metrics = icc_transform_cache_metrics();
        assert_eq!(global_metrics.max_entries, DEFAULT_TRANSFORM_CACHE_ENTRIES);
        assert_eq!(global_metrics.max_bytes, DEFAULT_TRANSFORM_CACHE_BYTES);
    }

    #[test]
    fn transform_cache_eviction_is_bounded() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new(1);
        cache
            .transform_builtin_srgb_to_srgb_for_proof(
                &pixels,
                ColorTransformOptions {
                    intent: ColorIntent::Perceptual,
                    black_point_compensation: false,
                    ..ColorTransformOptions::default()
                },
            )
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(
                &pixels,
                ColorTransformOptions {
                    intent: ColorIntent::RelativeColorimetric,
                    black_point_compensation: false,
                    ..ColorTransformOptions::default()
                },
            )
            .unwrap();
        let metrics = cache.metrics();
        assert_eq!(metrics.entries, 1);
        assert_eq!(metrics.evictions, 1);
        assert_eq!(metrics.admissions, 2);
        assert_eq!(metrics.rejections, 0);
        assert!(metrics.bytes_used > 0);
        assert!(metrics.bytes_used <= metrics.max_bytes);
    }

    #[test]
    fn transform_cache_rejects_entries_over_byte_budget() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new_with_budget(4, 1);
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, ColorTransformOptions::default())
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, ColorTransformOptions::default())
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.entries, 0);
        assert_eq!(metrics.admissions, 0);
        assert_eq!(metrics.rejections, 2);
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 0);
        assert_eq!(metrics.bytes_used, 0);
        assert_eq!(metrics.max_bytes, 1);
    }

    #[test]
    fn transform_cache_hit_refreshes_lru_eviction_order() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new(2);
        let relative = ColorTransformOptions {
            intent: ColorIntent::RelativeColorimetric,
            ..ColorTransformOptions::default()
        };
        let perceptual = ColorTransformOptions {
            intent: ColorIntent::Perceptual,
            ..ColorTransformOptions::default()
        };
        let saturation = ColorTransformOptions {
            intent: ColorIntent::Saturation,
            ..ColorTransformOptions::default()
        };

        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, relative)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, perceptual)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, relative)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, saturation)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, relative)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, perceptual)
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 2);
        assert_eq!(metrics.misses, 4);
        assert_eq!(metrics.evictions, 2);
        assert_eq!(metrics.entries, 2);
    }

    #[test]
    fn transform_cache_key_uses_full_profile_digest_identity() {
        fn key(profile_bytes: &[u8]) -> IccTransformKey {
            IccTransformKey {
                kind: ICC_TRANSFORM_KIND_PROFILE_TO_SRGB,
                backend: 0,
                cache_scope: 0,
                profile_digest: icc_profile_digest(profile_bytes),
                profile_len: profile_bytes.len(),
                components: 3,
                src_type: qcms_data_type_tag(qcms::DataType::RGB8),
                dst_type: qcms_data_type_tag(qcms::DataType::RGB8),
                intent: ColorIntent::RelativeColorimetric,
                black_point_compensation: false,
            }
        }

        let first = key(&[0, 1, 2, 3, 4, 5, 6, 7]);
        let second = key(&[0, 1, 2, 3, 4, 5, 6, 8]);

        assert_eq!(first.profile_len, second.profile_len);
        assert_ne!(first.profile_digest, second.profile_digest);
        assert_ne!(first, second);
    }

    #[test]
    fn pdf_rendering_intent_names_map_to_cmm_intents() {
        assert_eq!(
            ColorIntent::from_pdf_name("RelativeColorimetric"),
            ColorIntent::RelativeColorimetric
        );
        assert_eq!(
            ColorIntent::from_pdf_name("/AbsoluteColorimetric"),
            ColorIntent::AbsoluteColorimetric
        );
        assert_eq!(
            ColorIntent::from_pdf_name("relative-colorimetric"),
            ColorIntent::RelativeColorimetric
        );
        assert_eq!(
            ColorIntent::from_pdf_name("Perceptual"),
            ColorIntent::Perceptual
        );
        assert_eq!(
            ColorIntent::from_pdf_name("UnknownIntent"),
            ColorIntent::RelativeColorimetric
        );
    }

    #[test]
    fn transform_cache_key_includes_rendering_intent() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new(4);
        let relative = ColorTransformOptions {
            intent: ColorIntent::RelativeColorimetric,
            black_point_compensation: false,
            ..ColorTransformOptions::default()
        };
        let perceptual = ColorTransformOptions {
            intent: ColorIntent::Perceptual,
            black_point_compensation: false,
            ..ColorTransformOptions::default()
        };
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, relative)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, perceptual)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, relative)
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.entries, 2);
    }

    #[test]
    fn transform_cache_key_includes_render_contract_scope() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new(4);
        let display_scope = ColorTransformOptions {
            cache_scope: color_transform_cache_scope("contract:display"),
            ..ColorTransformOptions::default()
        };
        let print_scope = ColorTransformOptions {
            cache_scope: color_transform_cache_scope("contract:print"),
            ..ColorTransformOptions::default()
        };

        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, display_scope)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, print_scope)
            .unwrap();
        cache
            .transform_builtin_srgb_to_srgb_for_proof(&pixels, display_scope)
            .unwrap();

        let metrics = cache.metrics();
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.entries, 2);
    }

    #[test]
    fn deterministic_fallback_policy_disables_icc_backend() {
        let pixels = [10u8, 20, 30];
        let mut cache = IccTransformCache::new(4);
        let portable = ColorTransformOptions {
            intent: ColorIntent::RelativeColorimetric,
            black_point_compensation: false,
            backend: ColorTransformBackend::PortableQcms,
            ..ColorTransformOptions::default()
        };
        assert!(
            cache
                .transform_builtin_srgb_to_srgb_for_proof(&pixels, portable)
                .is_some(),
            "portable qcms policy should provide the built-in proof transform"
        );
        let after_portable = cache.metrics();

        let deterministic = ColorTransformOptions {
            backend: ColorTransformBackend::DeterministicFallback,
            ..portable
        };
        assert!(
            cache
                .transform_builtin_srgb_to_srgb_for_proof(&pixels, deterministic)
                .is_none(),
            "deterministic fallback policy must not silently use an ICC backend"
        );
        let after_deterministic = cache.metrics();
        assert_eq!(after_deterministic.entries, after_portable.entries);
        assert_eq!(after_deterministic.misses, after_portable.misses);
    }

    #[test]
    fn portable_qcms_rejects_unsupported_cmyk_profile_without_panic() {
        let profile = include_bytes!("../../../../tests/fixtures/icc/PRMG_v2.0.1_MR.icc");
        let mut cache = IccTransformCache::new(4);
        let options = ColorTransformOptions {
            backend: ColorTransformBackend::PortableQcms,
            ..ColorTransformOptions::default()
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.transform_profile_to_srgb(profile, 4, &[0, 0, 0, 0], options)
        }));
        assert!(
            result.is_ok(),
            "portable qcms admission must reject unsupported CMYK profiles before transform construction panics"
        );
        assert!(
            result.unwrap().is_none(),
            "unsupported CMYK qcms profile should fail closed"
        );
        assert!(cache.metrics().unsupported_profiles >= 1);
    }

    #[test]
    fn cmyk_overprint_preserves_zero_ink_channels_in_preview() {
        let yellow_background = device_cmyk_to_srgb(0.0, 0.0, 1.0, 0.0);
        let cyan_only = [1.0, 0.0, 0.0, 0.0];
        let overprint = device_cmyk_overprint_preview_srgb(yellow_background, cyan_only, true);
        let knockout = device_cmyk_overprint_preview_srgb(yellow_background, cyan_only, false);
        let expected_green = device_cmyk_to_srgb(1.0, 0.0, 1.0, 0.0);
        let overprint = bytes(overprint);
        let expected_green = bytes(expected_green);
        for (actual, expected) in overprint.into_iter().zip(expected_green) {
            assert!((actual as i16 - expected as i16).abs() <= 8);
        }
        assert_eq!(
            bytes(knockout),
            bytes(device_cmyk_to_srgb(1.0, 0.0, 0.0, 0.0))
        );
    }

    #[test]
    fn srgb_identity_fidelity_vectors_pass() {
        let probes = srgb_identity_fidelity_probes();
        assert_eq!(probes.len(), 3);
        assert!(probes.iter().all(|probe| probe.passed));
        assert!(SUPPORTED_QCMS_INTENTS
            .iter()
            .any(|intent| intent.as_str() == "absolute_colorimetric"));
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn lcms2_srgb_profile_bytes() -> Vec<u8> {
        lcms2::Profile::new_srgb().icc().unwrap()
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn lcms2_gray_profile_bytes() -> Vec<u8> {
        let white = lcms2::CIExyY {
            x: 0.3457,
            y: 0.3585,
            Y: 1.0,
        };
        let curve = lcms2::ToneCurve::new(2.2);
        lcms2::Profile::new_gray(&white, &curve)
            .unwrap()
            .icc()
            .unwrap()
    }

    #[cfg(feature = "native-cmm-lcms2")]
    fn lcms2_cmyk_profile_bytes() -> Vec<u8> {
        include_bytes!("../../../../tests/fixtures/icc/PRMG_v2.0.1_MR.icc").to_vec()
    }

    #[cfg(feature = "native-cmm-lcms2")]
    #[test]
    fn native_lcms2_rgb_gray_and_cmyk_transforms_are_real() {
        let mut cache = IccTransformCache::new(8);
        let options = ColorTransformOptions {
            intent: ColorIntent::RelativeColorimetric,
            black_point_compensation: true,
            backend: ColorTransformBackend::NativeLittleCms,
            ..ColorTransformOptions::default()
        };

        let rgb_profile = lcms2_srgb_profile_bytes();
        let rgb_pixels = [0u8, 64, 128, 255, 128, 0];
        let (rgb, channels) = cache
            .transform_profile_to_srgb(&rgb_profile, 3, &rgb_pixels, options)
            .unwrap();
        assert_eq!(channels, 3);
        assert_eq!(rgb.len(), rgb_pixels.len());
        assert!(rgb.iter().zip(rgb_pixels).all(|(a, b)| a.abs_diff(b) <= 1));

        let gray_profile = lcms2_gray_profile_bytes();
        let (gray, channels) = cache
            .transform_profile_to_srgb(&gray_profile, 1, &[0, 128, 255], options)
            .unwrap();
        assert_eq!(channels, 3);
        assert_eq!(gray.len(), 9);
        assert!(gray[0] <= 1 && gray[1] <= 1 && gray[2] <= 1, "{gray:?}");
        assert!(
            gray[6] >= 250 && gray[7] >= 250 && gray[8] >= 250,
            "{gray:?}"
        );

        let cmyk_profile = lcms2_cmyk_profile_bytes();
        let (cmyk, channels) = cache
            .transform_profile_to_srgb(&cmyk_profile, 4, &[0, 0, 0, 0, 255, 0, 0, 0], options)
            .unwrap();
        assert_eq!(channels, 3);
        assert_eq!(cmyk.len(), 6);

        let metrics = cache.metrics();
        assert!(metrics.native_lcms2_transforms >= 3, "{metrics:?}");
        assert_eq!(metrics.fallback_qcms_transforms, 0);
    }

    #[cfg(feature = "native-cmm-lcms2")]
    #[test]
    fn native_lcms2_malformed_and_mismatched_profiles_fail_closed() {
        let mut cache = IccTransformCache::new(4);
        let options = ColorTransformOptions {
            backend: ColorTransformBackend::NativeLittleCms,
            ..ColorTransformOptions::default()
        };
        assert!(cache
            .transform_profile_to_srgb(b"not an icc profile", 3, &[0, 0, 0], options,)
            .is_none());
        let gray_profile = lcms2_gray_profile_bytes();
        assert!(cache
            .transform_profile_to_srgb(&gray_profile, 3, &[0, 0, 0], options,)
            .is_none());
        let metrics = cache.metrics();
        assert!(metrics.invalid_profiles >= 1, "{metrics:?}");
        assert!(metrics.unsupported_profiles >= 1, "{metrics:?}");
        assert!(metrics.native_lcms2_failures >= 2, "{metrics:?}");
    }

    #[cfg(feature = "native-cmm-lcms2")]
    #[test]
    fn native_lcms2_output_intent_soft_proofing_is_available() {
        let profile = lcms2_srgb_profile_bytes();
        assert_eq!(
            native_lcms2_profile_valid_for_components(&profile, Some(3)),
            Some(true)
        );
        let pixels = [12u8, 40, 80, 200, 220, 240];
        let proofed = proof_srgb_via_output_intent(
            &profile,
            &pixels,
            ColorTransformOptions {
                intent: ColorIntent::AbsoluteColorimetric,
                black_point_compensation: true,
                backend: ColorTransformBackend::NativeLittleCms,
                ..ColorTransformOptions::default()
            },
        )
        .unwrap();
        assert_eq!(proofed.len(), pixels.len());
    }

    #[cfg(feature = "native-cmm-lcms2")]
    #[test]
    fn output_intent_proof_transform_uses_bounded_cache() {
        let profile = lcms2_srgb_profile_bytes();
        let pixels = [12u8, 40, 80, 200, 220, 240];
        let mut cache = IccTransformCache::new(4);
        let proof_scope = ColorTransformOptions {
            intent: ColorIntent::AbsoluteColorimetric,
            black_point_compensation: true,
            backend: ColorTransformBackend::NativeLittleCms,
            cache_scope: color_transform_cache_scope("contract:proof"),
        };
        let print_scope = ColorTransformOptions {
            cache_scope: color_transform_cache_scope("contract:print"),
            ..proof_scope
        };

        let first = cache
            .proof_srgb_via_output_intent(&profile, &pixels, proof_scope)
            .unwrap();
        let second = cache
            .proof_srgb_via_output_intent(&profile, &pixels, proof_scope)
            .unwrap();
        let other_scope = cache
            .proof_srgb_via_output_intent(&profile, &pixels, print_scope)
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(other_scope.len(), pixels.len());
        let metrics = cache.metrics();
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.entries, 2);
        assert_eq!(metrics.admissions, 2);
        assert_eq!(metrics.rejections, 0);
        assert!(metrics.bytes_used > 0);
        assert!(metrics.bytes_used <= metrics.max_bytes);
        assert_eq!(metrics.native_lcms2_transforms, 3);
    }
}
