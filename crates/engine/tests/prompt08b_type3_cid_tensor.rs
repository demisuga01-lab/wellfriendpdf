//! Prompt 08B closure fixtures for Type3/CID text clipping and Type 7 posture.

use wellfriendpdf_engine::{ContentEngine, PixelBuffer};

const CID_FONT_BYTES: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");

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
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                offsets.len() + 1,
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }
}

fn render(pdf: Vec<u8>) -> PixelBuffer {
    ContentEngine::open_bytes(pdf)
        .expect("prompt08b PDF opens")
        .render_page(1, 72)
        .expect("prompt08b PDF renders")
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

fn red_pixels(buf: &PixelBuffer) -> usize {
    count_pixels(buf, |p| p[0] > 160 && p[1] < 90 && p[2] < 90)
}

fn green_pixels(buf: &PixelBuffer) -> usize {
    count_pixels(buf, |p| p[1] > 130 && p[0] < 120 && p[2] < 120)
}

fn assert_clipped(buf: &PixelBuffer, painted: usize) {
    let total = (buf.width * buf.height) as usize;
    assert!(
        painted > 80,
        "later paint should be visible inside text clip"
    );
    assert!(
        painted < total / 3,
        "later paint should be clipped to glyph geometry: painted={painted} total={total}"
    );
    let corner = buf.get_pixel(4, 4);
    assert!(
        corner[0] > 230 && corner[1] > 230 && corner[2] > 230,
        "paint leaked outside clip at page corner: {corner:?}"
    );
}

fn type3_pdf(render_mode: i32, text: &str, charproc: &[u8], after_text: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let content = format!(
        "1 1 1 rg 0 0 160 100 re f\n\
         BT /T3 72 Tf {render_mode} Tr 20 20 Td ({text}) Tj ET\n\
         {after_text}\n"
    );
    b.add("<< /Type /Catalog /Pages 2 0 R >>");
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 100] /Contents 4 0 R \
         /Resources << /Font << /T3 5 0 R >> >> >>",
    );
    b.add_stream("", content.as_bytes());
    b.add(
        "<< /Type /Font /Subtype /Type3 /Name /T3 /FontBBox [0 0 1000 1000] \
         /FontMatrix [0.001 0 0 0.001 0 0] /Encoding << /Type /Encoding /Differences [65 /A] >> \
         /FirstChar 65 /LastChar 65 /Widths [700] /CharProcs << /A 6 0 R >> /Resources << >> >>",
    );
    b.add_stream("", charproc);
    b.build()
}

#[test]
fn type3_render_mode_7_clips_subsequent_fill() {
    let pdf = type3_pdf(
        7,
        "A",
        b"700 0 d0 0 0 700 700 re f\n",
        "1 0 0 rg 0 0 160 100 re f",
    );
    let buf = render(pdf);
    assert_clipped(&buf, red_pixels(&buf));
}

#[test]
fn type3_render_modes_4_5_6_collect_path_clip() {
    for mode in [4, 5, 6] {
        let pdf = type3_pdf(
            mode,
            "A",
            b"700 0 d0 0 0 700 700 re f\n",
            "0 0.75 0 rg 0 0 160 100 re f",
        );
        let buf = render(pdf);
        assert_clipped(&buf, green_pixels(&buf));
    }
}

#[test]
fn type3_multiple_glyphs_accumulate_before_et() {
    let pdf = type3_pdf(
        7,
        "AA",
        b"700 0 d0 0 0 700 700 re f\n",
        "1 0 0 rg 0 0 160 100 re f",
    );
    let buf = render(pdf);
    let painted = red_pixels(&buf);
    assert!(
        painted > 4000,
        "two Type3 glyphs should accumulate a larger clip region: {painted}"
    );
    assert!(painted < 9000, "clip should still be bounded: {painted}");
}

#[test]
fn type3_image_only_charproc_fails_closed_without_bbox_clip() {
    let pdf = type3_pdf(
        7,
        "A",
        b"700 0 d0 BI /W 1 /H 1 /CS /RGB /BPC 8 ID \xFF\x00\x00 EI\n",
        "1 0 0 rg 0 0 160 100 re f",
    );
    let buf = render(pdf);
    assert_eq!(
        red_pixels(&buf),
        0,
        "unsupported Type3 clip should fail closed"
    );
}

#[test]
fn type3_resource_heavy_charproc_fails_closed() {
    let pdf = type3_pdf(7, "A", b"700 0 d0 /Im1 Do\n", "1 0 0 rg 0 0 160 100 re f");
    let buf = render(pdf);
    assert_eq!(
        red_pixels(&buf),
        0,
        "resource-heavy Type3 clip should fail closed"
    );
}

fn cid_gid_for_a() -> u16 {
    let face = ttf_parser::Face::parse(CID_FONT_BYTES, 0).expect("LiberationSans parses");
    face.glyph_index('A').expect("A glyph").0
}

fn cid_pdf(
    resources_extra: &str,
    after_text: &str,
    add_extra: impl FnOnce(&mut PdfBuilder),
) -> Vec<u8> {
    let gid = cid_gid_for_a();
    let map = [0u8, 0, (gid >> 8) as u8, (gid & 0xFF) as u8];
    let content = format!(
        "1 1 1 rg 0 0 160 100 re f\n\
         BT /CIDF 72 Tf 7 Tr 20 25 Td <0001> Tj ET\n\
         {after_text}\n"
    );
    let mut b = PdfBuilder::new();
    b.add("<< /Type /Catalog /Pages 2 0 R >>");
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(&format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 100] /Contents 4 0 R \
         /Resources << /Font << /CIDF 5 0 R >> {resources_extra} >> >>"
    ));
    b.add_stream("", content.as_bytes());
    b.add("<< /Type /Font /Subtype /Type0 /BaseFont /LiberationSans /Encoding /Identity-H /DescendantFonts [6 0 R] >>");
    b.add("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /LiberationSans /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R /W [1 [722]] /CIDToGIDMap 9 0 R >>");
    b.add("<< /Type /FontDescriptor /FontName /LiberationSans /Flags 4 /FontBBox [-600 -300 1400 1100] /ItalicAngle 0 /Ascent 905 /Descent -212 /CapHeight 700 /StemV 80 /FontFile2 8 0 R >>");
    b.add_stream("", CID_FONT_BYTES);
    b.add_stream("", &map);
    add_extra(&mut b);
    b.build()
}

#[test]
fn cid_identity_h_text_clip_masks_image_paint() {
    let pdf = cid_pdf(
        "/XObject << /Im1 10 0 R >>",
        "q 160 0 0 100 0 0 cm /Im1 Do Q",
        |b| {
            b.add_stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8",
                &[0, 0, 255],
            );
        },
    );
    let buf = render(pdf);
    assert_clipped(&buf, count_pixels(&buf, |p| p[2] > 150 && p[0] < 100));
}

#[test]
fn cid_identity_h_text_clip_masks_form_shading_and_pattern_paint() {
    let form_pdf = cid_pdf("/XObject << /Fm1 10 0 R >>", "q /Fm1 Do Q", |b| {
        b.add_stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 160 100] /Resources << >>",
            b"1 0 0 rg 0 0 160 100 re f\n",
        );
    });
    let form_buf = render(form_pdf);
    assert_clipped(&form_buf, red_pixels(&form_buf));

    let shading_pdf = cid_pdf("/Shading << /Sh1 11 0 R >>", "/Sh1 sh", |b| {
        b.add("<< /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >>");
        b.add("<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 160 0] /Domain [0 1] /Extend [true true] /Function 10 0 R >>");
    });
    let shading_buf = render(shading_pdf);
    let shading_painted = count_pixels(&shading_buf, |p| (p[0] > 120 || p[2] > 120) && p[1] < 140);
    assert_clipped(&shading_buf, shading_painted);

    let pattern_pdf = cid_pdf(
        "/Pattern << /P1 10 0 R >>",
        "/Pattern cs /P1 scn 0 0 160 100 re f",
        |b| {
            b.add_stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
                b"0 0.75 0 rg 0 0 10 10 re f\n",
            );
        },
    );
    let pattern_buf = render(pattern_pdf);
    assert_clipped(&pattern_buf, green_pixels(&pattern_buf));
}

#[test]
fn cid_missing_outline_fails_closed() {
    let map = [0u8, 0, 0xFF, 0xFF];
    let content = "1 1 1 rg 0 0 160 100 re f\n\
                   BT /CIDF 72 Tf 7 Tr 20 25 Td <0001> Tj ET\n\
                   1 0 0 rg 0 0 160 100 re f\n";
    let mut b = PdfBuilder::new();
    b.add("<< /Type /Catalog /Pages 2 0 R >>");
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 100] /Contents 4 0 R /Resources << /Font << /CIDF 5 0 R >> >> >>");
    b.add_stream("", content.as_bytes());
    b.add("<< /Type /Font /Subtype /Type0 /BaseFont /LiberationSans /Encoding /Identity-H /DescendantFonts [6 0 R] >>");
    b.add("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /LiberationSans /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R /W [1 [722]] /CIDToGIDMap 9 0 R >>");
    b.add("<< /Type /FontDescriptor /FontName /LiberationSans /Flags 4 /FontBBox [-600 -300 1400 1100] /ItalicAngle 0 /Ascent 905 /Descent -212 /CapHeight 700 /StemV 80 /FontFile2 8 0 R >>");
    b.add_stream("", CID_FONT_BYTES);
    b.add_stream("", &map);

    let buf = render(b.build());
    assert_eq!(
        red_pixels(&buf),
        0,
        "missing CID outline should fail closed"
    );
}
