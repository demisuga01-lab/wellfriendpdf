//! Render-contract tests for [`RectSnapping::Pdfium`].

use crate::cancel::CancelToken;
use crate::render::buffer::{PixelBuffer, RenderMode};
use crate::render::RectSnapping;
use crate::ContentEngine;

fn pdf_with_content(width: u32, height: u32, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R >>"),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let startxref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn render(content: &str, mode: RenderMode, snapping: RectSnapping) -> PixelBuffer {
    let engine = ContentEngine::open_bytes(pdf_with_content(80, 40, content)).expect("open");
    let mut contract = engine
        .default_render_contract(1, 72, mode)
        .expect("default contract");
    contract.rect_snapping = snapping;
    engine
        .render_page_with_contract(&contract, &CancelToken::none())
        .expect("render")
}

/// Bounding box `(x0, y0, x1, y1)` (exclusive end) of non-white pixels, and
/// whether every such pixel is exactly `ink`.
fn ink_box(buf: &PixelBuffer, ink: [u8; 4]) -> (Option<(i32, i32, i32, i32)>, bool) {
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    let mut solid = true;
    for y in 0..buf.height as i32 {
        for x in 0..buf.width as i32 {
            let pixel = buf.get_pixel(x, y);
            if pixel == [255, 255, 255, 255] {
                continue;
            }
            solid &= pixel == ink;
            bounds = Some(match bounds {
                None => (x, y, x + 1, y + 1),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
            });
        }
    }
    (bounds, solid)
}

const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn pdfium_rect_snapping_fills_the_trimmed_outer_pixel_rectangle() {
    // Device x [10.3, 60.7], y [9.5, 29.7] (page height 40).
    let content = "0 0 1 rg 10.3 10.3 50.4 20.2 re f";
    for mode in [RenderMode::Compat, RenderMode::HighQuality] {
        let snapped = render(content, mode, RectSnapping::Pdfium);
        // Width ceil(50.4) = 51 = outer [10, 61); height ceil(20.2) = 21 =
        // outer [9, 30): nothing to trim, no anti-aliased edge pixels.
        assert_eq!(
            ink_box(&snapped, BLUE),
            (Some((10, 9, 61, 30)), true),
            "{mode:?}"
        );
    }
    // The default Compatibility mode rounds each edge (Splash stroke
    // adjustment): y [9.5, 29.7] -> [10, 30).
    let default = render(content, RenderMode::Compat, RectSnapping::Default);
    assert_eq!(ink_box(&default, BLUE), (Some((10, 10, 61, 30)), true));
    // HighQuality anti-aliases the fractional edges by default.
    let antialiased = render(content, RenderMode::HighQuality, RectSnapping::Default);
    assert!(!ink_box(&antialiased, BLUE).1);
}

#[test]
fn pdfium_rect_snapping_leaves_no_seam_between_abutting_fills() {
    // Two bars sharing the edge x = 30.55, drawn as separate fills.
    let content = "0 0 1 rg 10.3 5 20.25 20 re f 30.55 5 20.25 20 re f";
    let seam_column = 30;
    let row = 40 - 15;
    let antialiased = render(content, RenderMode::HighQuality, RectSnapping::Default);
    assert_ne!(
        antialiased.get_pixel(seam_column, row),
        BLUE,
        "anti-aliased abutting edges leave a lighter seam"
    );
    let snapped = render(content, RenderMode::HighQuality, RectSnapping::Pdfium);
    assert_eq!(ink_box(&snapped, BLUE), (Some((10, 15, 51, 35)), true));
}

#[test]
fn pdfium_rect_snapping_leaves_other_paths_antialiased() {
    // A triangle and a two-rectangle path are not PDFium rectangles.
    let triangle = "0 0 1 rg 10.3 5.2 m 60.7 5.2 l 35.5 33.3 l f";
    let default = render(triangle, RenderMode::Compat, RectSnapping::Default);
    let snapped = render(triangle, RenderMode::Compat, RectSnapping::Pdfium);
    assert_eq!(default.rgba_bytes(), snapped.rgba_bytes());
    let two = "0 0 1 rg 10.3 5.2 10.4 10.4 re 40.3 5.2 10.4 10.4 re f";
    let snapped = render(two, RenderMode::HighQuality, RectSnapping::Pdfium);
    assert!(
        !ink_box(&snapped, BLUE).1,
        "multi-subpath fills stay anti-aliased"
    );
}
