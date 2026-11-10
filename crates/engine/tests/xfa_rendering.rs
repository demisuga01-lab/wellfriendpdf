//! XFA rendering: static (foreground) field appearances drawn over PDF pages
//! and dynamic (full) XFA pages produced from the XFA layout.

use wellfriendpdf_engine::{
    CancelToken, ContentEngine, PixelBuffer, RenderMode, XfaFormType, XfaRenderPolicy,
};

struct PdfFixtureBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfFixtureBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: impl AsRef<[u8]>) -> usize {
        self.objects.push(body.as_ref().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, extra: &str, stream: &[u8]) -> usize {
        let mut body = format!("<< /Length {} {extra} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.add(body)
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

const STATIC_CONFIG: &str = r#"<config xmlns="http://www.xfa.org/schema/xci/3.0/"><present><pdf><interactive>1</interactive></pdf></present></config>"#;
const DYNAMIC_CONFIG: &str = r#"<config xmlns="http://www.xfa.org/schema/xci/3.0/"><acrobat><acrobat7><dynamicRender>required</dynamicRender></acrobat7></acrobat></config>"#;

const STATIC_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form1" layout="position">
  <pageSet><pageArea name="P" id="P"><contentArea x="0pt" y="0pt" w="200pt" h="200pt"/><medium short="200pt" long="200pt"/></pageArea></pageSet>
  <field name="name" x="20pt" y="20pt" w="160pt" h="30pt">
    <ui><textEdit/></ui>
    <font typeface="Helvetica" size="14pt"/>
    <border><edge thickness="2pt"><color value="255,0,0"/></edge></border>
  </field>
  <field name="agree" x="20pt" y="80pt" w="20pt" h="20pt">
    <ui><checkButton size="20pt" mark="square"/></ui>
    <items><integer>1</integer><integer>0</integer></items>
  </field>
  <draw name="band" x="20pt" y="140pt" w="160pt" h="40pt">
    <value><rectangle><edge presence="hidden"/><fill><color value="255,255,255"/><linear type="toRight"><color value="0,0,255"/></linear></fill></rectangle></value>
  </draw>
</subform></template>"#;

const STATIC_DATA: &str = r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form1><name>WWWW</name><agree>1</agree></form1></xfa:data></xfa:datasets>"#;

/// A one-page 200x200 PDF with an AcroForm /XFA packet array. `annots` and
/// `content` are optional page parts.
fn xfa_pdf(
    config: &str,
    template: &str,
    data: &str,
    content: Option<&[u8]>,
    widget: bool,
) -> Vec<u8> {
    let mut builder = PdfFixtureBuilder::new();
    let annots = if widget { "/Annots [9 0 R]" } else { "" };
    let fields = if widget {
        "/Fields [9 0 R]"
    } else {
        "/Fields []"
    };
    builder.add("<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    let contents = if content.is_some() {
        "/Contents 8 0 R"
    } else {
        ""
    };
    builder.add(format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] {contents} {annots} >>"
    ));
    builder.add(format!(
        "<< {fields} /XFA [(config) 5 0 R (template) 6 0 R (datasets) 7 0 R] >>"
    ));
    builder.add_stream("", config.as_bytes());
    builder.add_stream("", template.as_bytes());
    builder.add_stream("", data.as_bytes());
    builder.add_stream("", content.unwrap_or(b"q Q"));
    if widget {
        builder.add(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (w) /Rect [100 100 150 150] /P 3 0 R /AP << /N 10 0 R >> >>",
        );
        builder.add_stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 50 50]",
            b"0 0.8 0 rg 0 0 50 50 re f",
        );
    }
    builder.build()
}

fn pixel(buffer: &PixelBuffer, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * buffer.width + x) * 4) as usize;
    let bytes = buffer.rgba_bytes();
    [
        bytes[index],
        bytes[index + 1],
        bytes[index + 2],
        bytes[index + 3],
    ]
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 180 && p[1] < 90 && p[2] < 90
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 240 && p[1] > 240 && p[2] > 240
}

fn dark_pixels(buffer: &PixelBuffer, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    let mut count = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pixel(buffer, x, y);
            if u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 300 {
                count += 1;
            }
        }
    }
    count
}

fn render_with(engine: &ContentEngine, policy: XfaRenderPolicy) -> PixelBuffer {
    let mut contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .expect("contract");
    contract.xfa = policy;
    engine
        .render_page_with_contract(&contract, &CancelToken::none())
        .expect("render")
}

#[test]
fn static_xfa_without_widgets_draws_fields_from_datasets() {
    let pdf = xfa_pdf(STATIC_CONFIG, STATIC_TEMPLATE, STATIC_DATA, None, false);
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    assert_eq!(engine.xfa_form_type(), XfaFormType::Foreground);
    assert_eq!(engine.xfa_page_count().unwrap(), 1);
    for buffer in [
        engine.render_page(1, 72).unwrap(),
        render_with(&engine, XfaRenderPolicy::Auto),
    ] {
        // 2pt red border on the field's left edge.
        assert!(
            is_red(pixel(&buffer, 20, 35)),
            "{:?}",
            pixel(&buffer, 20, 35)
        );
        // The data value "WWWW" in 14pt text inside the field.
        assert!(dark_pixels(&buffer, 22, 22, 178, 48) > 40);
        // The checked square mark (box deflated by a quarter).
        assert!(dark_pixels(&buffer, 27, 87, 33, 93) >= 30);
        // Linear gradient: white on the left, blue on the right.
        let left = pixel(&buffer, 22, 160);
        let right = pixel(&buffer, 177, 160);
        assert!(left[0] > 200 && left[2] > 200, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    }
    let off = render_with(&engine, XfaRenderPolicy::Off);
    assert!(is_white(pixel(&off, 20, 35)));
    assert_eq!(dark_pixels(&off, 0, 0, 200, 200), 0);
}

#[test]
fn static_xfa_keeps_widget_appearances_in_auto_mode() {
    let pdf = xfa_pdf(
        STATIC_CONFIG,
        STATIC_TEMPLATE,
        STATIC_DATA,
        Some(b"0.9 g 0 0 5 5 re f"),
        true,
    );
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let auto = engine.render_page(1, 72).unwrap();
    let green = pixel(&auto, 125, 75);
    assert!(green[1] > 150 && green[0] < 60, "{green:?}");
    assert!(
        is_white(pixel(&auto, 20, 35)),
        "no XFA overlay over usable widgets"
    );
    let off = render_with(&engine, XfaRenderPolicy::Off);
    assert_eq!(auto.rgba_bytes(), off.rgba_bytes());

    // Forced foreground: XFA fields replace the widget annotations. The page
    // has content, so only fields are painted (the band draw is skipped).
    let forced = render_with(&engine, XfaRenderPolicy::Foreground);
    assert!(is_red(pixel(&forced, 20, 35)));
    assert!(is_white(pixel(&forced, 125, 75)), "widget suppressed");
    assert!(
        is_white(pixel(&forced, 177, 160)),
        "boilerplate draws stay with the page"
    );
    assert_eq!(
        engine.render_xfa_page(1, 72).unwrap().rgba_bytes(),
        forced.rgba_bytes()
    );
}

#[test]
fn pdf_without_xfa_is_unaffected_by_the_policy() {
    let mut builder = PdfFixtureBuilder::new();
    builder.add("<< /Type /Catalog /Pages 2 0 R >>");
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>");
    builder.add_stream("", b"1 0 0 rg 10 10 50 50 re f");
    let engine = ContentEngine::open_bytes(builder.build()).unwrap();
    assert_eq!(engine.xfa_form_type(), XfaFormType::None);
    assert_eq!(engine.xfa_page_count().unwrap(), 0);
    assert!(engine.render_xfa_page(1, 72).is_err());
    let base = engine.render_page(1, 72).unwrap();
    for policy in [
        XfaRenderPolicy::Auto,
        XfaRenderPolicy::Foreground,
        XfaRenderPolicy::Off,
    ] {
        assert_eq!(render_with(&engine, policy).rgba_bytes(), base.rgba_bytes());
    }
    let contract = engine
        .default_render_contract(1, 72, RenderMode::Compat)
        .unwrap();
    let json = serde_json::to_value(&contract).unwrap();
    assert!(
        json.get("xfa").is_none(),
        "Auto stays out of the cache identity"
    );
}

const DYNAMIC_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form1" layout="tb">
  <pageSet><pageArea name="P" id="P"><contentArea x="10pt" y="10pt" w="180pt" h="180pt"/><medium short="200pt" long="200pt"/></pageArea></pageSet>
  <subform name="row" layout="tb"><occur min="0" max="-1"/>
    <field name="v" w="100pt" h="50pt">
      <ui><textEdit/></ui>
      <font typeface="Arial" size="10pt"/>
      <border><edge thickness="1pt"/></border>
      <caption reserve="40pt"><value><text>Item</text></value></caption>
    </field>
  </subform>
  <subform name="tail" w="180pt" h="20pt"><breakBefore targetType="pageArea" target="P"/>
    <draw name="end" w="180pt" h="20pt"><value><text>End</text></value><font typeface="Times" size="12pt"/><para hAlign="center"/></draw>
  </subform>
</subform></template>"#;

fn dynamic_data(rows: usize) -> String {
    let mut data = String::from(
        r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form1>"#,
    );
    for index in 0..rows {
        data.push_str(&format!("<row><v>value {index}</v></row>"));
    }
    data.push_str("</form1></xfa:data></xfa:datasets>");
    data
}

#[test]
fn dynamic_xfa_lays_out_pages_beyond_the_pdf_page_count() {
    let pdf = xfa_pdf(
        DYNAMIC_CONFIG,
        DYNAMIC_TEMPLATE,
        &dynamic_data(7),
        Some(b"0 0 1 rg 0 0 10 10 re f"),
        false,
    );
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    assert_eq!(engine.xfa_form_type(), XfaFormType::Full);
    assert_eq!(engine.page_count().unwrap(), 1);
    // 7 rows of 50pt in 180pt content areas: 3 + 3 + 1, then the tail
    // breaks to a fourth page.
    assert_eq!(engine.xfa_page_count().unwrap(), 4);
    let page3 = engine.render_xfa_page(3, 72).unwrap();
    assert_eq!((page3.width, page3.height), (200, 200));
    // Field border at the content area's top-left; caption text inside.
    assert!(dark_pixels(&page3, 9, 9, 13, 60) > 20);
    assert!(dark_pixels(&page3, 12, 12, 48, 58) > 10, "caption text");
    // Only one row on page 3.
    assert_eq!(dark_pixels(&page3, 9, 62, 112, 190), 0);
    let page4 = engine.render_xfa_page(4, 72).unwrap();
    assert!(
        dark_pixels(&page4, 60, 10, 140, 30) > 10,
        "centered tail text"
    );
    assert!(engine.render_xfa_page(5, 72).is_err());
    assert!(engine.render_xfa_page(0, 72).is_err());

    // The PDF's own placeholder page is untouched in Auto mode.
    let page1 = engine.render_page(1, 72).unwrap();
    assert_eq!(
        page1.rgba_bytes(),
        render_with(&engine, XfaRenderPolicy::Off).rgba_bytes()
    );
    let corner = pixel(&page1, 5, 195);
    assert!(corner[2] > 200 && corner[0] < 60);

    let layout = ContentEngine::open_bytes(engine.xfa_layout_pdf().unwrap()).unwrap();
    assert_eq!(layout.page_count().unwrap(), 4);
}
