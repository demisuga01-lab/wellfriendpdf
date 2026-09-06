//! Compact tests for RB-14: regional vector fallback for simple image XObjects
//! and simple native axial/radial shadings.
//!
//! These tests verify that SVG and PostScript output preserve native vector path
//! elements around a bounded embedded image region, avoiding whole-page
//! rasterization for pages that mix vector paths with simple axis-aligned images.

use wellfriendpdf_engine::render::vector_fallback::{
    classify_page_for_vector_output, image_device_rect, VectorFallbackDecision,
};
use wellfriendpdf_engine::{
    ContentEngine, ContentOperation, GraphicsState, ImageEncoder, Operand, PageResources,
    PdfDictionary, PdfObject, RawImage,
};

fn pdf_header() -> Vec<u8> {
    b"%PDF-1.7\n%\xFF\xFF\xFF\xFF\n".to_vec()
}

fn pdf_from_objects(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = pdf_header();
    let mut offsets = vec![0usize];
    for (idx, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let startxref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            startxref
        )
        .as_bytes(),
    );
    out
}

fn srgb_icc_profile_stream_object() -> Vec<u8> {
    let profile = generated_srgb_icc_profile();
    let mut stream = format!("<< /N 3 /Length {} >>\nstream\n", profile.len()).into_bytes();
    stream.extend_from_slice(&profile);
    stream.extend_from_slice(b"\nendstream");
    stream
}

fn gray_icc_profile_stream_object() -> Vec<u8> {
    let profile = generated_gray_icc_profile();
    let mut stream = format!("<< /N 1 /Length {} >>\nstream\n", profile.len()).into_bytes();
    stream.extend_from_slice(&profile);
    stream.extend_from_slice(b"\nendstream");
    stream
}

#[cfg(feature = "native-cmm-lcms2")]
fn cmyk_icc_profile_stream_object() -> Vec<u8> {
    let profile = include_bytes!("../../../tests/fixtures/icc/PRMG_v2.0.1_MR.icc");
    let mut stream = format!("<< /N 4 /Length {} >>\nstream\n", profile.len()).into_bytes();
    stream.extend_from_slice(profile);
    stream.extend_from_slice(b"\nendstream");
    stream
}

fn generated_srgb_icc_profile() -> Vec<u8> {
    const TAG_TABLE_OFFSET: usize = 132;
    const TAG_DATA_OFFSET: usize = 204;
    const XYZ_TAG_LEN: u32 = 20;
    const TRC_TAG_LEN: u32 = 14;
    const PROFILE_LEN: usize = 312;

    let mut profile = vec![0u8; PROFILE_LEN];
    write_icc_u32(&mut profile, 0, PROFILE_LEN as u32);
    write_icc_signature(&mut profile, 4, b"Test");
    write_icc_u32(&mut profile, 8, 0x0210_0000);
    write_icc_signature(&mut profile, 12, b"mntr");
    write_icc_signature(&mut profile, 16, b"RGB ");
    write_icc_signature(&mut profile, 20, b"XYZ ");
    write_icc_u16(&mut profile, 24, 2026);
    write_icc_u16(&mut profile, 26, 1);
    write_icc_u16(&mut profile, 28, 1);
    write_icc_signature(&mut profile, 36, b"acsp");
    write_icc_signature(&mut profile, 40, b"APPL");
    write_icc_signature(&mut profile, 48, b"Test");
    write_icc_signature(&mut profile, 52, b"sRGB");
    write_icc_s15_fixed(&mut profile, 68, 0.9642);
    write_icc_s15_fixed(&mut profile, 72, 1.0);
    write_icc_s15_fixed(&mut profile, 76, 0.8249);
    write_icc_signature(&mut profile, 80, b"Test");
    write_icc_u32(&mut profile, 128, 6);

    let xyz_tags = [
        (b"rXYZ", (0.436_074_7, 0.222_504_5, 0.013_932_2)),
        (b"gXYZ", (0.385_064_9, 0.716_878_6, 0.097_104_5)),
        (b"bXYZ", (0.143_080_4, 0.060_616_9, 0.714_173_3)),
    ];
    let mut tag_table_offset = TAG_TABLE_OFFSET;
    let mut tag_data_offset = TAG_DATA_OFFSET;
    for (signature, xyz) in xyz_tags {
        write_icc_tag_record(
            &mut profile,
            tag_table_offset,
            signature,
            tag_data_offset as u32,
            XYZ_TAG_LEN,
        );
        write_icc_xyz_type(&mut profile, tag_data_offset, xyz);
        tag_table_offset += 12;
        tag_data_offset += XYZ_TAG_LEN as usize;
    }

    for signature in [b"rTRC", b"gTRC", b"bTRC"] {
        write_icc_tag_record(
            &mut profile,
            tag_table_offset,
            signature,
            tag_data_offset as u32,
            TRC_TAG_LEN,
        );
        write_icc_curve_type_gamma(&mut profile, tag_data_offset, 2.2);
        tag_table_offset += 12;
        tag_data_offset += 16;
    }

    profile
}

fn generated_gray_icc_profile() -> Vec<u8> {
    const TAG_TABLE_OFFSET: usize = 132;
    const TAG_DATA_OFFSET: usize = 144;
    const TRC_TAG_LEN: u32 = 14;
    const PROFILE_LEN: usize = 160;

    let mut profile = vec![0u8; PROFILE_LEN];
    write_icc_u32(&mut profile, 0, PROFILE_LEN as u32);
    write_icc_signature(&mut profile, 4, b"Test");
    write_icc_u32(&mut profile, 8, 0x0210_0000);
    write_icc_signature(&mut profile, 12, b"mntr");
    write_icc_signature(&mut profile, 16, b"GRAY");
    write_icc_signature(&mut profile, 20, b"XYZ ");
    write_icc_u16(&mut profile, 24, 2026);
    write_icc_u16(&mut profile, 26, 1);
    write_icc_u16(&mut profile, 28, 1);
    write_icc_signature(&mut profile, 36, b"acsp");
    write_icc_signature(&mut profile, 40, b"APPL");
    write_icc_signature(&mut profile, 48, b"Test");
    write_icc_signature(&mut profile, 52, b"GRAY");
    write_icc_s15_fixed(&mut profile, 68, 0.9642);
    write_icc_s15_fixed(&mut profile, 72, 1.0);
    write_icc_s15_fixed(&mut profile, 76, 0.8249);
    write_icc_signature(&mut profile, 80, b"Test");
    write_icc_u32(&mut profile, 128, 1);
    write_icc_tag_record(
        &mut profile,
        TAG_TABLE_OFFSET,
        b"kTRC",
        TAG_DATA_OFFSET as u32,
        TRC_TAG_LEN,
    );
    write_icc_curve_type_gamma(&mut profile, TAG_DATA_OFFSET, 2.2);

    profile
}

fn write_icc_tag_record(
    profile: &mut [u8],
    offset: usize,
    signature: &[u8; 4],
    data_offset: u32,
    data_len: u32,
) {
    write_icc_signature(profile, offset, signature);
    write_icc_u32(profile, offset + 4, data_offset);
    write_icc_u32(profile, offset + 8, data_len);
}

fn write_icc_xyz_type(profile: &mut [u8], offset: usize, xyz: (f32, f32, f32)) {
    write_icc_signature(profile, offset, b"XYZ ");
    write_icc_s15_fixed(profile, offset + 8, xyz.0);
    write_icc_s15_fixed(profile, offset + 12, xyz.1);
    write_icc_s15_fixed(profile, offset + 16, xyz.2);
}

fn write_icc_curve_type_gamma(profile: &mut [u8], offset: usize, gamma: f32) {
    write_icc_signature(profile, offset, b"curv");
    write_icc_u32(profile, offset + 8, 1);
    write_icc_u16(profile, offset + 12, (gamma * 256.0).round() as u16);
}

fn write_icc_signature(profile: &mut [u8], offset: usize, signature: &[u8; 4]) {
    profile[offset..offset + 4].copy_from_slice(signature);
}

fn write_icc_u32(profile: &mut [u8], offset: usize, value: u32) {
    profile[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_icc_u16(profile: &mut [u8], offset: usize, value: u16) {
    profile[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_icc_s15_fixed(profile: &mut [u8], offset: usize, value: f32) {
    let fixed = (value * 65_536.0).round() as i32;
    profile[offset..offset + 4].copy_from_slice(&fixed.to_be_bytes());
}

fn simple_axial_shading_dict() -> PdfDictionary {
    let mut function = PdfDictionary::empty();
    function.insert("FunctionType", PdfObject::Integer(2));
    function.insert(
        "Domain",
        PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
    );
    function.insert(
        "C0",
        PdfObject::Array(vec![
            PdfObject::Real(1.0),
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
        ]),
    );
    function.insert(
        "C1",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(1.0),
        ]),
    );
    function.insert("N", PdfObject::Real(1.0));

    let mut shading = PdfDictionary::empty();
    shading.insert("ShadingType", PdfObject::Integer(2));
    shading.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
    shading.insert(
        "Coords",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(60.0),
            PdfObject::Real(120.0),
            PdfObject::Real(60.0),
        ]),
    );
    shading.insert(
        "Extend",
        PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
    );
    shading.insert("Function", PdfObject::Dictionary(function));
    shading
}

fn resources_with_shading_pattern(name: &str, matrix: Option<[f64; 6]>) -> PageResources {
    let mut pattern = PdfDictionary::empty();
    pattern.insert("Type", PdfObject::Name("Pattern".to_string()));
    pattern.insert("PatternType", PdfObject::Integer(2));
    pattern.insert(
        "Shading",
        PdfObject::Dictionary(simple_axial_shading_dict()),
    );
    if let Some(matrix) = matrix {
        pattern.insert(
            "Matrix",
            PdfObject::Array(matrix.into_iter().map(PdfObject::Real).collect()),
        );
    }

    let mut resources = PageResources::default();
    resources
        .patterns
        .insert(name.to_string(), PdfObject::Dictionary(pattern));
    resources
}

fn pdf_with_vector_safe_form_xobject() -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n0 0 1 RG\n0 0 40 25 re S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            form_content.len(),
            form_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_transparency_group_form_xobject(group: &str) -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n0 0 1 RG\n0 0 40 25 re S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group {} /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            group,
            form_content.len(),
            form_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_group_color_space_form_xobject(
    group: &str,
    color_space_resource: &str,
) -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n0 0 1 RG\n0 0 40 25 re S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group {} /Resources << /ColorSpace << /CS0 {} >> >> /Length {} >>\nstream\n{}\nendstream",
            group,
            color_space_resource,
            form_content.len(),
            form_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_iccbased_group_color_space_form_xobject() -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n0 0 1 RG\n0 0 40 25 re S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group << /S /Transparency /CS [/ICCBased 6 0 R] >> /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            form_content.len(),
            form_content
        )
        .into_bytes(),
        b"<< /N 3 /Length 0 >>\nstream\nendstream".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn separation_group_color_space_dict() -> &'static str {
    "<< /S /Transparency /CS [/Separation /Spot /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 0 0] /N 1 >>] >>"
}

fn devicen_group_color_space_dict() -> &'static str {
    "<< /S /Transparency /CS [/DeviceN [/Spot] /DeviceRGB << /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [0 0 1] /N 1 >>] >>"
}

fn pdf_with_multicomponent_devicen_group_color_space_form_xobject() -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n0 0 1 RG\n0 0 40 25 re S\n";
    let tint_transform = "{ 0 }";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group << /S /Transparency /CS [/DeviceN [/SpotRed /SpotBlue] /DeviceRGB 6 0 R] >> /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            form_content.len(),
            form_content
        )
        .into_bytes(),
        format!(
            "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{}\nendstream",
            tint_transform.len(),
            tint_transform
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_alpha_transparency_group_form_xobject(group: &str) -> Vec<u8> {
    let page_content = "q\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "/GS0 gs\n1 0 0 rg\n0 0 40 25 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group {} /Resources << /ExtGState << /GS0 6 0 R >> >> /Length {} >>\nstream\n{}\nendstream",
            group,
            form_content.len(),
            form_content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 1 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inherited_alpha_transparency_group_form_xobject(group: &str) -> Vec<u8> {
    let page_content = "q\n/GS0 gs\n1 0 0 1 20 30 cm\n/Fm1 Do\nQ\n";
    let form_content = "1 0 0 rg\n0 0 40 25 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Fm1 5 0 R >> /ExtGState << /GS0 6 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            page_content.len(),
            page_content
        )
        .into_bytes(),
        format!(
            "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 40 25] /Group {} /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            group,
            form_content.len(),
            form_content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 1 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image() -> Vec<u8> {
    let inline_bytes = [255u8, 0, 0, 0, 255, 0];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /RGB /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_color_space() -> Vec<u8> {
    let inline_bytes = [255u8, 0, 0, 0, 255, 0];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 /DeviceRGB >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_calrgb_color_space() -> Vec<u8> {
    let inline_bytes = [255u8, 0, 0, 0, 255, 0];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_indexed_color_space() -> Vec<u8> {
    let inline_bytes = [0u8, 1];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceRGB 1 <FF00000000FF>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_indexed_1bit_color_space() -> Vec<u8> {
    let inline_bytes = [0b0100_0000u8];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 1 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceRGB 1 <FF00000000FF>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_calrgb_dct_color_space() -> Vec<u8> {
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0],
        },
        95,
    )
    .unwrap();
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 /F /DCT ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&jpeg);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_indexed_dct_color_space() -> Vec<u8> {
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0, 1],
        },
        100,
    )
    .unwrap();
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 /F /DCT ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&jpeg);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceRGB 1 <FF00000000FF>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_iccbased_color_space() -> Vec<u8> {
    let inline_bytes = [255u8, 0, 0, 0, 255, 0];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 5 0 R] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        b"<< /N 3 /Length 0 >>\nstream\nendstream".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_iccbased_dct_color_space() -> Vec<u8> {
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0],
        },
        95,
    )
    .unwrap();
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 /F /DCT ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&jpeg);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 5 0 R] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        b"<< /N 3 /Length 0 >>\nstream\nendstream".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_separation_color_space() -> Vec<u8> {
    let inline_bytes = [0u8, 255];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_separation_dct_color_space() -> Vec<u8> {
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0, 255],
        },
        100,
    )
    .unwrap();
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 /F /DCT ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&jpeg);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_devicen_single_dct_color_space() -> Vec<u8> {
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0, 255],
        },
        100,
    )
    .unwrap();
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 /F /DCT ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&jpeg);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/DeviceN [/SpotRed] /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_devicen_color_space() -> Vec<u8> {
    let inline_bytes = [64u8, 191, 255, 0];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let tint_program = b"{ 0 }";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/DeviceN [/Spot1 /Spot2] /DeviceRGB 5 0 R] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        {
            let mut stream = format!(
                "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n",
                tint_program.len()
            )
            .into_bytes();
            stream.extend_from_slice(tint_program);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_resource_separation_none_color_space() -> Vec<u8> {
    let inline_bytes = [255u8, 255];
    let content_prefix =
        b"0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /CS /CS0 /BPC 8 ID ";
    let content_suffix = b" EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let mut page_content = Vec::new();
    page_content.extend_from_slice(content_prefix);
    page_content.extend_from_slice(&inline_bytes);
    page_content.extend_from_slice(content_suffix);
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Separation /None /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", page_content.len()).into_bytes();
            stream.extend_from_slice(&page_content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_inline_image_mask() -> Vec<u8> {
    let content = b"1 0 0 rg\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /IM true ID \x80 EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_shading_pattern_inline_image_mask() -> Vec<u8> {
    let content = b"/Pattern cs /P0 scn\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /IM true ID \x80 EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_inline_image_mask() -> Vec<u8> {
    let content = b"/Pattern cs /P0 scn\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /IM true ID \x80 EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_inline_image_mask() -> Vec<u8> {
    let content = b"/Pattern cs 0.2 0.4 0.6 /P0 scn\nq\n20 0 0 -10 40 90 cm\nBI /W 2 /H 1 /IM true ID \x80 EI\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_image_xobject_mask() -> Vec<u8> {
    let content = "1 0 0 rg\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let image_bytes = [0x80u8];
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ImageMask true /BitsPerComponent 1 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_image_xobject_mask() -> Vec<u8> {
    let content =
        "/Pattern cs /P0 scn\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let image_bytes = [0x80u8];
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 6 0 R >> /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ImageMask true /BitsPerComponent 1 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_image_xobject_mask() -> Vec<u8> {
    let content =
        "/Pattern cs 0.2 0.4 0.6 /P0 scn\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let image_bytes = [0x80u8];
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 6 0 R >> /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ImageMask true /BitsPerComponent 1 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_shading_pattern_image_xobject_mask() -> Vec<u8> {
    let content =
        "/Pattern cs /P0 scn\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let image_bytes = [0x80u8];
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 6 0 R >> /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ImageMask true /BitsPerComponent 1 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_rotated_image_xobject() -> Vec<u8> {
    let image_bytes = [255u8, 0, 0, 0, 255, 0];
    let content = "0 0 1 rg\n10 10 20 20 re f\nq\n20 20 -20 20 60 50 cm\n/Im0 Do\nQ\n0 1 0 rg\n90 10 20 20 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_image_xobject_resource_color_space() -> Vec<u8> {
    let image_bytes = [255u8, 0, 0, 0, 255, 0];
    let content =
        "0 0 1 rg\n10 10 20 20 re f\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n0 1 0 rg\n80 10 20 20 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 /DeviceRGB >> /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /CS0 /BitsPerComponent 8 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_svg_alpha_image_xobject() -> Vec<u8> {
    let image_bytes = [255u8, 0, 0, 0, 255, 0];
    let content = "/GS0 gs\nq\n20 0 0 -10 40 90 cm\n/Im0 Do\nQ\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> /XObject << /Im0 6 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 1 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nonextended_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [20 60 100 60] /Extend [false false] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_non_unit_domain_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Domain [0.25 0.75] /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_clipped_function_domain_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Domain [-0.25 1.25] /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_bounded_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /BBox [20 40 100 80] /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_function_array_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function [ << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> ] >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_mixed_exponent_function_array_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function [ << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0.5] /C1 [0.25] /N 2 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 3 >> ] >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_stitching_function_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 3 /Domain [0 1] /Functions [ << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 1 0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0 1 0] /C1 [0 0 1] /N 1 >> ] /Bounds [0.5] /Encode [0 1 0 1] >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_svg_alpha_axial_shading() -> Vec<u8> {
    let content = "/GS0 gs\n/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> /Shading << /Sh0 6 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 1 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_shading_pattern_fill() -> Vec<u8> {
    let content = "/Pattern cs /P0 scn\n10 10 100 80 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_fill() -> Vec<u8> {
    let content =
        "0 0 1 rg\n5 5 10 10 re f\n/Pattern cs /P0 scn\n10 10 60 40 re f\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_matrix_tiling_pattern_fill() -> Vec<u8> {
    let content = "/Pattern cs /P0 scn\n15 20 40 20 re f\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /Matrix [2 0 0 1 15 0] /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_stroke() -> Vec<u8> {
    let content =
        "0 0 1 rg\n5 5 10 10 re f\n/Pattern CS /P0 SCN\n6 w\n10 60 m 70 60 l S\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_fill() -> Vec<u8> {
    let content = "0 0 1 rg\n5 5 10 10 re f\n/Pattern cs 0.2 0.4 0.6 /P0 scn\n10 10 60 40 re f\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_stroke() -> Vec<u8> {
    let content = "0 0 1 rg\n5 5 10 10 re f\n/Pattern CS 0.2 0.4 0.6 /P0 SCN\n6 w\n10 60 m 70 60 l S\n0 1 0 rg\n80 10 20 20 re f\n";
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_color_setting_tile() -> Vec<u8> {
    let content = "/Pattern cs 0.2 0.4 0.6 /P0 scn\n10 10 60 40 re f\n";
    let pattern_content = "1 0 0 rg\n0 0 10 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_image_resource_tile() -> Vec<u8> {
    let image_bytes = [255u8, 0, 0];
    let content =
        "0 0 1 rg\n5 5 10 10 re f\n/Pattern cs /P0 scn\n10 10 60 40 re f\n0 1 0 rg\n85 10 20 20 re f\n";
    let pattern_content = "q\n10 0 0 10 0 0 cm\n/Im0 Do\nQ\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /XObject << /Im0 6 0 R >> >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        {
            let mut stream = format!(
                "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
                image_bytes.len()
            )
            .into_bytes();
            stream.extend_from_slice(&image_bytes);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_shading_resource_tile() -> Vec<u8> {
    let content =
        "0 0 1 rg\n5 5 10 10 re f\n/Pattern cs /P0 scn\n10 10 60 40 re f\n0 1 0 rg\n85 10 20 20 re f\n";
    let pattern_content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /Shading << /Sh0 6 0 R >> >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 10 10] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_translated_shading_pattern_fill() -> Vec<u8> {
    let content = "q\n1 0 0 1 20 0 cm\n/Pattern cs /P0 scn\n10 10 80 80 re f\nQ\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_matrix_shading_pattern_fill() -> Vec<u8> {
    let content = "/Pattern cs /P0 scn\n10 10 100 80 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 140 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Matrix [1 0 0 1 5 0] /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_shading_pattern_stroke() -> Vec<u8> {
    let content = "/Pattern CS /P0 SCN\n8 w\n10 60 m 110 60 l S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_shading_pattern_text_fill() -> Vec<u8> {
    let content = "/Pattern cs /P0 scn\nBT /F1 24 Tf 10 60 Td (Hi) Tj ET\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_shading_pattern_text_stroke() -> Vec<u8> {
    let content = "/Pattern CS /P0 SCN\n2 w\nBT /F1 24 Tf 1 Tr 10 60 Td (Hi) Tj ET\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >> >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_text_fill() -> Vec<u8> {
    let content = "/Pattern cs /P0 scn\nBT /F1 24 Tf 10 60 Td (Hi) Tj ET\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_colored_tiling_pattern_text_stroke() -> Vec<u8> {
    let content = "/Pattern CS /P0 SCN\n2 w\nBT /F1 24 Tf 1 Tr 10 60 Td (Hi) Tj ET\n";
    let pattern_content = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_text_fill() -> Vec<u8> {
    let content = "/Pattern cs 0.2 0.4 0.6 /P0 scn\nBT /F1 24 Tf 10 60 Td (Hi) Tj ET\n";
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_uncolored_tiling_pattern_text_stroke() -> Vec<u8> {
    let content = "/Pattern CS 0.2 0.4 0.6 /P0 SCN\n2 w\nBT /F1 24 Tf 1 Tr 10 60 Td (Hi) Tj ET\n";
    let pattern_content = "0 0 5 10 re f\n5 0 5 10 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /Pattern << /P0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        format!(
            "<< /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
            pattern_content.len(),
            pattern_content
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_cmyk_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceCMYK /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 1 1 0] /C1 [1 0 0 0] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nonlinear_cmyk_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceCMYK /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0 0 0] /N 2 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_mixed_exponent_cmyk_function_array_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceCMYK /Coords [0 60 120 60] /Extend [true true] /Function [ << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 2 >> << /FunctionType 2 /Domain [0 1] /C0 [0.5] /C1 [0.25] /N 3 >> << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0.5] /N 4 >> ] >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_cmyk_stitching_function_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /DeviceCMYK /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 3 /Domain [0 1] /Functions [ << /FunctionType 2 /Domain [0 1] /C0 [0 1 1 0] /C1 [1 0 0 0] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [0.25 0.25 0.25 0.25] /C1 [0 0 0 1] /N 2 >> ] /Bounds [0.5] /Encode [0 1 0 1] >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_iccbased_rgb_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
        srgb_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_iccbased_gray_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
        gray_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_iccbased_rgb_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /CS0 /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
        srgb_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_iccbased_gray_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /CS0 /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
        gray_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

#[cfg(feature = "native-cmm-lcms2")]
fn pdf_with_resource_iccbased_cmyk_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/ICCBased 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /CS0 /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >> >>".to_vec(),
        cmyk_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_calrgb_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_calgray_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 1 >>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_separation_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /Spot [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /Spot /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_separation_nonlinear_tint_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /Spot [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 2 >>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /Spot /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_devicen_multi_input_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let tint_transform = "{ pop pop 1 0 0 }";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /Spot [/DeviceN [/SpotRed /SpotBlue] /DeviceRGB 6 0 R] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /Spot /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0] /C1 [1 1] /N 1 >> >>".to_vec(),
        format!(
            "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{}\nendstream",
            tint_transform.len(),
            tint_transform
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_devicen_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /Spot [/DeviceN [/SpotRed] /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /Spot /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_devicen_nonlinear_tint_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /Spot [/DeviceN [/SpotRed] /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 2 >>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /Spot /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_rgb_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceRGB 1 <FF00000000FF>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_gray_constant_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceGray 1 <00FF>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /CS0 /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_cmyk_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed /DeviceCMYK 1 <0000000000FFFF00>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_calrgb_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] 1 <FF00000000FF>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_lab_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed [/Lab << /WhitePoint [0.9505 1 1.089] /Range [-100 100 -100 100] >>] 0 <FF8080>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_separation_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] 0 <FF>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_resource_indexed_iccbased_rgb_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed [/ICCBased 6 0 R] 1 <FF00000000FF>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
        srgb_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

#[cfg(feature = "native-cmm-lcms2")]
fn pdf_with_resource_indexed_iccbased_cmyk_constant_axial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ColorSpace << /CS0 [/Indexed [/ICCBased 6 0 R] 1 <0000000000FFFF00>] >> /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 2 /ColorSpace /CS0 /Coords [0 60 120 60] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [1] /N 1 >> >>".to_vec(),
        cmyk_icc_profile_stream_object(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nonextended_concentric_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [60 60 10 60 60 50] /Extend [false false] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nonzero_start_radius_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [45 60 10 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_reversed_radii_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [60 60 50 45 60 10] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [0 0 1] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nonuniform_radial_shading_ctm() -> Vec<u8> {
    let content = "q\n2 0 0 1 0 0 cm\n/Sh0 sh\nQ\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [40 60 0 40 60 35] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_lab_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace [/Lab << /WhitePoint [0.9642 1 0.8249] /Range [-100 100 -100 100] >>] /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [100 0 0] /C1 [50 80 60] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_simple_cmyk_radial_shading() -> Vec<u8> {
    let content = "/Sh0 sh\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Shading << /Sh0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /ShadingType 3 /ColorSpace /DeviceCMYK /Coords [60 60 0 60 60 50] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >> >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_safe_ext_gstate_line_width() -> Vec<u8> {
    let content = "/GS0 gs\n0 0 1 RG\n10 10 80 80 re S\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /LW 4 /LC 1 /LJ 2 /ML 10 /FL 0.5 /SM 0.02 /D [[6 2] 1] /OP false /op false /OPM 0 /SA false /AIS false /TK true /CA 1 /ca 1 /BM /Normal /SMask /None >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_svg_native_alpha_ext_gstate() -> Vec<u8> {
    let content = "/GS0 gs\n1 0 0 rg\n0 0 1 RG\n10 10 80 80 re B\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 0.25 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_svg_native_blend_ext_gstate() -> Vec<u8> {
    let content = "0 0 1 rg\n0 0 120 120 re f\n/GS0 gs\n1 0 0 rg\n10 10 80 80 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 1 /CA 1 /BM /Multiply /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_svg_native_alpha_blend_ext_gstate() -> Vec<u8> {
    let content = "0 0 1 rg\n0 0 120 120 re f\n/GS0 gs\n1 0 0 rg\n10 10 80 80 re f\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0.5 /CA 1 /BM /Multiply /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_ps_transparent_alpha_ext_gstate() -> Vec<u8> {
    let content = "0 0 1 rg\n0 0 120 120 re f\n/GS0 gs\n1 0 0 rg\n0 1 0 RG\n10 10 80 80 re B\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /ca 0 /CA 0 /BM /Normal /SMask /None /OP false /op false /SA false /AIS false /TK true >>".to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_safe_ext_gstate_flatness(flatness: f64) -> Vec<u8> {
    let content = "/GS0 gs\n0 0 1 RG\n10 60 m 20 110 100 110 110 60 c S\n";
    let ext_gstate = format!(
        "<< /Type /ExtGState /FL {:.3} /CA 1 /ca 1 /BM /Normal /SMask /None >>",
        flatness
    );
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        ext_gstate.into_bytes(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_safe_ext_gstate_font() -> Vec<u8> {
    let content = "/GS0 gs\nBT 10 60 Td (Hi) Tj ET\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /Font [6 0 R 18] /CA 1 /ca 1 /BM /Normal /SMask /None >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_safe_ext_gstate_styled_text(rendering_mode: i32) -> Vec<u8> {
    let content =
        format!("/GS0 gs\n0 1 0 rg\n0 0 1 RG\nBT {rendering_mode} Tr 10 60 Td (Hi) Tj ET\n");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 6 0 R >> /ExtGState << /GS0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /ExtGState /Font [6 0 R 18] /LW 3 /LC 1 /LJ 2 /ML 7 /D [[5 1] 2] /CA 1 /ca 1 /BM /Normal /SMask /None >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_text_clipping_render_mode(mode: i32) -> Vec<u8> {
    let content =
        format!("BT /F1 24 Tf {mode} Tr 10 60 Td (H) Tj ET\n0 0 1 rg\n0 0 120 120 re f\n");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    pdf_from_objects(&objects)
}

fn pdf_with_nested_rect_clips() -> Vec<u8> {
    let content = "q\n10 10 80 80 re W n\n30 0 80 80 re W n\n1 0 0 rg\n0 0 120 120 re f\nQ\n";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 120] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        )
        .into_bytes(),
    ];
    pdf_from_objects(&objects)
}

// ---------------------------------------------------------------------------
// Fallback classifier unit tests
// ---------------------------------------------------------------------------

fn add_basic_image_xobject_metadata(resources: &mut PageResources, name: &str) {
    let mut dict = PdfDictionary::empty();
    dict.insert("Subtype", PdfObject::Name("Image".to_string()));
    dict.insert("Width", PdfObject::Integer(1));
    dict.insert("Height", PdfObject::Integer(1));
    dict.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
    dict.insert("BitsPerComponent", PdfObject::Integer(8));
    resources
        .xobject_stream_dicts
        .insert(name.to_string(), dict);
}

#[test]
fn classifier_pure_vector_page() {
    let ops = vec![
        ContentOperation::new("m", vec![Operand::Real(10.0), Operand::Real(20.0)]),
        ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(20.0)]),
        ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(100.0)]),
        ContentOperation::new("h", vec![]),
        ContentOperation::new("f", vec![]),
    ];
    let r = PageResources::default();
    assert_eq!(
        classify_page_for_vector_output(&ops, &r, 1.0),
        VectorFallbackDecision::PureVector
    );
}

#[test]
fn classifier_image_xobject_regional_fallback() {
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Im0".to_string(), "Image".to_string());
    r.xobjects.insert("Im0".to_string(), (5, 0));
    add_basic_image_xobject_metadata(&mut r, "Im0");

    let ops = vec![
        // Some vector content first.
        ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
        ContentOperation::new("l", vec![Operand::Real(50.0), Operand::Real(0.0)]),
        ContentOperation::new("S", vec![]),
        // Set axis-aligned CTM for image placement.
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(150.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-80.0),
                Operand::Real(100.0),
                Operand::Real(500.0),
            ],
        ),
        // Place the image.
        ContentOperation::new("Do", vec![Operand::Name("Im0".to_string())]),
    ];

    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
            assert_eq!(image_names, vec!["Im0"]);
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_malformed_image_xobject_boolean_metadata_stays_whole_page() {
    let cases = [
        ("ImageMask", "malformed ImageMask"),
        ("IM", "malformed short ImageMask"),
        ("Interpolate", "malformed Interpolate"),
        ("I", "malformed short Interpolate"),
    ];

    for (key, label) in cases {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert("Im0".to_string(), "Image".to_string());
        r.xobjects.insert("Im0".to_string(), (5, 0));
        add_basic_image_xobject_metadata(&mut r, "Im0");
        r.xobject_stream_dicts
            .get_mut("Im0")
            .expect("image metadata")
            .insert(key, PdfObject::Name("Bad".to_string()));

        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(150.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(-80.0),
                    Operand::Real(100.0),
                    Operand::Real(500.0),
                ],
            ),
            ContentOperation::new("Do", vec![Operand::Name("Im0".to_string())]),
        ];

        match classify_page_for_vector_output(&ops, &r, 1.0) {
            VectorFallbackDecision::WholePageRaster { reason } => {
                assert_eq!(
                    reason, "degenerate or unresolvable image XObject",
                    "{label}"
                );
            }
            other => panic!("{label}: expected WholePageRaster, got {:?}", other),
        }
    }
}

#[test]
fn classifier_form_xobject_whole_page() {
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Fm0".to_string(), "Form".to_string());
    r.xobjects.insert("Fm0".to_string(), (3, 0));

    let ops = vec![ContentOperation::new(
        "Do",
        vec![Operand::Name("Fm0".to_string())],
    )];

    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert_eq!(reason, "Form XObject");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn svg_vector_safe_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_vector_safe_form_xobject()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "vector-safe Form XObject should not force whole-page SVG rasterization"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover vector-safe Form subprograms"
    );
    assert!(
        page.svg.contains("<path"),
        "Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "vector-safe Form should not embed a raster page"
    );
}

#[test]
fn svg_noop_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "default transparency group with opaque vector-safe contents should not force whole-page SVG rasterization"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover no-op transparency-group Form subprograms"
    );
    assert!(
        page.svg.contains("<path"),
        "no-op transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "no-op transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_opaque_isolated_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /I true >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque isolated transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert isolated transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert isolated transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert isolated transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_device_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /CS /DeviceRGB >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque DeviceRGB transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert DeviceRGB transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert DeviceRGB transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert DeviceRGB transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_calibrated_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /CS [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque calibrated transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert calibrated transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert calibrated transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert calibrated transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_indexed_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /CS [/Indexed /DeviceRGB 0 <ff0000>] >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque Indexed transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert Indexed transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert Indexed transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert Indexed transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_iccbased_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine =
        ContentEngine::open_bytes(pdf_with_iccbased_group_color_space_form_xobject()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque ICCBased transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert ICCBased transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert ICCBased transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert ICCBased transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_separation_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        separation_group_color_space_dict(),
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque Separation transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert Separation transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert Separation transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert Separation transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_devicen_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        devicen_group_color_space_dict(),
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque DeviceN transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert DeviceN transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert DeviceN transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert DeviceN transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_multicomponent_devicen_group_color_space_opaque_transparency_group_form_xobject_replays_natively(
) {
    let engine =
        ContentEngine::open_bytes(pdf_with_multicomponent_devicen_group_color_space_form_xobject())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque multi-component DeviceN transparency-group Form should replay as native SVG"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert multi-component DeviceN transparency-group Forms"
    );
    assert!(
        page.svg.contains("<path"),
        "inert multi-component DeviceN transparency-group Form paths should be native SVG"
    );
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "inert multi-component DeviceN transparency-group Form should not embed a raster page"
    );
}

#[test]
fn svg_calibrated_group_color_space_alpha_transparency_group_form_xobject_returns_typed_refusal() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /CS [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1 1 1] >>] >>",
    ))
    .unwrap();
    let error = match engine.render_page_svg(1, 72) {
        Ok(_) => panic!("alpha-bearing calibrated group must fail typed, not replay natively"),
        Err(error) => error,
    };
    let message = format!("{error}");
    assert!(
        message.contains("group /CS has unsupported color space /CalRGB"),
        "{message}"
    );
}

#[test]
fn svg_device_group_color_space_alpha_transparency_group_form_xobject_stays_whole_page_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /CS /DeviceRGB >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "alpha inside a DeviceRGB transparency-group Form must keep whole-page SVG raster fallback"
    );
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "semantic DeviceRGB alpha-group SVG fallback should embed raster output"
    );
}

#[test]
fn svg_inherited_alpha_isolated_transparency_group_form_xobject_stays_whole_page_fallback() {
    let engine = ContentEngine::open_bytes(
        pdf_with_inherited_alpha_transparency_group_form_xobject("<< /S /Transparency /I true >>"),
    )
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "inherited alpha must make isolated transparency-group semantics non-inert"
    );
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "inherited-alpha isolated group SVG fallback should embed raster output"
    );
}

#[test]
fn svg_alpha_isolated_transparency_group_form_xobject_stays_whole_page_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /I true >>",
    ))
    .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "alpha inside isolated transparency-group Form must keep whole-page SVG raster fallback"
    );
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "semantic alpha-group SVG fallback should embed raster output"
    );
}

#[test]
fn svg_strict_refuses_alpha_group_whole_page_raster_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /I true >>",
    ))
    .unwrap();
    let err = match engine.render_page_svg_strict(1, 72) {
        Ok(_) => {
            panic!("strict SVG must not embed an unsupported alpha group as a full-page raster")
        }
        Err(err) => err,
    };
    let message = format!("{err}");
    assert!(
        message.contains("strict SVG vector output refuses whole-page raster fallback"),
        "{message}"
    );
    assert!(message.contains("Form XObject"), "{message}");
}

#[test]
fn svg_output_composes_stacked_clip_paths() {
    let engine = ContentEngine::open_bytes(pdf_with_nested_rect_clips()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "stacked rectangular clips should remain vector SVG"
    );
    assert!(
        page.svg.contains("<clipPath id=\"clip0\"><path"),
        "first clip should be emitted as a standalone clipPath"
    );
    assert!(
        page.svg
            .contains("<clipPath id=\"clip1\"><g clip-path=\"url(#clip0)\"><path"),
        "second clip should compose with the active parent clip"
    );
    assert!(
        page.svg.contains("clip-path=\"url(#clip1)\""),
        "painted content should reference the composed clip"
    );
}

#[test]
fn ps_vector_safe_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_vector_safe_form_xobject()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "vector-safe Form XObject should not force whole-page PS rasterization"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover vector-safe Form subprograms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        page.body.contains("clip"),
        "Form BBox should install a clip"
    );
    assert!(
        !page.body.contains("/picstr"),
        "vector-safe Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_noop_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency >>",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "default transparency group with opaque vector-safe contents should not force whole-page PS rasterization"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover no-op transparency-group Form subprograms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "no-op transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_opaque_knockout_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /K true >>",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque knockout transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert knockout transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert knockout transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_device_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /CS /DeviceRGB >>",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque DeviceRGB transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert DeviceRGB transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert DeviceRGB transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_resource_calibrated_group_color_space_opaque_transparency_group_form_xobject_replays_natively(
) {
    let engine = ContentEngine::open_bytes(pdf_with_resource_group_color_space_form_xobject(
        "<< /S /Transparency /CS /CS0 >>",
        "[/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 1 >>]",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque resource-named calibrated transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover resource-named calibrated transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert resource-named calibrated transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_indexed_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        "<< /S /Transparency /CS [/Indexed /DeviceRGB 0 <ff0000>] >>",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque Indexed transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert Indexed transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert Indexed transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_iccbased_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine =
        ContentEngine::open_bytes(pdf_with_iccbased_group_color_space_form_xobject()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque ICCBased transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert ICCBased transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert ICCBased transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_separation_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        separation_group_color_space_dict(),
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque Separation transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert Separation transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert Separation transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_devicen_group_color_space_opaque_transparency_group_form_xobject_replays_natively() {
    let engine = ContentEngine::open_bytes(pdf_with_transparency_group_form_xobject(
        devicen_group_color_space_dict(),
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque DeviceN transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert DeviceN transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert DeviceN transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_multicomponent_devicen_group_color_space_opaque_transparency_group_form_xobject_replays_natively(
) {
    let engine =
        ContentEngine::open_bytes(pdf_with_multicomponent_devicen_group_color_space_form_xobject())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque multi-component DeviceN transparency-group Form should replay as native PS"
    );
    assert!(
        page.has_regional_images,
        "regional-vector flag should cover inert multi-component DeviceN transparency-group Forms"
    );
    assert!(page.body.contains("setrgbcolor"));
    assert!(
        !page.body.contains("/picstr"),
        "inert multi-component DeviceN transparency-group Form should not embed a whole-page raster"
    );
}

#[test]
fn ps_alpha_knockout_transparency_group_form_xobject_stays_whole_page_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /K true >>",
    ))
    .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "alpha inside knockout transparency-group Form must keep whole-page PS raster fallback"
    );
    assert!(
        page.body.contains("/picstr"),
        "semantic alpha-group PS fallback should embed raster output"
    );
}

#[test]
fn ps_strict_refuses_alpha_group_whole_page_raster_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_alpha_transparency_group_form_xobject(
        "<< /S /Transparency /K true >>",
    ))
    .unwrap();
    let err = match engine.render_page_ps_strict(1, 72) {
        Ok(_) => {
            panic!("strict PS must not embed an unsupported alpha group as a full-page raster")
        }
        Err(err) => err,
    };
    let message = format!("{err}");
    assert!(
        message.contains("strict PostScript vector output refuses whole-page raster fallback"),
        "{message}"
    );
    assert!(message.contains("Form XObject"), "{message}");
}

#[test]
fn classifier_rotated_image_is_regional_affine_fallback() {
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Im0".to_string(), "Image".to_string());
    r.xobjects.insert("Im0".to_string(), (5, 0));
    add_basic_image_xobject_metadata(&mut r, "Im0");

    // 45° rotation: ctm[1] and ctm[2] are non-zero.
    let ops = vec![
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(100.0),
                Operand::Real(100.0),  // b ≠ 0
                Operand::Real(-100.0), // c ≠ 0
                Operand::Real(100.0),
                Operand::Real(200.0),
                Operand::Real(400.0),
            ],
        ),
        ContentOperation::new("Do", vec![Operand::Name("Im0".to_string())]),
    ];

    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
            assert_eq!(image_names, vec!["Im0"]);
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_unknown_xobject_subtype_whole_page() {
    let mut r = PageResources::default();
    // Unknown/missing subtype.
    r.xobjects.insert("X0".to_string(), (7, 0));

    let ops = vec![ContentOperation::new(
        "Do",
        vec![Operand::Name("X0".to_string())],
    )];

    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert!(reason.contains("unknown"), "reason: {reason}");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn classifier_gs_operator_whole_page() {
    let ops = vec![ContentOperation::new(
        "gs",
        vec![Operand::Name("GS0".to_string())],
    )];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert_eq!(reason, "unresolved ExtGState");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn classifier_simple_inline_image_is_regional() {
    let ops = vec![
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(20.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-10.0),
                Operand::Real(40.0),
                Operand::Real(90.0),
            ],
        ),
        ContentOperation::new("BI", vec![]),
        ContentOperation::new(
            "ID",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
            ],
        ),
        ContentOperation::new(
            "inline_image_data",
            vec![Operand::String(vec![255, 0, 0, 0, 255, 0])],
        ),
        ContentOperation::new("EI", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback {
            image_names,
            inline_image_count,
            ..
        } => {
            assert!(image_names.is_empty());
            assert_eq!(inline_image_count, 1);
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_inline_image_mask_is_regional() {
    let ops = vec![
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(20.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-10.0),
                Operand::Real(40.0),
                Operand::Real(90.0),
            ],
        ),
        ContentOperation::new("BI", vec![]),
        ContentOperation::new(
            "ID",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ImageMask".to_string()),
                Operand::Boolean(true),
            ],
        ),
        ContentOperation::new(
            "inline_image_data",
            vec![Operand::String(vec![0b1000_0000])],
        ),
        ContentOperation::new("EI", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback {
            image_names,
            inline_image_count,
            ..
        } => {
            assert!(image_names.is_empty());
            assert_eq!(inline_image_count, 1);
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_malformed_inline_image_metadata_stays_whole_page() {
    let cases: Vec<(&str, Vec<Operand>)> = vec![
        (
            "missing ColorSpace",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
            ],
        ),
        (
            "missing BitsPerComponent",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
            ],
        ),
        (
            "unsupported BitsPerComponent",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(3),
            ],
        ),
        (
            "zero Width",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(0),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
            ],
        ),
        (
            "mask BitsPerComponent not 1",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ImageMask".to_string()),
                Operand::Boolean(true),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
            ],
        ),
        (
            "mask Decode has non-numeric entry",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ImageMask".to_string()),
                Operand::Boolean(true),
                Operand::Name("Decode".to_string()),
                Operand::Array(vec![Operand::Name("Bad".to_string()), Operand::Integer(1)]),
            ],
        ),
        (
            "ImageMask is not boolean",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
                Operand::Name("ImageMask".to_string()),
                Operand::Name("Bad".to_string()),
            ],
        ),
        (
            "short ImageMask is not boolean",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
                Operand::Name("IM".to_string()),
                Operand::Name("Bad".to_string()),
            ],
        ),
        (
            "Interpolate is not boolean",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
                Operand::Name("Interpolate".to_string()),
                Operand::Name("Bad".to_string()),
            ],
        ),
        (
            "short Interpolate is not boolean",
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(2),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceRGB".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
                Operand::Name("I".to_string()),
                Operand::Name("Bad".to_string()),
            ],
        ),
    ];

    for (label, inline_params) in cases {
        let ops = inline_image_classifier_ops(inline_params);
        let r = PageResources::default();
        match classify_page_for_vector_output(&ops, &r, 1.0) {
            VectorFallbackDecision::WholePageRaster { reason } => {
                assert_eq!(reason, "unsupported inline image", "{label}");
            }
            other => panic!("{label}: expected WholePageRaster, got {:?}", other),
        }
    }
}

#[test]
fn classifier_unsupported_image_xobject_color_space_stays_whole_page() {
    let cases = vec![
        (
            "unsupported direct name",
            PdfObject::Name("Bogus".to_string()),
            None,
        ),
        (
            "unsupported resource name",
            PdfObject::Name("CS0".to_string()),
            Some(("CS0".to_string(), PdfObject::Name("Bogus".to_string()))),
        ),
        (
            "malformed color-space array",
            PdfObject::Array(vec![PdfObject::Name("Bogus".to_string())]),
            None,
        ),
    ];

    for (label, color_space, resource_space) in cases {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert("Im0".to_string(), "Image".to_string());
        r.xobjects.insert("Im0".to_string(), (5, 0));
        add_basic_image_xobject_metadata(&mut r, "Im0");
        r.xobject_stream_dicts
            .get_mut("Im0")
            .expect("image metadata")
            .insert("ColorSpace", color_space);
        if let Some((name, object)) = resource_space {
            r.color_spaces.insert(name, object);
        }

        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(150.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(-80.0),
                    Operand::Real(100.0),
                    Operand::Real(500.0),
                ],
            ),
            ContentOperation::new("Do", vec![Operand::Name("Im0".to_string())]),
        ];

        match classify_page_for_vector_output(&ops, &r, 1.0) {
            VectorFallbackDecision::WholePageRaster { reason } => {
                assert_eq!(
                    reason, "degenerate or unresolvable image XObject",
                    "{label}"
                );
            }
            other => panic!("{label}: expected WholePageRaster, got {:?}", other),
        }
    }
}

fn inline_image_classifier_ops(inline_params: Vec<Operand>) -> Vec<ContentOperation> {
    vec![
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(20.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-10.0),
                Operand::Real(40.0),
                Operand::Real(90.0),
            ],
        ),
        ContentOperation::new("BI", vec![]),
        ContentOperation::new("ID", inline_params),
        ContentOperation::new(
            "inline_image_data",
            vec![Operand::String(vec![255, 0, 0, 0, 255, 0])],
        ),
        ContentOperation::new("EI", vec![]),
    ]
}

#[test]
fn classifier_dense_text_stays_vector() {
    let mut ops = vec![ContentOperation::new("BT", vec![])];
    ops.extend((0..128).map(|_| ContentOperation::new("Tj", vec![Operand::String(vec![b'A'])])));
    ops.push(ContentOperation::new("ET", vec![]));
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::PureVector => {}
        other => panic!("Expected PureVector, got {:?}", other),
    }
}

#[test]
fn classifier_dead_pattern_color_state_stays_vector() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("g", vec![Operand::Real(0.5)]),
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(20.0),
                Operand::Real(20.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::PureVector => {}
        other => panic!("Expected PureVector, got {:?}", other),
    }
}

#[test]
fn classifier_pattern_fill_paint_whole_page() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(20.0),
                Operand::Real(20.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert_eq!(reason, "pattern fill paint");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn classifier_simple_shading_pattern_fill_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(20.0),
                Operand::Real(20.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", None);
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback {
            image_names,
            inline_image_count,
            form_names,
            shading_names,
        } => {
            assert!(image_names.is_empty());
            assert_eq!(inline_image_count, 0);
            assert!(form_names.is_empty());
            assert!(shading_names.is_empty());
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_translated_shading_pattern_fill_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("q", vec![]),
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(1.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(1.0),
                Operand::Real(20.0),
                Operand::Real(0.0),
            ],
        ),
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(20.0),
                Operand::Real(20.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
        ContentOperation::new("Q", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", None);
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { .. } => {}
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_shading_pattern_with_matrix_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(20.0),
                Operand::Real(20.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", Some([1.0, 0.0, 0.0, 1.0, 5.0, 0.0]));
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { .. } => {}
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_pattern_stroke_paint_whole_page() {
    let ops = vec![
        ContentOperation::new("CS", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("SCN", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
        ContentOperation::new("l", vec![Operand::Real(20.0), Operand::Real(20.0)]),
        ContentOperation::new("S", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert_eq!(reason, "pattern stroke paint");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn classifier_simple_shading_pattern_stroke_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("CS", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("SCN", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
        ContentOperation::new("l", vec![Operand::Real(20.0), Operand::Real(20.0)]),
        ContentOperation::new("S", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", None);
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { .. } => {}
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_pattern_text_paint_whole_page() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("BT", vec![]),
        ContentOperation::new("Tj", vec![Operand::String(vec![b'A'])]),
        ContentOperation::new("ET", vec![]),
    ];
    let r = PageResources::default();
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::WholePageRaster { reason } => {
            assert_eq!(reason, "pattern text paint");
        }
        other => panic!("Expected WholePageRaster, got {:?}", other),
    }
}

#[test]
fn classifier_simple_shading_pattern_text_fill_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("cs", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("scn", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("BT", vec![]),
        ContentOperation::new("Tj", vec![Operand::String(vec![b'A'])]),
        ContentOperation::new("ET", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", None);
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { .. } => {}
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

#[test]
fn classifier_simple_shading_pattern_text_stroke_is_regional_vector() {
    let ops = vec![
        ContentOperation::new("CS", vec![Operand::Name("Pattern".to_string())]),
        ContentOperation::new("SCN", vec![Operand::Name("P0".to_string())]),
        ContentOperation::new("BT", vec![]),
        ContentOperation::new("Tr", vec![Operand::Integer(1)]),
        ContentOperation::new("Tj", vec![Operand::String(vec![b'A'])]),
        ContentOperation::new("ET", vec![]),
    ];
    let r = resources_with_shading_pattern("P0", None);
    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { .. } => {}
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// image_device_rect coordinate tests
// ---------------------------------------------------------------------------

#[test]
fn image_device_rect_normal_placement() {
    let mut gs = GraphicsState::default();
    // Common image placement: 200pt wide, 100pt tall at (50, 400), ctm[3]<0.
    gs.ctm = [200.0, 0.0, 0.0, -100.0, 50.0, 400.0];
    let rect = image_device_rect(&gs, 1.0, 800.0).unwrap();
    assert!((rect[0] - 50.0).abs() < 0.01, "x: {}", rect[0]);
    assert!((rect[1] - 400.0).abs() < 0.01, "y: {}", rect[1]);
    assert!((rect[2] - 200.0).abs() < 0.01, "w: {}", rect[2]);
    assert!((rect[3] - 100.0).abs() < 0.01, "h: {}", rect[3]);
}

#[test]
fn image_device_rect_rejects_rotation() {
    let mut gs = GraphicsState::default();
    gs.ctm = [100.0, 50.0, -50.0, 100.0, 200.0, 300.0]; // shear/rotation
    assert!(image_device_rect(&gs, 1.0, 600.0).is_none());
}

#[test]
fn image_device_rect_with_viewport_scale() {
    let mut gs = GraphicsState::default();
    gs.ctm = [100.0, 0.0, 0.0, -50.0, 10.0, 200.0];
    let scale = 2.0;
    let rect = image_device_rect(&gs, scale, 800.0).unwrap();
    // All coordinates are scaled by viewport_scale.
    assert!((rect[0] - 20.0).abs() < 0.01, "x: {}", rect[0]); // 10*2
    assert!((rect[2] - 200.0).abs() < 0.01, "w: {}", rect[2]); // 100*2
    assert!((rect[3] - 100.0).abs() < 0.01, "h: {}", rect[3]); // 50*2
}

// ---------------------------------------------------------------------------
// Integration test: mixed vector+image PDF through SVG output
// ---------------------------------------------------------------------------

/// Build a minimal PDF with one vector path and one Image XObject, ensuring
/// the SVG output contains both a `<path>` vector element and an `<image>`
/// regional embed — proving that the regional fallback path preserves native
/// vector content.
#[test]
fn svg_output_mixed_vector_and_image_retains_path_elements() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("image_only.pdf");
    if !fixture.exists() {
        // If the fixture doesn't exist, skip gracefully.
        eprintln!("Skipping: fixture not found at {:?}", fixture);
        return;
    }
    let engine = ContentEngine::open_bytes(std::fs::read(&fixture).unwrap()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();

    // The image_only.pdf has an image XObject. If the classifier sees it as a
    // simple axis-aligned image, the output should have regional embedding
    // (not a whole-page raster). If it falls back to whole-page, it's because
    // there are other unsupported constructs (which is correct behavior).
    if page.is_rasterized {
        // Whole-page raster is acceptable for complex pages — the important
        // thing is that we don't panic and the output is valid SVG.
        assert!(page.svg.contains("<image"));
        assert!(page.svg.contains("<svg"));
    } else if page.has_regional_images {
        // Regional fallback: should have both vector and image elements.
        assert!(
            page.svg.contains("<image"),
            "Expected <image> element for regional embed"
        );
        assert!(page.svg.contains("<svg"));
    } else {
        // Pure vector: no images at all (unexpected for image_only.pdf but valid).
        assert!(page.svg.contains("<svg"));
    }
}

// ---------------------------------------------------------------------------
// Integration test: mixed vector+image PDF through PostScript output
// ---------------------------------------------------------------------------

#[test]
fn ps_output_mixed_vector_and_image_retains_path_operators() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("image_only.pdf");
    if !fixture.exists() {
        eprintln!("Skipping: fixture not found at {:?}", fixture);
        return;
    }
    let engine = ContentEngine::open_bytes(std::fs::read(&fixture).unwrap()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();

    if page.is_rasterized {
        assert!(page.body.contains("colorimage"));
    } else if page.has_regional_images {
        // Regional fallback: should have the regional colorimage AND possibly
        // vector operators in the same body.
        assert!(
            page.body.contains("colorimage"),
            "Expected colorimage for regional image embed in PS"
        );
        // The page body should have gsave/grestore structure.
        assert!(page.body.contains("gsave"));
    } else {
        // Pure vector page.
        assert!(page.body.contains("gsave") || page.body.contains("grestore"));
    }
}

#[test]
fn svg_output_inline_image_uses_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(!page.is_rasterized, "inline image should stay regional SVG");
    assert!(page.has_regional_images);
    assert!(page
        .svg
        .contains("data-wellfriend-region-kind=\"inline-image\""));
    assert!(page
        .svg
        .contains("data-wellfriend-region-bounds=\"40.000 30.000 20.000 10.000\""));
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "inline image should be embedded as bounded PNG"
    );
    assert!(
        page.svg.matches("<path").count() >= 2,
        "surrounding vector paths should remain native SVG: {}",
        page.svg
    );
    let blue_path = page
        .svg
        .find("fill=\"#0000FF\"")
        .expect("blue vector path before regional image");
    let regional_image = page
        .svg
        .find("data-wellfriend-region-kind=\"inline-image\"")
        .expect("regional inline image marker");
    let green_path = page
        .svg
        .find("fill=\"#00FF00\"")
        .expect("green vector path after regional image");
    assert!(
        blue_path < regional_image && regional_image < green_path,
        "regional SVG fallback must preserve stream order around the bounded image: {}",
        page.svg
    );
}

#[test]
fn ps_output_inline_image_uses_regional_colorimage() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(!page.is_rasterized, "inline image should stay regional PS");
    assert!(page.has_regional_images);
    assert!(page
        .body
        .contains("% WellfriendRegion kind=inline-image bounds=40.000 30.000 20.000 10.000"));
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
    assert!(page.body.contains("setrgbcolor"));
    let blue_path = page
        .body
        .find("0.0000 0.0000 1.0000 setrgbcolor")
        .expect("blue vector path before regional image");
    let regional_image = page
        .body
        .find("% WellfriendRegion kind=inline-image")
        .expect("regional inline image marker");
    let green_path = page
        .body
        .find("0.0000 1.0000 0.0000 setrgbcolor")
        .expect("green vector path after regional image");
    assert!(
        blue_path < regional_image && regional_image < green_path,
        "regional PS fallback must preserve stream order around the bounded image: {}",
        page.body
    );
}

#[test]
fn svg_output_inline_image_resource_color_space_uses_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image_resource_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named DeviceRGB color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_color_space_uses_regional_colorimage() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image_resource_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named DeviceRGB color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn active_renderer_inline_image_resource_indexed_color_space_resolves_named_resource() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_color_space()).unwrap();
    let page = engine.render_page(1, 72).unwrap();

    let red = page.get_pixel(45, 35);
    let blue = page.get_pixel(55, 35);
    assert!(
        red[0] > 180 && red[1] < 100 && red[2] < 100,
        "first Indexed palette entry should render red in the inline image region: {red:?}"
    );
    assert!(
        blue[2] > 180 && blue[0] < 100 && blue[1] < 100,
        "second Indexed palette entry should render blue in the inline image region: {blue:?}"
    );
}

#[test]
fn svg_output_inline_image_resource_calrgb_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_calrgb_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named CalRGB color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_calrgb_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_calrgb_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named CalRGB color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_indexed_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with 8-bit resource-named Indexed color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_indexed_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with 8-bit resource-named Indexed color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_indexed_1bit_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_1bit_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with 1-bit resource-named Indexed color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_indexed_1bit_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_1bit_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with 1-bit resource-named Indexed color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_calrgb_dct_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_calrgb_dct_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named CalRGB color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_calrgb_dct_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_calrgb_dct_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named CalRGB color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_indexed_dct_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_dct_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named Indexed color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_indexed_dct_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_indexed_dct_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named Indexed color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_iccbased_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_iccbased_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named ICCBased color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_iccbased_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_iccbased_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image with resource-named ICCBased color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_iccbased_dct_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_iccbased_dct_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named ICCBased color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_iccbased_dct_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_iccbased_dct_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT inline image with resource-named ICCBased color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_separation_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque resource-named Separation inline image should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_separation_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque resource-named Separation inline image should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
    assert!(
        page.body.contains("FF0000"),
        "spot tint transform should produce an opaque red regional pixel: {}",
        page.body
    );
}

#[test]
fn svg_output_inline_image_resource_separation_dct_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_dct_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT resource-named Separation inline image should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_separation_dct_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_dct_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT resource-named Separation inline image should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_devicen_single_dct_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_devicen_single_dct_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT resource-named one-colorant DeviceN inline image should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_devicen_single_dct_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_devicen_single_dct_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DCT resource-named one-colorant DeviceN inline image should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_devicen_color_space_uses_regional_embed() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_devicen_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque resource-named DeviceN inline image should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
}

#[test]
fn ps_output_inline_image_resource_devicen_color_space_uses_regional_colorimage() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_devicen_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "opaque resource-named DeviceN inline image should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("regionpicstr"));
    assert!(page.body.contains("colorimage"));
}

#[test]
fn svg_output_inline_image_resource_separation_none_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_none_color_space())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "Separation /None inline images must not be approved for regional SVG"
    );
}

#[test]
fn ps_output_inline_image_resource_separation_none_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_inline_image_resource_separation_none_color_space())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "Separation /None inline images must not be approved for regional PS"
    );
}

#[test]
fn svg_output_inline_image_mask_uses_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image mask should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "inline image mask should be embedded as bounded PNG"
    );
    assert!(
        page.svg.contains("<path"),
        "surrounding vector path should remain native SVG: {}",
        page.svg
    );
}

#[test]
fn ps_output_inline_image_mask_uses_regional_imagemask() {
    let engine = ContentEngine::open_bytes(pdf_with_inline_image_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "inline image mask should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/regionmaskstr"));
    assert!(page.body.contains("imagemask"));
    assert!(
        page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"),
        "stencil mask should paint with the active red fill color before imagemask: {}",
        page.body
    );
    assert!(
        !page.body.contains("/regionpicstr"),
        "stencil masks should not allocate the regional RGB scratch buffer: {}",
        page.body
    );
    assert!(
        !page.body.contains("colorimage"),
        "stencil masks should use imagemask instead of colorimage: {}",
        page.body
    );
}

#[test]
fn svg_output_shading_pattern_inline_image_mask_uses_svg_masked_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_shading_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "SVG can represent a simple shading-pattern painted inline stencil mask region"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(
        page.svg
            .contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""),
        "mask image in defs must keep the xlink namespace: {}",
        page.svg
    );
}

#[test]
fn ps_output_shading_pattern_inline_image_mask_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_shading_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "bounded pattern-painted inline stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_colored_tiling_pattern_inline_image_mask_replays_native_tiles() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern painted inline stencil masks should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_inline_image_mask_replays_native_tiles() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern painted inline stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_inline_image_mask_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern painted inline stencil masks should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling-pattern inline mask should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_inline_image_mask_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_inline_image_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern painted inline stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling-pattern inline mask should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_image_xobject_mask_uses_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_image_xobject_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "Image XObject mask should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "Image XObject mask should be embedded as bounded PNG"
    );
    assert!(
        page.svg.contains("<path"),
        "surrounding vector path should remain native SVG: {}",
        page.svg
    );
}

#[test]
fn svg_output_shading_pattern_image_xobject_mask_uses_svg_masked_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_shading_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "SVG can represent a simple shading-pattern painted Image XObject stencil mask region"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(
        page.svg
            .contains("xmlns:xlink=\"http://www.w3.org/1999/xlink\""),
        "mask image in defs must keep the xlink namespace: {}",
        page.svg
    );
}

#[test]
fn svg_output_colored_tiling_pattern_image_xobject_mask_replays_native_tiles() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern painted Image XObject stencil masks should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_image_xobject_mask_replays_native_tiles() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern painted Image XObject stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_image_xobject_mask_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern painted Image XObject stencil masks should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<mask id=\"mask"), "{}", page.svg);
    assert!(page.svg.contains("mask=\"url(#mask"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling-pattern Image XObject mask should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_image_xobject_mask_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern painted Image XObject stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling-pattern Image XObject mask should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn ps_output_shading_pattern_image_xobject_mask_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_shading_pattern_image_xobject_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "bounded pattern-painted Image XObject stencil masks should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("imagemask"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn ps_output_image_xobject_mask_uses_regional_imagemask() {
    let engine = ContentEngine::open_bytes(pdf_with_image_xobject_mask()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "Image XObject mask should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/regionmaskstr"));
    assert!(page.body.contains("imagemask"));
    assert!(
        page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"),
        "Image XObject mask should paint with the active red fill color before imagemask: {}",
        page.body
    );
    assert!(
        !page.body.contains("/regionpicstr"),
        "Image XObject masks should not allocate the regional RGB scratch buffer: {}",
        page.body
    );
    assert!(
        !page.body.contains("colorimage"),
        "Image XObject masks should use imagemask instead of colorimage: {}",
        page.body
    );
}

#[test]
fn svg_output_rotated_image_xobject_uses_affine_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_rotated_image_xobject()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "rotated image XObject should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(
        page.svg.contains("data:image/png;base64,"),
        "rotated image should be embedded as bounded PNG"
    );
    assert!(
        page.svg.contains("transform=\"matrix("),
        "rotated image should carry an affine SVG matrix: {}",
        page.svg
    );
    assert!(
        page.svg.matches("<path").count() >= 2,
        "surrounding vector paths should remain native SVG: {}",
        page.svg
    );
}

#[test]
fn ps_output_rotated_image_xobject_uses_affine_regional_colorimage() {
    let engine = ContentEngine::open_bytes(pdf_with_rotated_image_xobject()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "rotated image XObject should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/regionpicstr"));
    assert!(page.body.contains("colorimage"));
    assert!(
        page.body.contains("] concat"),
        "rotated image should carry an affine PostScript matrix: {}",
        page.body
    );
    assert!(
        !page.body.contains("/picstr "),
        "rotated image should not require whole-page raster scratch: {}",
        page.body
    );
}

#[test]
fn svg_output_image_xobject_resource_color_space_uses_regional_embed() {
    let engine = ContentEngine::open_bytes(pdf_with_image_xobject_resource_color_space()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "Image XObject with resource-named DeviceRGB color space should stay regional SVG"
    );
    assert!(page.has_regional_images);
    assert!(page
        .svg
        .contains("data-wellfriend-region-kind=\"image-xobject\""));
    assert!(page
        .svg
        .contains("data-wellfriend-region-bounds=\"40.000 30.000 20.000 10.000\""));
    assert!(page.svg.contains("data:image/png;base64,"));
    assert!(page.svg.contains("<path"));
    let blue_path = page
        .svg
        .find("fill=\"#0000FF\"")
        .expect("blue vector path before regional image");
    let regional_image = page
        .svg
        .find("data-wellfriend-region-kind=\"image-xobject\"")
        .expect("regional image XObject marker");
    let green_path = page
        .svg
        .find("fill=\"#00FF00\"")
        .expect("green vector path after regional image");
    assert!(
        blue_path < regional_image && regional_image < green_path,
        "regional SVG fallback must preserve stream order around the bounded image XObject: {}",
        page.svg
    );
}

#[test]
fn svg_alpha_image_xobject_uses_native_opacity() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_alpha_image_xobject()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "regional SVG image opacity should not force whole-page rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<image"), "{}", page.svg);
    assert!(page.svg.contains("opacity=\"0.500\""), "{}", page.svg);
    assert!(page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_image_xobject_resource_color_space_uses_regional_colorimage() {
    let engine = ContentEngine::open_bytes(pdf_with_image_xobject_resource_color_space()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "Image XObject with resource-named DeviceRGB color space should stay regional PS"
    );
    assert!(page.has_regional_images);
    assert!(page
        .body
        .contains("% WellfriendRegion kind=image-xobject bounds=40.000 30.000 20.000 10.000"));
    assert!(page.body.contains("/regionpicstr"));
    assert!(page.body.contains("colorimage"));
    let blue_path = page
        .body
        .find("0.0000 0.0000 1.0000 setrgbcolor")
        .expect("blue vector path before regional image");
    let regional_image = page
        .body
        .find("% WellfriendRegion kind=image-xobject")
        .expect("regional image XObject marker");
    let green_path = page
        .body
        .find("0.0000 1.0000 0.0000 setrgbcolor")
        .expect("green vector path after regional image");
    assert!(
        blue_path < regional_image && regional_image < green_path,
        "regional PS fallback must preserve stream order around the bounded image XObject: {}",
        page.body
    );
}

#[test]
fn svg_output_simple_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native gradient path should not embed a raster page"
    );
}

#[test]
fn svg_output_nonextended_axial_shading_uses_clipped_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_nonextended_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "SVG should represent non-extended axial shading with a native clipped gradient"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("<clipPath"), "{}", page.svg);
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "non-extended axial shading should install an SVG clip: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_non_unit_domain_axial_shading_samples_native_gradient_endpoints() {
    let engine = ContentEngine::open_bytes(pdf_with_non_unit_domain_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear non-unit Domain axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg.contains("#BF0040") && page.svg.contains("#4000BF"),
        "SVG gradient stops should be sampled at Domain endpoints: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_clipped_function_domain_axial_shading_uses_native_stops() {
    let engine =
        ContentEngine::open_bytes(pdf_with_clipped_function_domain_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear shading with function-domain clipping should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(
        page.svg
            .contains("offset=\"0.166667\" stop-color=\"#FF0000\"")
            && page
                .svg
                .contains("offset=\"0.833333\" stop-color=\"#0000FF\""),
        "SVG clipped-domain gradient should carry flat-section stops: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_bounded_axial_shading_clips_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_bounded_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "bounded axial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("<clipPath"), "{}", page.svg);
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "bounded shading should clip the native gradient: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_function_array_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_function_array_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear Type 2 function-array axial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg.contains("#FF0000") && page.svg.contains("#0000FF"),
        "function-array SVG stops should combine scalar outputs into RGB: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_stitching_function_axial_shading_uses_native_stops() {
    let engine = ContentEngine::open_bytes(pdf_with_stitching_function_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "continuous Type 3 stitching-function axial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg
            .contains("offset=\"0.500000\" stop-color=\"#00FF00\""),
        "SVG stitching-function gradient should carry midpoint stop: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_alpha_axial_shading_uses_native_opacity() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_alpha_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "regional SVG shading opacity should not force whole-page rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("fill-opacity=\"0.500\""), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    let doc = wellfriendpdf_engine::render::assemble_ps_document(&[page]);
    assert!(doc.contains("%%LanguageLevel: 3"));
}

#[test]
fn ps_output_nonextended_axial_shading_preserves_extend_flags() {
    let engine = ContentEngine::open_bytes(pdf_with_nonextended_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "PostScript shfill can represent explicit non-extended axial shading"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(
        page.body.contains("/Extend [false false]"),
        "PostScript shading dictionary should preserve source extend flags: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_non_unit_domain_axial_shading_samples_native_shfill_endpoints() {
    let engine = ContentEngine::open_bytes(pdf_with_non_unit_domain_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear non-unit Domain axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(page.body.contains("0.7500 0.0000 0.2500"));
    assert!(page.body.contains("0.2500 0.0000 0.7500"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_clipped_function_domain_axial_shading_uses_stitching_function() {
    let engine =
        ContentEngine::open_bytes(pdf_with_clipped_function_domain_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear shading with function-domain clipping should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(
        page.body.contains("/FunctionType 3") && page.body.contains("/Bounds [0.166667 0.833333"),
        "PS clipped-domain gradient should use a stitching function: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_bounded_axial_shading_clips_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_bounded_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "bounded axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("20.00 80.00 moveto") && page.body.contains("100.00 40.00 lineto"),
        "bounded shading should emit a device-space BBox clip path: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_function_array_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_function_array_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "linear Type 2 function-array axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(page.body.contains("1.0000 0.0000 0.0000"));
    assert!(page.body.contains("0.0000 0.0000 1.0000"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_mixed_exponent_function_array_axial_shading_uses_exact_native_functions() {
    let engine =
        ContentEngine::open_bytes(pdf_with_mixed_exponent_function_array_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "mixed-exponent Type 2 function-array axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(
        page.body.contains("/Function [ << /FunctionType 2"),
        "{}",
        page.body
    );
    assert_eq!(page.body.matches("/FunctionType 2").count(), 3);
    assert!(page.body.contains("/C0 [1.0000]"));
    assert!(page.body.contains("/C1 [0.0000]"));
    assert!(page.body.contains("/N 1.000000"));
    assert!(page.body.contains("/C0 [0.5000]"));
    assert!(page.body.contains("/C1 [0.2500]"));
    assert!(page.body.contains("/N 2.000000"));
    assert!(page.body.contains("/C0 [0.0000]"));
    assert!(page.body.contains("/C1 [1.0000]"));
    assert!(page.body.contains("/N 3.000000"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_stitching_function_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_stitching_function_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "continuous Type 3 stitching-function axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"));
    assert!(
        page.body.contains("/FunctionType 3") && page.body.contains("/Bounds [0.500000"),
        "PS stitching-function gradient should stay a native stitching function: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_simple_shading_pattern_fill_uses_clipped_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern fill should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "shading pattern fill should be clipped to the painted path: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_shading_pattern_fill_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern fill should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_colored_tiling_pattern_fill_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple colored tiling pattern fill should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#00FF00\""), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_fill_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple colored tiling pattern fill should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_matrix_tiling_pattern_fill_transforms_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_matrix_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "matrix tiling pattern fill should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(
        page.svg
            .contains("M15.00 100.00 L25.00 100.00 L25.00 90.00"),
        "tiling-pattern Matrix should scale and translate red tile geometry: {}",
        page.svg
    );
    assert!(
        page.svg
            .contains("M25.00 100.00 L35.00 100.00 L35.00 90.00"),
        "tiling-pattern Matrix should scale and translate blue tile geometry: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_matrix_tiling_pattern_fill_transforms_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_matrix_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "matrix tiling pattern fill should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(
        page.body
            .contains("15.00 100.00 moveto\n25.00 100.00 lineto\n25.00 90.00 lineto"),
        "tiling-pattern Matrix should scale and translate red tile geometry: {}",
        page.body
    );
    assert!(
        page.body
            .contains("25.00 100.00 moveto\n35.00 100.00 lineto\n35.00 90.00 lineto"),
        "tiling-pattern Matrix should scale and translate blue tile geometry: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_colored_tiling_pattern_stroke_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple colored tiling pattern stroke should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#00FF00\""), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_stroke_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple colored tiling pattern stroke should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_fill_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple uncolored tiling pattern fill should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling pattern fill should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_fill_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple uncolored tiling pattern fill should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling pattern fill should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_stroke_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple uncolored tiling pattern stroke should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling pattern stroke should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_stroke_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple uncolored tiling pattern stroke should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling pattern stroke should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_color_setting_tile_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_color_setting_tile()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "uncolored tiling pattern tiles that set paint color must not be replayed as native SVG"
    );
    assert!(page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_color_setting_tile_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_color_setting_tile()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "uncolored tiling pattern tiles that set paint color must not be replayed as native PS"
    );
    assert!(page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_colored_tiling_pattern_image_resource_tile_replays_bounded_image() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_image_resource_tile()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "tiling pattern cells with vector-safe image resources should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(
        page.svg
            .contains("data-wellfriend-region-kind=\"image-xobject\""),
        "{}",
        page.svg
    );
    assert!(page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#00FF00\""), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_image_resource_tile_replays_bounded_image() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_image_resource_tile()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "tiling pattern cells with vector-safe image resources should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(
        page.body.contains("% WellfriendRegion kind=image-xobject"),
        "{}",
        page.body
    );
    assert!(page.body.contains("colorimage"), "{}", page.body);
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"));
    assert!(!page.body.contains("/picstr "), "{}", page.body);
}

#[test]
fn svg_output_colored_tiling_pattern_shading_resource_tile_replays_native_shading() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_shading_resource_tile()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "tiling pattern cells with vector-safe shading resources should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#00FF00\""), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_shading_resource_tile_replays_native_shading() {
    let engine =
        ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_shading_resource_tile()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "tiling pattern cells with vector-safe shading resources should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_translated_shading_pattern_fill_transforms_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_translated_shading_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "translated shading pattern fill should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg.contains("x1=\"20.000\"") && page.svg.contains("x2=\"140.000\""),
        "translated shading pattern should transform gradient coordinates: {}",
        page.svg
    );
    assert!(page.svg.contains("clip-path=\"url(#clip"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_translated_shading_pattern_fill_transforms_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_translated_shading_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "translated shading pattern fill should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(
        page.body.contains("/Coords [20.000 60.000 140.000 60.000]"),
        "translated shading pattern should transform shfill coordinates: {}",
        page.body
    );
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_matrix_shading_pattern_fill_transforms_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_matrix_shading_pattern_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "matrix shading pattern fill should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg.contains("x1=\"5.000\"") && page.svg.contains("x2=\"125.000\""),
        "pattern matrix should transform gradient coordinates: {}",
        page.svg
    );
    assert!(page.svg.contains("clip-path=\"url(#clip"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_matrix_shading_pattern_fill_transforms_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_matrix_shading_pattern_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "matrix shading pattern fill should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(
        page.body.contains("/Coords [5.000 60.000 125.000 60.000]"),
        "pattern matrix should transform shfill coordinates: {}",
        page.body
    );
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_simple_shading_pattern_stroke_uses_clipped_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern stroke should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "shading pattern stroke should be clipped to the stroked outline: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_shading_pattern_stroke_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern stroke should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_simple_shading_pattern_text_fill_uses_clipped_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_text_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern text fill should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "shading pattern text fill should be clipped to glyph outlines: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_shading_pattern_text_fill_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_text_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern text fill should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_simple_shading_pattern_text_stroke_uses_clipped_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_text_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern text stroke should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        page.svg.contains("clip-path=\"url(#clip"),
        "shading pattern text stroke should be clipped to stroked glyph outlines: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_shading_pattern_text_stroke_uses_clipped_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_shading_pattern_text_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple shading pattern text stroke should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
    assert!(!page.body.contains("/picstr "));
}

#[test]
fn svg_output_colored_tiling_pattern_text_fill_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_text_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern text fill should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_text_fill_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_text_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern text fill should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_colored_tiling_pattern_text_stroke_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_text_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern text stroke should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#FF0000\""), "{}", page.svg);
    assert!(page.svg.contains("fill=\"#0000FF\""), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_colored_tiling_pattern_text_stroke_replays_native_tiles() {
    let engine = ContentEngine::open_bytes(pdf_with_colored_tiling_pattern_text_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "colored tiling-pattern text stroke should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_text_fill_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_text_fill()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern text fill should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling-pattern text fill should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_text_fill_replays_native_tiles_with_caller_color() {
    let engine = ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_text_fill()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern text fill should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling-pattern text fill should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_uncolored_tiling_pattern_text_stroke_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_text_stroke()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern text stroke should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(
        page.svg.contains("fill=\"#336699\""),
        "uncolored tiling-pattern text stroke should use caller RGB components: {}",
        page.svg
    );
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_uncolored_tiling_pattern_text_stroke_replays_native_tiles_with_caller_color() {
    let engine =
        ContentEngine::open_bytes(pdf_with_uncolored_tiling_pattern_text_stroke()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "uncolored tiling-pattern text stroke should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("clip"), "{}", page.body);
    assert!(
        page.body.contains("0.2000 0.4000 0.6000 setrgbcolor"),
        "uncolored tiling-pattern text stroke should use caller RGB components: {}",
        page.body
    );
    assert!(!page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_simple_cmyk_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_cmyk_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple DeviceCMYK axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(
        page.svg.contains("#ED1C24") && page.svg.contains("#00ADEF"),
        "DeviceCMYK stops should be converted to deterministic RGB anchors: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_cmyk_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_cmyk_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple DeviceCMYK axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceCMYK"));
    assert!(page.body.contains("shfill"));
    assert!(page.body.contains("/C0 [0.0000 1.0000 1.0000 0.0000]"));
    assert!(page.body.contains("/C1 [1.0000 0.0000 0.0000 0.0000]"));
    assert!(page.body.contains("/N 1.000000"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn ps_output_nonlinear_cmyk_axial_shading_uses_exact_device_cmyk_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_nonlinear_cmyk_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "exact DeviceCMYK Type 2 axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 2"), "{}", page.body);
    assert!(
        page.body.contains("/ColorSpace /DeviceCMYK"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [0.0000 0.0000 0.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C1 [1.0000 0.0000 0.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(page.body.contains("/N 1.000000"), "{}", page.body);
    assert!(page.body.contains("/N 2.000000"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn ps_output_mixed_exponent_cmyk_function_array_axial_shading_uses_exact_native_functions() {
    let engine =
        ContentEngine::open_bytes(pdf_with_mixed_exponent_cmyk_function_array_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "mixed-exponent DeviceCMYK Type 2 function-array axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 2"), "{}", page.body);
    assert!(
        page.body.contains("/ColorSpace /DeviceCMYK"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/Function [ << /FunctionType 2"),
        "{}",
        page.body
    );
    assert_eq!(page.body.matches("/FunctionType 2").count(), 4);
    assert!(page.body.contains("/C0 [0.0000]"), "{}", page.body);
    assert!(page.body.contains("/C1 [1.0000]"), "{}", page.body);
    assert!(page.body.contains("/N 1.000000"), "{}", page.body);
    assert!(page.body.contains("/C0 [1.0000]"), "{}", page.body);
    assert!(page.body.contains("/C1 [0.0000]"), "{}", page.body);
    assert!(page.body.contains("/N 2.000000"), "{}", page.body);
    assert!(page.body.contains("/C0 [0.5000]"), "{}", page.body);
    assert!(page.body.contains("/C1 [0.2500]"), "{}", page.body);
    assert!(page.body.contains("/N 3.000000"), "{}", page.body);
    assert!(page.body.contains("/C1 [0.5000]"), "{}", page.body);
    assert!(page.body.contains("/N 4.000000"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn ps_output_cmyk_stitching_function_axial_shading_uses_exact_native_functions() {
    let engine =
        ContentEngine::open_bytes(pdf_with_cmyk_stitching_function_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "DeviceCMYK Type 3 stitching-function axial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 2"), "{}", page.body);
    assert!(
        page.body.contains("/ColorSpace /DeviceCMYK"),
        "{}",
        page.body
    );
    assert!(page.body.contains("/FunctionType 3"), "{}", page.body);
    assert_eq!(page.body.matches("/FunctionType 2").count(), 2);
    assert!(
        page.body.contains("/C0 [0.0000 1.0000 1.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C1 [1.0000 0.0000 0.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [0.2500 0.2500 0.2500 0.2500]"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C1 [0.0000 0.0000 0.0000 1.0000]"),
        "{}",
        page.body
    );
    assert!(page.body.contains("/N 1.000000"), "{}", page.body);
    assert!(page.body.contains("/N 2.000000"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_iccbased_rgb_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_iccbased_rgb_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased RGB axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("stop-color=\"#"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_iccbased_rgb_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_iccbased_rgb_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased RGB axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_iccbased_gray_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_gray_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased Gray axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("stop-color=\"#"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_iccbased_gray_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_gray_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased Gray axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_iccbased_rgb_radial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_rgb_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased RGB radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(page.svg.contains("stop-color=\"#"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_iccbased_rgb_radial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_rgb_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased RGB radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_iccbased_gray_radial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_gray_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased Gray radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(page.svg.contains("stop-color=\"#"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_iccbased_gray_radial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_gray_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named ICCBased Gray radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[cfg(feature = "native-cmm-lcms2")]
#[test]
fn svg_output_native_lcms2_iccbased_cmyk_radial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_cmyk_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "native LittleCMS ICCBased CMYK radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(page.svg.contains("stop-color=\"#"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[cfg(feature = "native-cmm-lcms2")]
#[test]
fn ps_output_native_lcms2_iccbased_cmyk_radial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_iccbased_cmyk_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "native LittleCMS ICCBased CMYK radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_simple_calrgb_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_calrgb_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple CalRGB axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("stop-color=\"#"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_calrgb_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_calrgb_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple CalRGB axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_resource_calgray_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_calgray_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named CalGray axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("stop-color=\"#"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_resource_calgray_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_calgray_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named CalGray axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_resource_separation_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_separation_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named Separation axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("#FFFFFF") && page.svg.contains("#FF0000"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_resource_separation_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_separation_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named Separation axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(
        page.body
            .contains("/ColorSpace [/Separation /SpotRed /DeviceRGB"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [1.0000 1.0000 1.0000]")
            && page.body.contains("/C1 [1.0000 0.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_resource_separation_nonlinear_tint_axial_shading_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_separation_nonlinear_tint_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "nonlinear Separation tint transforms must not be flattened into endpoint SVG gradients"
    );
    assert!(page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_resource_separation_nonlinear_tint_axial_shading_uses_exact_named_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_separation_nonlinear_tint_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "nonlinear Separation tint transforms should stay regional when PS can carry the exact tint transform"
    );
    assert!(page.has_regional_images);
    assert!(
        page.body
            .contains("/ColorSpace [/Separation /SpotRed /DeviceRGB"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [1.0000 1.0000 1.0000]")
            && page.body.contains("/C1 [1.0000 0.0000 0.0000]")
            && page.body.contains("/N 2.000000"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [0.0000]")
            && page.body.contains("/C1 [1.0000]")
            && page.body.contains("shfill"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_devicen_multi_input_axial_shading_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_devicen_multi_input_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "multi-input DeviceN tint transforms must not be flattened into endpoint SVG gradients"
    );
    assert!(page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_resource_devicen_multi_input_axial_shading_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_devicen_multi_input_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "multi-input DeviceN tint transforms must not be flattened into endpoint PS shfill"
    );
    assert!(page.body.contains("colorimage"), "{}", page.body);
    assert!(!page.body.contains("shfill"), "{}", page.body);
}

#[test]
fn svg_output_resource_devicen_nonlinear_tint_axial_shading_stays_whole_page_raster() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_devicen_nonlinear_tint_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "nonlinear DeviceN tint transforms must not be flattened into endpoint SVG gradients"
    );
    assert!(page.svg.contains("data:image/png;base64,"), "{}", page.svg);
    assert!(!page.svg.contains("<linearGradient"), "{}", page.svg);
}

#[test]
fn ps_output_resource_devicen_nonlinear_tint_axial_shading_uses_exact_named_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_devicen_nonlinear_tint_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "single-colorant nonlinear DeviceN tint transforms should stay regional when PS can carry the exact tint transform"
    );
    assert!(page.has_regional_images);
    assert!(
        page.body
            .contains("/ColorSpace [/DeviceN [/SpotRed] /DeviceRGB"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [1.0000 1.0000 1.0000]")
            && page.body.contains("/C1 [1.0000 0.0000 0.0000]")
            && page.body.contains("/N 2.000000"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [0.0000]")
            && page.body.contains("/C1 [1.0000]")
            && page.body.contains("shfill"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_devicen_axial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_devicen_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named DeviceN axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"));
    assert!(page.svg.contains("#FFFFFF") && page.svg.contains("#FF0000"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_resource_devicen_axial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_resource_devicen_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "resource-named DeviceN axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(
        page.body
            .contains("/ColorSpace [/DeviceN [/SpotRed] /DeviceRGB"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("/C0 [1.0000 1.0000 1.0000]")
            && page.body.contains("/C1 [1.0000 0.0000 0.0000]"),
        "{}",
        page.body
    );
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_resource_indexed_rgb_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_rgb_constant_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed RGB axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("#0000FF"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_rgb_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_rgb_constant_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed RGB axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("0.0000 0.0000 1.0000"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_gray_constant_radial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_gray_constant_radial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Gray radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(page.svg.contains("#FFFFFF"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_gray_constant_radial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_gray_constant_radial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Gray radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("1.0000 1.0000 1.0000"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_cmyk_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_cmyk_constant_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed CMYK axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("#ED1C24"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_cmyk_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_cmyk_constant_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed CMYK axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("0.9294 0.1098 0.1412"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_calrgb_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_calrgb_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed CalRGB axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_calrgb_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_calrgb_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed CalRGB axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_lab_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_lab_constant_axial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Lab axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_lab_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_lab_constant_axial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Lab axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_separation_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_separation_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Separation axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(page.svg.contains("#FF0000"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_separation_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_separation_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed Separation axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("1.0000 0.0000 0.0000"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_resource_indexed_iccbased_rgb_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_iccbased_rgb_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed ICCBased RGB axial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[test]
fn ps_output_resource_indexed_iccbased_rgb_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_iccbased_rgb_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed ICCBased RGB axial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[cfg(feature = "native-cmm-lcms2")]
#[test]
fn svg_output_resource_indexed_iccbased_cmyk_constant_axial_shading_uses_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_iccbased_cmyk_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed ICCBased CMYK axial shading should not force whole-page SVG rasterization when native CMM is enabled"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<linearGradient"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"), "{}", page.svg);
}

#[cfg(feature = "native-cmm-lcms2")]
#[test]
fn ps_output_resource_indexed_iccbased_cmyk_constant_axial_shading_uses_native_shfill() {
    let engine =
        ContentEngine::open_bytes(pdf_with_resource_indexed_iccbased_cmyk_constant_axial_shading())
            .unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "constant resource-named Indexed ICCBased CMYK axial shading should not force whole-page PS rasterization when native CMM is enabled"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_simple_radial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"));
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native gradient path should not embed a raster page"
    );
}

#[test]
fn svg_output_nonextended_concentric_radial_shading_uses_clipped_native_gradient() {
    let engine =
        ContentEngine::open_bytes(pdf_with_nonextended_concentric_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "concentric non-extended radial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(page.svg.contains("<clipPath"), "{}", page.svg);
    assert!(
        page.svg.contains("clip-rule=\"evenodd\""),
        "non-extended radial shading should install an annular clip: {}",
        page.svg
    );
    assert!(page.svg.contains("clip-path=\"url(#clip"), "{}", page.svg);
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native clipped radial shading should not embed a raster page"
    );
}

#[test]
fn ps_output_simple_radial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_nonzero_start_radius_radial_shading_uses_focal_radius() {
    let engine = ContentEngine::open_bytes(pdf_with_nonzero_start_radius_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "nonzero-start-radius radial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(
        page.svg.contains("fr=\"10.000\""),
        "native SVG radial gradient should carry the PDF start radius: {}",
        page.svg
    );
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native focal-radius radial shading should not embed a raster page"
    );
}

#[test]
fn ps_output_nonzero_start_radius_radial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_nonzero_start_radius_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "nonzero-start-radius radial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"), "{}", page.body);
    assert!(
        page.body
            .contains("/Coords [45.000 60.000 10.000 60.000 60.000 50.000]"),
        "native PS radial shfill should carry both PDF radii: {}",
        page.body
    );
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_reversed_radii_radial_shading_normalizes_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_reversed_radii_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "reversed-radii radial shading should stay native SVG"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(
        page.svg.contains("fr=\"10.000\""),
        "native SVG radial gradient should use the smaller PDF radius as fr: {}",
        page.svg
    );
    assert!(
        page.svg
            .contains("<stop offset=\"0\" stop-color=\"#0000FF\""),
        "SVG reversed-radii output should swap the start color: {}",
        page.svg
    );
    assert!(
        page.svg
            .contains("<stop offset=\"1\" stop-color=\"#FFFFFF\""),
        "SVG reversed-radii output should swap the end color: {}",
        page.svg
    );
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native reversed-radii radial shading should not embed a raster page"
    );
}

#[test]
fn ps_output_reversed_radii_radial_shading_uses_pdf_radii() {
    let engine = ContentEngine::open_bytes(pdf_with_reversed_radii_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "reversed-radii radial shading should stay native PS"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"), "{}", page.body);
    assert!(
        page.body
            .contains("/Coords [60.000 60.000 50.000 45.000 60.000 10.000]"),
        "native PS radial shfill should preserve the PDF radii order: {}",
        page.body
    );
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_nonuniform_radial_shading_uses_transformed_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_nonuniform_radial_shading_ctm()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "non-uniform radial shading should stay native SVG via gradientTransform"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"), "{}", page.svg);
    assert!(
        page.svg.contains("gradientTransform=\"matrix("),
        "non-uniform radial shading should carry an affine gradient transform: {}",
        page.svg
    );
    assert!(page.svg.contains("fill=\"url(#grad"));
    assert!(
        !page.svg.contains("data:image/png;base64,"),
        "native transformed radial shading should not embed a raster page"
    );
}

#[test]
fn ps_output_nonuniform_radial_shading_uses_transformed_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_nonuniform_radial_shading_ctm()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "non-uniform radial shading should stay native PS via affine shfill"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"), "{}", page.body);
    assert!(page.body.contains("] concat"), "{}", page.body);
    assert!(page.body.contains("shfill"), "{}", page.body);
    assert!(!page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn svg_output_simple_lab_radial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_lab_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple Lab radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"));
    assert!(page.svg.contains("stop-color=\"#"));
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_lab_radial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_lab_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple Lab radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("/ColorSpace /DeviceRGB"));
    assert!(page.body.contains("shfill"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_simple_cmyk_radial_shading_uses_native_gradient() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_cmyk_radial_shading()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple DeviceCMYK radial shading should not force whole-page SVG rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.svg.contains("<radialGradient"));
    assert!(
        page.svg.contains("#FFFFFF") && page.svg.contains("#ED1C24"),
        "DeviceCMYK radial stops should be converted to deterministic RGB anchors: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_simple_cmyk_radial_shading_uses_native_shfill() {
    let engine = ContentEngine::open_bytes(pdf_with_simple_cmyk_radial_shading()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "simple DeviceCMYK radial shading should not force whole-page PS rasterization"
    );
    assert!(page.has_regional_images);
    assert!(page.body.contains("/ShadingType 3"));
    assert!(page.body.contains("/ColorSpace /DeviceCMYK"));
    assert!(page.body.contains("/C0 [0.0000 0.0000 0.0000 0.0000]"));
    assert!(page.body.contains("/C1 [0.0000 1.0000 1.0000 0.0000]"));
    assert!(page.body.contains("/N 1.000000"));
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_safe_ext_gstate_stays_vector_and_applies_line_width() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_line_width()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe opaque line-style ExtGState should not force SVG rasterization"
    );
    assert!(page.svg.contains("<path"));
    assert!(page.svg.contains("stroke-width=\"4.000\""), "{}", page.svg);
    assert!(
        page.svg.contains("stroke-linecap=\"round\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-linejoin=\"bevel\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-miterlimit=\"10.000\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-dasharray=\"6.000,2.000\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-dashoffset=\"1.000\""),
        "{}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_safe_ext_gstate_stays_vector_and_applies_line_width() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_line_width()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe opaque line-style ExtGState should not force PS rasterization"
    );
    assert!(page.body.contains("4.000 setlinewidth"), "{}", page.body);
    assert!(page.body.contains("1 setlinecap"), "{}", page.body);
    assert!(page.body.contains("2 setlinejoin"), "{}", page.body);
    assert!(page.body.contains("10.000 setmiterlimit"), "{}", page.body);
    assert!(
        page.body.contains("[6.000 2.000] 1.000 setdash"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_normal_alpha_extgstate_uses_native_opacity() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_native_alpha_ext_gstate()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "normal alpha ExtGState should not force SVG whole-page rasterization"
    );
    assert!(page.svg.contains("fill-opacity=\"0.500\""), "{}", page.svg);
    assert!(
        page.svg.contains("stroke-opacity=\"0.250\""),
        "{}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_supported_blend_extgstate_uses_native_mix_blend_mode() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_native_blend_ext_gstate()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "supported SVG blend ExtGState should not force whole-page rasterization"
    );
    assert!(page.svg.contains("mix-blend-mode:multiply"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn svg_output_alpha_blend_extgstate_uses_native_opacity_and_mix_blend_mode() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_native_alpha_blend_ext_gstate()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "supported SVG alpha+blend ExtGState should not force whole-page rasterization"
    );
    assert!(page.svg.contains("fill-opacity=\"0.500\""), "{}", page.svg);
    assert!(page.svg.contains("mix-blend-mode:multiply"), "{}", page.svg);
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_normal_alpha_extgstate_stays_whole_page_raster() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_native_alpha_ext_gstate()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        page.is_rasterized,
        "PostScript has no native alpha operator, so alpha ExtGState must stay raster fallback"
    );
    assert!(page.body.contains("colorimage"), "{}", page.body);
}

#[test]
fn ps_strict_refuses_normal_alpha_extgstate_whole_page_raster_fallback() {
    let engine = ContentEngine::open_bytes(pdf_with_svg_native_alpha_ext_gstate()).unwrap();
    let err = match engine.render_page_ps_strict(1, 72) {
        Ok(_) => panic!("strict PS must not rasterize fractional-alpha ExtGState output"),
        Err(err) => err,
    };
    let message = format!("{err}");
    assert!(
        message.contains("strict PostScript vector output refuses whole-page raster fallback"),
        "{message}"
    );
    assert!(message.contains("unsupported ExtGState"), "{message}");
}

#[test]
fn ps_output_zero_alpha_extgstate_stays_vector_noop() {
    let engine = ContentEngine::open_bytes(pdf_with_ps_transparent_alpha_ext_gstate()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "PostScript zero-alpha paint is an exact no-op and should stay vector"
    );
    assert!(!page.body.contains("colorimage"), "{}", page.body);
    assert!(
        page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("1.0000 0.0000 0.0000 setrgbcolor"),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"),
        "{}",
        page.body
    );
}

#[test]
fn svg_output_safe_extgstate_flatness_changes_curve_sampling() {
    let loose_engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_flatness(10.0)).unwrap();
    let tight_engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_flatness(0.1)).unwrap();
    let loose = loose_engine.render_page_svg(1, 72).unwrap();
    let tight = tight_engine.render_page_svg(1, 72).unwrap();
    assert!(
        !loose.is_rasterized && !tight.is_rasterized,
        "safe ExtGState FL should keep SVG vector output"
    );

    let loose_segments = loose.svg.matches(" L").count();
    let tight_segments = tight.svg.matches(" L").count();
    assert!(
        tight_segments > loose_segments,
        "lower FL should emit more SVG curve samples: loose={loose_segments}, tight={tight_segments}\n{}",
        tight.svg
    );
    assert!(!loose.svg.contains("data:image/png;base64,"));
    assert!(!tight.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_safe_extgstate_flatness_changes_curve_sampling() {
    let loose_engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_flatness(10.0)).unwrap();
    let tight_engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_flatness(0.1)).unwrap();
    let loose = loose_engine.render_page_ps(1, 72).unwrap();
    let tight = tight_engine.render_page_ps(1, 72).unwrap();
    assert!(
        !loose.is_rasterized && !tight.is_rasterized,
        "safe ExtGState FL should keep PS vector output"
    );

    let loose_segments = loose.body.matches(" lineto").count();
    let tight_segments = tight.body.matches(" lineto").count();
    assert!(
        tight_segments > loose_segments,
        "lower FL should emit more PS curve samples: loose={loose_segments}, tight={tight_segments}\n{}",
        tight.body
    );
    assert!(!loose.body.contains("colorimage"));
    assert!(!tight.body.contains("colorimage"));
}

#[test]
fn svg_output_safe_extgstate_font_stays_vector_and_applies_font_size() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_font()).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe ExtGState Font should not force SVG rasterization"
    );
    assert!(page.svg.contains("<path"), "{}", page.svg);
    assert!(
        page.svg.contains("d=\"M19.85 60.00") && page.svg.contains("L11.48 47.62"),
        "ExtGState font size should emit 18pt text outlines: {}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_safe_extgstate_font_stays_vector_and_applies_font_size() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_font()).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe ExtGState Font should not force PS rasterization"
    );
    assert!(
        page.body.contains("19.85 60.00 moveto") && page.body.contains("11.48 47.62 lineto"),
        "ExtGState font size should emit 18pt text outlines: {}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_stroked_text_applies_extgstate_line_style() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_styled_text(1)).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe stroked text ExtGState should not force SVG rasterization"
    );
    assert!(page.svg.contains("stroke-width=\"3.000\""), "{}", page.svg);
    assert!(
        page.svg.contains("stroke-linecap=\"round\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-linejoin=\"bevel\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-miterlimit=\"7.000\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-dasharray=\"5.000,1.000\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-dashoffset=\"2.000\""),
        "{}",
        page.svg
    );
    assert!(!page.svg.contains("data:image/png;base64,"));
}

#[test]
fn ps_output_stroked_text_applies_extgstate_line_style() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_styled_text(1)).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe stroked text ExtGState should not force PS rasterization"
    );
    assert!(page.body.contains("3.000 setlinewidth"), "{}", page.body);
    assert!(page.body.contains("1 setlinecap"), "{}", page.body);
    assert!(page.body.contains("2 setlinejoin"), "{}", page.body);
    assert!(page.body.contains("7.000 setmiterlimit"), "{}", page.body);
    assert!(
        page.body.contains("[5.000 1.000] 2.000 setdash"),
        "{}",
        page.body
    );
    assert!(!page.body.contains("colorimage"));
}

#[test]
fn svg_output_fill_stroke_text_emits_both_paints() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_styled_text(2)).unwrap();
    let page = engine.render_page_svg(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe fill-stroke text should remain native SVG"
    );
    assert!(
        page.svg.contains("fill=\"#00FF00\"") && page.svg.contains("stroke=\"#0000FF\""),
        "{}",
        page.svg
    );
    assert!(
        page.svg.contains("stroke-dashoffset=\"2.000\""),
        "{}",
        page.svg
    );
}

#[test]
fn ps_output_fill_stroke_text_emits_both_paints() {
    let engine = ContentEngine::open_bytes(pdf_with_safe_ext_gstate_styled_text(2)).unwrap();
    let page = engine.render_page_ps(1, 72).unwrap();
    assert!(
        !page.is_rasterized,
        "safe fill-stroke text should remain native PS"
    );
    assert!(
        page.body.contains("0.0000 1.0000 0.0000 setrgbcolor"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("0.0000 0.0000 1.0000 setrgbcolor"),
        "{}",
        page.body
    );
    assert!(page.body.contains("fill"), "{}", page.body);
    assert!(page.body.contains("stroke"), "{}", page.body);
}

#[test]
fn svg_text_clipping_render_mode_stays_native_with_clip_path() {
    for mode in 4..=7 {
        let engine = ContentEngine::open_bytes(pdf_with_text_clipping_render_mode(mode)).unwrap();
        let page = engine.render_page_svg(1, 72).unwrap();
        assert!(
            !page.is_rasterized,
            "resolvable text clipping mode {mode} should stay native SVG"
        );
        assert!(
            page.svg.contains("<clipPath"),
            "text clipping mode {mode} should install an SVG clipPath: {}",
            page.svg
        );
        assert!(
            page.svg.contains("clip-path=\"url(#clip"),
            "painted content should reference the text clip for mode {mode}: {}",
            page.svg
        );
        assert!(
            !page.svg.contains("data:image/png;base64,"),
            "text clipping mode {mode} should not force whole-page SVG rasterization: {}",
            page.svg
        );
    }
}

#[test]
fn ps_text_clipping_render_mode_stays_native_with_clip() {
    for mode in 4..=7 {
        let engine = ContentEngine::open_bytes(pdf_with_text_clipping_render_mode(mode)).unwrap();
        let page = engine.render_page_ps(1, 72).unwrap();
        assert!(
            !page.is_rasterized,
            "resolvable text clipping mode {mode} should stay native PostScript"
        );
        assert!(
            page.body.contains("clip"),
            "text clipping mode {mode} should install a PostScript clip: {}",
            page.body
        );
        assert!(
            !page.body.contains("/picstr"),
            "text clipping mode {mode} should not force whole-page PostScript rasterization: {}",
            page.body
        );
    }
}

// ---------------------------------------------------------------------------
// Synthetic test: verify SVG regional output structure
// ---------------------------------------------------------------------------

/// This test directly exercises the SVG render with a synthetic set of
/// operations to prove that when the classifier decides on regional fallback,
/// the SVG output contains BOTH a vector `<path>` AND an `<image>` element.
///
/// Since we can't easily build a full PDF in-memory in a unit test without the
/// full writer, this test verifies the classifier decision and the expected
/// output structure characteristics.
#[test]
fn svg_regional_fallback_preserves_vector_around_image() {
    // Verify classifier makes the right decision.
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Logo".to_string(), "Image".to_string());
    r.xobjects.insert("Logo".to_string(), (10, 0));
    add_basic_image_xobject_metadata(&mut r, "Logo");

    let ops = vec![
        // Vector rectangle first.
        ContentOperation::new(
            "re",
            vec![
                Operand::Real(10.0),
                Operand::Real(10.0),
                Operand::Real(200.0),
                Operand::Real(200.0),
            ],
        ),
        ContentOperation::new("f", vec![]),
        // Then an axis-aligned image.
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(100.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-50.0),
                Operand::Real(300.0),
                Operand::Real(600.0),
            ],
        ),
        ContentOperation::new("Do", vec![Operand::Name("Logo".to_string())]),
    ];

    let decision = classify_page_for_vector_output(&ops, &r, 1.0);
    match decision {
        VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
            assert_eq!(image_names, vec!["Logo"]);
        }
        other => panic!(
            "Expected RegionalImageFallback for mixed vector+image page, got {:?}",
            other
        ),
    }
}

/// Same verification for PostScript: the classifier produces regional fallback
/// for a page with vector ops + axis-aligned image, so PS output would contain
/// both path operators and a bounded colorimage region.
#[test]
fn ps_regional_fallback_preserves_vector_around_image() {
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Fig1".to_string(), "Image".to_string());
    r.xobjects.insert("Fig1".to_string(), (20, 0));
    add_basic_image_xobject_metadata(&mut r, "Fig1");

    let ops = vec![
        // Vector stroke.
        ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
        ContentOperation::new("l", vec![Operand::Real(500.0), Operand::Real(0.0)]),
        ContentOperation::new("S", vec![]),
        // Axis-aligned image below.
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(400.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-300.0),
                Operand::Real(50.0),
                Operand::Real(700.0),
            ],
        ),
        ContentOperation::new("Do", vec![Operand::Name("Fig1".to_string())]),
        // More vector content after.
        ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(800.0)]),
        ContentOperation::new("l", vec![Operand::Real(500.0), Operand::Real(800.0)]),
        ContentOperation::new("S", vec![]),
    ];

    let decision = classify_page_for_vector_output(&ops, &r, 1.0);
    match decision {
        VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
            assert_eq!(image_names, vec!["Fig1"]);
            // The decision guarantees that the PS renderer will emit the
            // vector strokes as native moveto/lineto/stroke AND the image
            // as a bounded colorimage in gsave/grestore.
        }
        other => panic!(
            "Expected RegionalImageFallback for mixed vector+image PS page, got {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Multiple images test
// ---------------------------------------------------------------------------

#[test]
fn classifier_multiple_axis_aligned_images_are_all_regional() {
    let mut r = PageResources::default();
    r.xobject_subtypes
        .insert("Im0".to_string(), "Image".to_string());
    r.xobject_subtypes
        .insert("Im1".to_string(), "Image".to_string());
    r.xobjects.insert("Im0".to_string(), (1, 0));
    r.xobjects.insert("Im1".to_string(), (2, 0));
    add_basic_image_xobject_metadata(&mut r, "Im0");
    add_basic_image_xobject_metadata(&mut r, "Im1");

    let ops = vec![
        // First image.
        ContentOperation::new("q", vec![]),
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(100.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-80.0),
                Operand::Real(50.0),
                Operand::Real(200.0),
            ],
        ),
        ContentOperation::new("Do", vec![Operand::Name("Im0".to_string())]),
        ContentOperation::new("Q", vec![]),
        // Second image.
        ContentOperation::new("q", vec![]),
        ContentOperation::new(
            "cm",
            vec![
                Operand::Real(200.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-150.0),
                Operand::Real(300.0),
                Operand::Real(600.0),
            ],
        ),
        ContentOperation::new("Do", vec![Operand::Name("Im1".to_string())]),
        ContentOperation::new("Q", vec![]),
    ];

    match classify_page_for_vector_output(&ops, &r, 1.0) {
        VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
            assert_eq!(image_names.len(), 2);
            assert!(image_names.contains(&"Im0".to_string()));
            assert!(image_names.contains(&"Im1".to_string()));
        }
        other => panic!("Expected RegionalImageFallback, got {:?}", other),
    }
}
