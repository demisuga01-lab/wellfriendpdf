//! Prompt 08 text clipping fixtures.
//!
//! These tests build small deterministic PDFs where text rendering mode 7
//! accumulates glyph outlines and the following paint operation must be clipped
//! to those outlines.

use oxide_engine::{ContentEngine, PixelBuffer};

struct PdfBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: &str) -> usize {
        self.objects.push(body.as_bytes().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, dict_extra: &str, stream: &[u8]) -> usize {
        let mut body =
            format!("<< /Length {} {} >>\nstream\n", stream.len(), dict_extra).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
        self.objects.len()
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref_start = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                offsets.len() + 1,
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }
}

fn add_standard_page(b: &mut PdfBuilder, resources_extra: &str, content: &[u8]) {
    b.add("<< /Type /Catalog /Pages 2 0 R >>"); // 1
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"); // 2
    b.add(&format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 100] /Contents 4 0 R \
         /Resources << /Font << /Helvetica 5 0 R >> {} >> >>",
        resources_extra
    )); // 3
    b.add_stream("", content); // 4
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"); // 5
}

fn text_clip_prefix() -> &'static str {
    "1 1 1 rg 0 0 160 100 re f\n\
     BT /Helvetica 72 Tf 7 Tr 20 25 Td (HI) Tj ET\n"
}

fn render(pdf: Vec<u8>) -> PixelBuffer {
    ContentEngine::open_bytes(pdf)
        .expect("prompt08 text clip PDF opens")
        .render_page(1, 72)
        .expect("prompt08 text clip PDF renders")
}

fn count_pixels(buf: &PixelBuffer, predicate: impl Fn([u8; 4]) -> bool) -> usize {
    let mut count = 0;
    for y in 0..buf.height as i32 {
        for x in 0..buf.width as i32 {
            if predicate(buf.get_pixel(x, y)) {
                count += 1;
            }
        }
    }
    count
}

fn assert_text_clip_shape(buf: &PixelBuffer, predicate: impl Fn([u8; 4]) -> bool) {
    let outside = buf.get_pixel(4, 4);
    assert!(
        outside[0] > 230 && outside[1] > 230 && outside[2] > 230,
        "paint leaked outside text clip at page corner: {:?}",
        outside
    );
    let painted = count_pixels(buf, predicate);
    let total = (buf.width * buf.height) as usize;
    assert!(
        painted > 100,
        "text clip should expose a visible amount of later paint; painted={painted}"
    );
    assert!(
        painted < total / 3,
        "later paint should be clipped to text outlines, not cover the page; painted={painted} total={total}"
    );
}

#[test]
fn text_render_mode_7_clips_subsequent_fill() {
    let mut b = PdfBuilder::new();
    let content = format!("{}1 0 0 rg 0 0 160 100 re f\n", text_clip_prefix());
    add_standard_page(&mut b, "", content.as_bytes());

    let buf = render(b.build());
    assert_text_clip_shape(&buf, |p| p[0] > 160 && p[1] < 90 && p[2] < 90);
}

#[test]
fn text_clip_masks_image_xobject() {
    let mut b = PdfBuilder::new();
    let content = format!("{}q 160 0 0 100 0 0 cm /Im1 Do Q\n", text_clip_prefix());
    add_standard_page(&mut b, "/XObject << /Im1 6 0 R >>", content.as_bytes());
    b.add_stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 \
         /ColorSpace /DeviceRGB /BitsPerComponent 8",
        &[0, 0, 255],
    ); // 6

    let buf = render(b.build());
    assert_text_clip_shape(&buf, |p| p[2] > 150 && p[0] < 100 && p[1] < 100);
}

#[test]
fn text_clip_masks_form_xobject() {
    let mut b = PdfBuilder::new();
    let content = format!("{}q /Fm1 Do Q\n", text_clip_prefix());
    add_standard_page(&mut b, "/XObject << /Fm1 6 0 R >>", content.as_bytes());
    b.add_stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 160 100] /Resources << >>",
        b"0 0.8 0 rg 0 0 160 100 re f\n",
    ); // 6

    let buf = render(b.build());
    assert_text_clip_shape(&buf, |p| p[1] > 120 && p[0] < 100 && p[2] < 100);
}

#[test]
fn text_clip_masks_axial_shading() {
    let mut b = PdfBuilder::new();
    let content = format!("{} /Sh1 sh\n", text_clip_prefix());
    add_standard_page(&mut b, "/Shading << /Sh1 7 0 R >>", content.as_bytes());
    b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>"); // 6
    b.add(
        "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 160 0] \
         /Domain [0 1] /Extend [true true] /Function 6 0 R >>",
    ); // 7

    let buf = render(b.build());
    assert_text_clip_shape(&buf, |p| {
        (p[0] > 100 || p[2] > 100) && p[1] < 120 && !(p[0] > 230 && p[2] > 230)
    });
}

#[test]
fn text_clip_masks_colored_tiling_pattern() {
    let mut b = PdfBuilder::new();
    let content = format!(
        "{} /Pattern cs /P1 scn 0 0 160 100 re f\n",
        text_clip_prefix()
    );
    add_standard_page(&mut b, "/Pattern << /P1 6 0 R >>", content.as_bytes());
    b.add_stream(
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
         /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
        b"0 0.75 0 rg 0 0 10 10 re f\n",
    ); // 6

    let buf = render(b.build());
    assert_text_clip_shape(&buf, |p| p[1] > 120 && p[0] < 100 && p[2] < 100);
}
