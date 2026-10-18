//! Additional ISO 19005 rules for Wellfriend's PDF/A validator.
//!
//! These checks cover the features the PDF/A converter repairs (actions,
//! stream filters, external content, annotations, forms, optional content,
//! device colour, Unicode mapping and PDF/A-1 transparency) so that the
//! converter's quality gate can never pass a file whose repairs did not take.

use std::collections::BTreeSet;

use crate::compliance::{ComplianceViolation, PdfAProfile};
use crate::document::PdfDocument;
use crate::error::Result;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

/// Action types allowed by the profile (ISO 19005-1 6.6.1, 19005-2/3 6.6.1).
pub(crate) fn action_allowed(profile: PdfAProfile, action: &PdfDictionary) -> bool {
    let Some(kind) = action.get_name("S") else {
        return true;
    };
    match kind {
        "GoTo" | "GoToR" | "Thread" | "URI" | "SubmitForm" => true,
        "GoToE" => !profile.is_pdfa1(),
        "Named" => matches!(
            action.get_name("N"),
            Some("NextPage" | "PrevPage" | "FirstPage" | "LastPage")
        ),
        _ => false,
    }
}

/// Annotation subtypes allowed by the profile.
pub(crate) fn annotation_allowed(profile: PdfAProfile, subtype: &str) -> bool {
    let common = matches!(
        subtype,
        "Text"
            | "Link"
            | "FreeText"
            | "Line"
            | "Square"
            | "Circle"
            | "Highlight"
            | "Underline"
            | "Squiggly"
            | "StrikeOut"
            | "Stamp"
            | "Ink"
            | "Popup"
            | "Widget"
            | "PrinterMark"
            | "TrapNet"
    );
    if profile.is_pdfa1() {
        return common;
    }
    common
        || matches!(
            subtype,
            "Polygon" | "PolyLine" | "Caret" | "Watermark" | "Redact"
        )
        || (subtype == "FileAttachment" && profile.allows_embedded_files())
}

pub(crate) const ANNOT_INVISIBLE: i64 = 1;
pub(crate) const ANNOT_HIDDEN: i64 = 2;
pub(crate) const ANNOT_PRINT: i64 = 4;
pub(crate) const ANNOT_NOVIEW: i64 = 32;
pub(crate) const ANNOT_TOGGLE_NOVIEW: i64 = 256;

pub(crate) fn filter_names(dict: &PdfDictionary) -> Vec<String> {
    match dict.get("Filter") {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_name().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn validate_extended_rules(
    doc: &PdfDocument,
    catalog: &PdfDictionary,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) -> Result<()> {
    let reader = doc.reader();
    let mut has_default_rgb = false;
    let mut has_default_cmyk = false;
    for (number, generation) in reader.object_ids() {
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let location = format!("object {number} {generation}");
        match &object {
            PdfObject::Stream { dict, .. } => {
                check_stream(dict, &location, profile, violations);
                check_dict(reader, dict, &location, profile, violations);
            }
            PdfObject::Dictionary(dict) => check_dict(reader, dict, &location, profile, violations),
            _ => {}
        }
        if let Some(dict) = object_dict(&object) {
            if dict_contains_key_deep(dict, "DefaultRGB", 0) {
                has_default_rgb = true;
            }
            if dict_contains_key_deep(dict, "DefaultCMYK", 0) {
                has_default_cmyk = true;
            }
        }
    }
    check_catalog(reader, catalog, profile, violations);
    check_annotations(doc, profile, violations)?;
    check_device_colour(doc, catalog, has_default_rgb, has_default_cmyk, violations)?;
    if profile.requires_unicode() {
        check_unicode(doc, violations)?;
    }
    Ok(())
}

fn object_dict(object: &PdfObject) -> Option<&PdfDictionary> {
    match object {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn dict_contains_key_deep(dict: &PdfDictionary, key: &str, depth: usize) -> bool {
    if depth > 3 {
        return false;
    }
    dict.entries().any(|(k, v)| {
        k == key
            || matches!(v, PdfObject::Dictionary(inner) if dict_contains_key_deep(inner, key, depth + 1))
    })
}

fn check_stream(
    dict: &PdfDictionary,
    location: &str,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) {
    let filters = filter_names(dict);
    if filters.iter().any(|f| f == "LZWDecode" || f == "LZW") {
        violations.push(ComplianceViolation::error(
            "pdfa.filter.lzw",
            location,
            "PDF/A forbids the LZWDecode filter",
        ));
    }
    if profile.is_pdfa1() && filters.iter().any(|f| f == "JPXDecode") {
        violations.push(ComplianceViolation::error(
            "pdfa1.jpx",
            location,
            "PDF/A-1 is limited to PDF 1.4 and forbids JPXDecode (JPEG 2000)",
        ));
    }
    for key in ["F", "FFilter", "FDecodeParms"] {
        if dict.contains_key(key) && dict.get_name("Type") != Some("EmbeddedFile") {
            violations.push(ComplianceViolation::error(
                "pdfa.external_stream",
                format!("{location}/{key}"),
                "PDF/A forbids stream data stored in external files",
            ));
        }
    }
    match dict.get_name("Subtype") {
        Some("PS") => violations.push(ComplianceViolation::error(
            "pdfa.xobject.postscript",
            location,
            "PDF/A forbids PostScript XObjects",
        )),
        Some("Form") => {
            if dict.contains_key("Ref") {
                violations.push(ComplianceViolation::error(
                    "pdfa.xobject.reference",
                    location,
                    "PDF/A forbids reference XObjects",
                ));
            }
            if dict.get_name("Subtype2") == Some("PS") {
                violations.push(ComplianceViolation::error(
                    "pdfa.xobject.postscript",
                    location,
                    "PDF/A forbids PostScript form XObjects",
                ));
            }
        }
        Some("Image") => {
            if dict.get_bool("Interpolate") == Some(true) {
                violations.push(ComplianceViolation::error(
                    "pdfa.image.interpolate",
                    location,
                    "PDF/A images must not set /Interpolate true",
                ));
            }
            if dict.contains_key("Alternates") {
                violations.push(ComplianceViolation::error(
                    "pdfa.image.alternates",
                    location,
                    "PDF/A images must not declare /Alternates",
                ));
            }
            if profile.is_pdfa1()
                && (dict.get("SMask").is_some_and(|v| !v.is_null())
                    || dict.get_integer("SMaskInData").unwrap_or(0) > 0)
            {
                violations.push(ComplianceViolation::error(
                    "pdfa1.transparency",
                    location,
                    "PDF/A-1 forbids soft-masked images",
                ));
            }
        }
        _ => {}
    }
    if dict.contains_key("OPI") {
        violations.push(ComplianceViolation::error(
            "pdfa.xobject.opi",
            location,
            "PDF/A forbids OPI proxies",
        ));
    }
}

fn check_dict(
    reader: &PdfReader,
    dict: &PdfDictionary,
    location: &str,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) {
    check_dict_inner(reader, dict, location, profile, violations, 0);
}

fn check_dict_inner(
    reader: &PdfReader,
    dict: &PdfDictionary,
    location: &str,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    if dict.contains_key("TR") {
        violations.push(ComplianceViolation::error(
            "pdfa.extgstate.transfer",
            format!("{location}/TR"),
            "PDF/A forbids transfer functions in graphics states",
        ));
    }
    if let Some(tr2) = dict.get("TR2") {
        if tr2.as_name() != Some("Default") {
            violations.push(ComplianceViolation::error(
                "pdfa.extgstate.transfer",
                format!("{location}/TR2"),
                "PDF/A only permits /TR2 /Default",
            ));
        }
    }
    if profile.is_pdfa1() {
        if let Some(smask) = dict.get("SMask") {
            let is_gstate = dict.get_name("Type") == Some("ExtGState")
                || dict.contains_key("ca")
                || dict.contains_key("CA")
                || dict.contains_key("BM");
            if is_gstate && smask.as_name() != Some("None") {
                violations.push(ComplianceViolation::error(
                    "pdfa1.transparency",
                    format!("{location}/SMask"),
                    "PDF/A-1 forbids soft masks",
                ));
            }
        }
        if let Some(bm) = dict.get("BM") {
            let normal = |obj: &PdfObject| matches!(obj.as_name(), Some("Normal" | "Compatible"));
            let ok = match bm {
                PdfObject::Array(items) => items.iter().all(normal),
                other => normal(other),
            };
            if !ok {
                violations.push(ComplianceViolation::error(
                    "pdfa1.transparency",
                    format!("{location}/BM"),
                    "PDF/A-1 forbids non-Normal blend modes",
                ));
            }
        }
    }
    for key in ["A", "Next", "PA"] {
        let Some(value) = dict.get(key) else { continue };
        let items: Vec<PdfObject> = match reader.resolve(value.clone()) {
            Ok(PdfObject::Array(items)) if key == "Next" => items,
            Ok(other) => vec![other],
            Err(_) => continue,
        };
        for item in items {
            let Ok(PdfObject::Dictionary(action)) = reader.resolve(item) else {
                continue;
            };
            if action.contains_key("S") && !action_allowed(profile, &action) {
                violations.push(ComplianceViolation::error(
                    "pdfa.action.disallowed",
                    format!("{location}/{key}"),
                    format!(
                        "PDF/A forbids the '{}' action",
                        action.get_name("S").unwrap_or("?")
                    ),
                ));
            }
        }
    }
    if !profile.allows_embedded_files() && dict.contains_key("EF") {
        violations.push(ComplianceViolation::error(
            "pdfa.embedded_file",
            format!("{location}/EF"),
            "This PDF/A level does not allow embedded files",
        ));
    }
    for (key, value) in dict.entries() {
        if let PdfObject::Dictionary(inner) = value {
            check_dict_inner(
                reader,
                inner,
                &format!("{location}/{key}"),
                profile,
                violations,
                depth + 1,
            );
        }
    }
}

fn resolve_dict(reader: &PdfReader, object: Option<&PdfObject>) -> Option<PdfDictionary> {
    match reader.resolve(object?.clone()).ok()? {
        PdfObject::Dictionary(dict) => Some(dict),
        PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn check_catalog(
    reader: &PdfReader,
    catalog: &PdfDictionary,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) {
    if let Some(form) = resolve_dict(reader, catalog.get("AcroForm")) {
        if form.get_bool("NeedAppearances") == Some(true) {
            violations.push(ComplianceViolation::error(
                "pdfa.form.need_appearances",
                "Catalog/AcroForm/NeedAppearances",
                "PDF/A forms must not set NeedAppearances true",
            ));
        }
        if form.contains_key("XFA") {
            violations.push(ComplianceViolation::error(
                "pdfa.form.xfa",
                "Catalog/AcroForm/XFA",
                "PDF/A forbids XFA forms",
            ));
        }
    }
    if catalog.contains_key("NeedsRendering") {
        violations.push(ComplianceViolation::error(
            "pdfa.form.xfa",
            "Catalog/NeedsRendering",
            "PDF/A forbids the NeedsRendering key",
        ));
    }
    let Some(oc) = resolve_dict(reader, catalog.get("OCProperties")) else {
        return;
    };
    if profile.is_pdfa1() {
        violations.push(ComplianceViolation::error(
            "pdfa1.optional_content",
            "Catalog/OCProperties",
            "PDF/A-1 forbids optional content",
        ));
        return;
    }
    let mut configs = Vec::new();
    if let Some(d) = resolve_dict(reader, oc.get("D")) {
        configs.push(("D".to_string(), d));
    }
    if let Some(PdfObject::Array(items)) = oc
        .get("Configs")
        .and_then(|v| reader.resolve(v.clone()).ok())
    {
        for (idx, item) in items.iter().enumerate() {
            if let Some(d) = resolve_dict(reader, Some(item)) {
                configs.push((format!("Configs[{idx}]"), d));
            }
        }
    }
    let mut names = BTreeSet::new();
    for (path, config) in configs {
        match config.get("Name").and_then(PdfObject::as_string) {
            Some(name) if names.insert(name.to_vec()) => {}
            _ => violations.push(ComplianceViolation::error(
                "pdfa.optional_content.config",
                format!("Catalog/OCProperties/{path}/Name"),
                "Optional content configurations require a unique Name",
            )),
        }
        if config.contains_key("AS") {
            violations.push(ComplianceViolation::error(
                "pdfa.optional_content.config",
                format!("Catalog/OCProperties/{path}/AS"),
                "Optional content configurations must not contain AS",
            ));
        }
    }
}

fn check_annotations(
    doc: &PdfDocument,
    profile: PdfAProfile,
    violations: &mut Vec<ComplianceViolation>,
) -> Result<()> {
    let reader = doc.reader();
    for page in doc.get_pages()? {
        let Ok(PdfObject::Dictionary(page_dict)) =
            reader.get_and_resolve(page.object_number, page.generation_number)
        else {
            continue;
        };
        let Some(PdfObject::Array(annots)) = page_dict
            .get("Annots")
            .and_then(|v| reader.resolve(v.clone()).ok())
        else {
            continue;
        };
        for (idx, annot) in annots.iter().enumerate() {
            let Some(annot) = resolve_dict(reader, Some(annot)) else {
                continue;
            };
            let location = format!("page {} Annots[{idx}]", page.page_number);
            let subtype = annot.get_name("Subtype").unwrap_or("");
            if !annotation_allowed(profile, subtype) {
                violations.push(ComplianceViolation::error(
                    "pdfa.annotation.subtype",
                    &location,
                    format!("PDF/A forbids '{subtype}' annotations"),
                ));
                continue;
            }
            let flags = annot.get_integer("F");
            if subtype != "Popup" {
                match flags {
                    None => violations.push(ComplianceViolation::error(
                        "pdfa.annotation.flags",
                        &location,
                        "Annotations require an F key with the Print flag",
                    )),
                    Some(f) => check_flags(f, &location, violations),
                }
            } else if let Some(f) = flags {
                check_flags(f, &location, violations);
            }
            let ap = resolve_dict(reader, annot.get("AP"));
            match &ap {
                Some(ap) => {
                    if ap.entries().any(|(key, _)| key != "N") {
                        violations.push(ComplianceViolation::error(
                            "pdfa.annotation.appearance",
                            format!("{location}/AP"),
                            "Appearance dictionaries must contain only the N key",
                        ));
                    }
                    if subtype == "Widget" {
                        let btn = field_type(reader, &annot).as_deref() == Some("Btn");
                        let normal = ap.get("N").and_then(|v| reader.resolve(v.clone()).ok());
                        let is_dict = matches!(normal, Some(PdfObject::Dictionary(_)));
                        if btn != is_dict && normal.is_some() {
                            violations.push(ComplianceViolation::error(
                                "pdfa.annotation.appearance",
                                format!("{location}/AP/N"),
                                "Button widgets need an N state dictionary; other widgets an N stream",
                            ));
                        }
                    }
                }
                None if !profile.is_pdfa1()
                    && !matches!(subtype, "Popup" | "Link")
                    && !zero_size_rect(reader, &annot) =>
                {
                    violations.push(ComplianceViolation::error(
                        "pdfa.annotation.appearance",
                        &location,
                        "Annotations require an appearance stream",
                    ));
                }
                None => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn field_type(reader: &PdfReader, annot: &PdfDictionary) -> Option<String> {
    let mut current = annot.clone();
    for _ in 0..32 {
        if let Some(ft) = current.get_name("FT") {
            return Some(ft.to_string());
        }
        current = resolve_dict(reader, current.get("Parent"))?;
    }
    None
}

fn zero_size_rect(reader: &PdfReader, annot: &PdfDictionary) -> bool {
    let Some(PdfObject::Array(rect)) = annot
        .get("Rect")
        .and_then(|v| reader.resolve(v.clone()).ok())
    else {
        return true;
    };
    let n: Vec<f64> = rect.iter().filter_map(PdfObject::as_number).collect();
    n.len() == 4 && ((n[2] - n[0]).abs() < 1e-9 || (n[3] - n[1]).abs() < 1e-9)
}

fn check_flags(flags: i64, location: &str, violations: &mut Vec<ComplianceViolation>) {
    if flags & ANNOT_PRINT == 0
        || flags & (ANNOT_HIDDEN | ANNOT_INVISIBLE | ANNOT_NOVIEW | ANNOT_TOGGLE_NOVIEW) != 0
    {
        violations.push(ComplianceViolation::error(
            "pdfa.annotation.flags",
            format!("{location}/F"),
            "Annotation flags must set Print and clear Hidden/Invisible/NoView/ToggleNoView",
        ));
    }
}

/// Number of components of the PDF/A OutputIntent profile, if any.
pub(crate) fn output_intent_components(reader: &PdfReader, catalog: &PdfDictionary) -> Option<i64> {
    let intents = reader.resolve(catalog.get("OutputIntents")?.clone()).ok()?;
    for intent in intents.as_array()? {
        let Some(dict) = resolve_dict(reader, Some(intent)) else {
            continue;
        };
        if dict.get_name("S") != Some("GTS_PDFA1") {
            continue;
        }
        if let Some(profile) = resolve_dict(reader, dict.get("DestOutputProfile")) {
            return profile.get_integer("N");
        }
    }
    None
}

fn check_device_colour(
    doc: &PdfDocument,
    catalog: &PdfDictionary,
    has_default_rgb: bool,
    has_default_cmyk: bool,
    violations: &mut Vec<ComplianceViolation>,
) -> Result<()> {
    let Some(components) = output_intent_components(doc.reader(), catalog) else {
        return Ok(()); // the missing OutputIntent is already reported
    };
    let usage = crate::pdfa_fonts::scan_content_usage(doc)?;
    if usage.device_rgb && components != 3 && !has_default_rgb {
        violations.push(ComplianceViolation::error(
            "pdfa.color.device_rgb",
            "content",
            "DeviceRGB is used but the OutputIntent is not RGB and no DefaultRGB is defined",
        ));
    }
    if usage.device_cmyk && components != 4 && !has_default_cmyk {
        violations.push(ComplianceViolation::error(
            "pdfa.color.device_cmyk",
            "content",
            "DeviceCMYK is used but the OutputIntent is not CMYK and no DefaultCMYK is defined",
        ));
    }
    Ok(())
}

fn check_unicode(doc: &PdfDocument, violations: &mut Vec<ComplianceViolation>) -> Result<()> {
    let reader = doc.reader();
    for font in crate::fonts_report::list_fonts(doc)? {
        if font.to_unicode {
            continue;
        }
        let Ok(PdfObject::Dictionary(dict)) =
            reader.get_and_resolve(font.object_number, font.generation)
        else {
            continue;
        };
        if simple_font_names_are_unicode(reader, &dict) {
            continue;
        }
        violations.push(ComplianceViolation::error(
            "pdfa.unicode.mapping",
            format!("object {} {}", font.object_number, font.generation),
            format!(
                "font '{}' has no ToUnicode CMap and no standard glyph names",
                font.name
            ),
        ));
    }
    Ok(())
}

fn simple_font_names_are_unicode(reader: &PdfReader, font: &PdfDictionary) -> bool {
    if matches!(
        font.get_name("Subtype"),
        Some("Type0") | Some("Type3") | None
    ) {
        return false;
    }
    let symbolic = resolve_dict(reader, font.get("FontDescriptor"))
        .and_then(|fd| fd.get_integer("Flags"))
        .is_some_and(|flags| flags & 4 != 0);
    match font
        .get("Encoding")
        .and_then(|v| reader.resolve(v.clone()).ok())
    {
        Some(PdfObject::Name(name)) => {
            matches!(
                name.as_str(),
                "WinAnsiEncoding" | "MacRomanEncoding" | "StandardEncoding"
            )
        }
        Some(PdfObject::Dictionary(enc)) => enc
            .get_array("Differences")
            .unwrap_or(&[])
            .iter()
            .filter_map(PdfObject::as_name)
            .all(|name| crate::fonts::glyph_list::glyph_name_to_unicode(name).is_some()),
        _ => !symbolic,
    }
}
