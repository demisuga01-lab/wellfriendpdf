pub mod ccitt;
pub mod decoder;
pub mod encoder;
pub(crate) mod indexed_samples;
pub mod jbig2;
pub mod jpx;
pub mod locator;
pub mod raster;
pub(crate) mod sample_decode;
pub mod smask;
pub mod tiff;

pub use smask::SmaskLoader;

/// Whether the last filter of an image stream produces bilevel data (JBIG2,
/// CCITT fax), so an omitted /BitsPerComponent can only mean 1.
pub(crate) fn bilevel_terminal_filter(filters: &[String]) -> bool {
    matches!(
        filters.last().map(String::as_str),
        Some("JBIG2Decode" | "CCITTFaxDecode" | "CCF")
    )
}
