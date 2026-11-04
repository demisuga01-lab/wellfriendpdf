//! Applying operations. Every function maps input bytes to output bytes and
//! a report; the browser and the server call the same functions in the same
//! order, so the preview the user approved is the version the server saves.

use serde::Serialize;
use serde_json::json;
use wellfriend_pdf_edit as edit;
use wellfriendpdf_engine::{
    text_reflow::{apply_reflow_region, GeometricReflowRequest},
    ApprovedFontAsset, AuthorPageSize, Color, EditMode, FontFace, GraphicsStyle, ImageRect,
    PageTextLayout, ParagraphStyle, PdfBuilder, PdfEditor, RedactionOptions, TextAlign, TextStyle,
};

use crate::error::EditorError;
use crate::fonts::{ascent_ratio, covers, FontSource};
use crate::ops::{
    decode_image, decode_pdf, Align, EditorOperation, FieldValueKind, OperationList, Rect, Rgb, ShapeKind, SpotColor,
};
use crate::{overlay, scratch};

/// What one applied operation did, for the UI and the version record.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperationReport {
    pub index: usize,
    pub op: String,
    pub page: u32,
    /// User-space rectangle that changed, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_region: Option<[f64; 4]>,
    /// Caret offset after a text edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret: Option<usize>,
    /// Font resources the edit added (a substitute or library font).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fonts_added: Vec<String>,
    /// Plain-language notes for the editor UI (substitutions, removals).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Text removed by a redaction (glyph count only, never the text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_glyphs: Option<usize>,
    /// How text that did not fit was placed (`shrink`, `continue`, `nextPage`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow: Option<crate::overflow::OverflowOutcome>,
    /// The two parts of text continued on the next page, for the text check.
    #[serde(skip)]
    pub continued: Option<crate::overflow::Continued>,
}

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub bytes: Vec<u8>,
    pub reports: Vec<OperationReport>,
}

/// Validate and apply a whole list, in order.
pub fn apply_operations(
    input: &[u8],
    list: &OperationList,
    fonts: &dyn FontSource,
    allow_signature_invalidation: bool,
) -> Result<ApplyOutcome, EditorError> {
    list.validate()?;
    host_independent();
    let mut bytes = input.to_vec();
    let mut reports = Vec::with_capacity(list.operations.len());
    for (index, operation) in list.operations.iter().enumerate() {
        let (next, mut report) =
            apply_operation(&bytes, operation, fonts, allow_signature_invalidation)
                .map_err(|error| error.at(index))?;
        report.index = index;
        bytes = next;
        reports.push(report);
    }
    Ok(ApplyOutcome { bytes, reports })
}

/// Host-installed fonts would make the server's output differ from the
/// browser's (which has none): only the document's own fonts and the caller's
/// library are ever used.
pub fn host_independent() {
    wellfriendpdf_engine::fonts::set_installed_fonts_enabled(false);
}

/// Apply one already-validated operation.
pub fn apply_operation(
    input: &[u8],
    operation: &EditorOperation,
    fonts: &dyn FontSource,
    allow_signature_invalidation: bool,
) -> Result<(Vec<u8>, OperationReport), EditorError> {
    let mut report = OperationReport {
        op: operation.name().to_owned(),
        page: operation.page(),
        ..OperationReport::default()
    };
    let allow = allow_signature_invalidation;
    host_independent();
    let bytes = match operation {
        EditorOperation::ReplaceText {
            page,
            start,
            end,
            expected,
            text,
            fallback_font,
            overflow,
        } => {
            let (out, resolved) = crate::overflow::resolve(
                *overflow,
                || crate::restyle::text_size(input, *page, *start, *end),
                |fit| {
                    let mut attempt = report.clone();
                    let out = replace_text(
                        input,
                        *page,
                        *start,
                        *end,
                        expected,
                        text,
                        fallback_font.as_deref(),
                        fonts,
                        fit,
                        &mut attempt,
                    )?;
                    Ok((out, attempt))
                },
            )?;
            report = resolved;
            out
        }
        EditorOperation::TransformImage { selector, matrix } => {
            let (_, out) = scratch::run(input, None, |p| {
                edit::transform_image(&edit::TransformImageRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    selector: selector.clone(),
                    destination_matrix: *matrix,
                    allow_signature_invalidation: allow,
                })
            })?;
            report.changed_region = Some(union(
                matrix_bounds(&selector.expected_matrix),
                matrix_bounds(matrix),
            ));
            out
        }
        EditorOperation::DeleteImage { selector } => {
            let (_, out) = scratch::run(input, None, |p| {
                edit::delete_image(&edit::DeleteImageRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    selector: selector.clone(),
                    allow_signature_invalidation: allow,
                })
            })?;
            report.changed_region = Some(matrix_bounds(&selector.expected_matrix));
            out
        }
        EditorOperation::ReplaceImage { selector, image } if image.mime_type == "image/png" => {
            crate::images::replace_image_png(input, selector, &decode_image(image)?)?
        }
        EditorOperation::ReplaceImage { selector, image } => {
            let jpeg = decode_image(image)?;
            let (_, out) = scratch::run(input, Some(&jpeg), |p| {
                edit::replace_image(&edit::ReplaceImageRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    replacement_image_path: p.extra.expect("extra file").to_path_buf(),
                    selector: selector.clone(),
                    allow_signature_invalidation: allow,
                })
            })?;
            out
        }
        EditorOperation::EditVector { selector, mutation } => {
            let (_, out) = scratch::run(input, None, |p| {
                edit::edit_vector_path(&edit::EditVectorPathRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    selector: selector.clone(),
                    mutation: mutation.clone(),
                    allow_signature_invalidation: allow,
                })
            })?;
            out
        }
        EditorOperation::AddText {
            page,
            rect,
            text,
            font,
            size,
            color,
            align,
            line_height,
            overflow,
        } => {
            let item = crate::overflow::TextBox {
                page: *page,
                rect: *rect,
                text,
                font,
                size: *size,
                color,
                align: *align,
                line_height: *line_height,
            };
            let (out, resolved) = crate::overflow::resolve(
                *overflow,
                || Ok(*size),
                |fit| {
                    let mut attempt = report.clone();
                    let out = crate::overflow::add_text(input, &item, fit, fonts, &mut attempt)?;
                    Ok((out, attempt))
                },
            )?;
            report = resolved;
            out
        }
        EditorOperation::AddImage { page, rect, image } => {
            let data = decode_image(image)?;
            let png = image.mime_type == "image/png";
            report.changed_region = Some(bounds(rect));
            author_onto(input, *page, &[], |builder, origin| {
                let handle = if png {
                    builder.add_png_image(&data)?
                } else {
                    builder.add_jpeg_image(data.clone())?
                };
                let r = *rect;
                Ok(Box::new(
                    move |page: &mut wellfriendpdf_engine::PdfPageBuilder| {
                        page.draw_image(
                            handle,
                            r.x - origin[0],
                            r.y - origin[1],
                            r.width,
                            r.height,
                        );
                        Ok(())
                    },
                ))
            })?
        }
        EditorOperation::AddShape {
            page,
            shape,
            rect,
            stroke,
            fill,
            line_width,
        } => {
            report.changed_region = Some(bounds(rect));
            let style = GraphicsStyle {
                stroke: stroke.as_ref().map(rgb),
                fill: fill.as_ref().map(rgb),
                line_width: *line_width,
                ..GraphicsStyle::default()
            };
            let (shape, r) = (*shape, *rect);
            author_onto(input, *page, &spots([stroke.as_ref(), fill.as_ref()]), |_, origin| {
                Ok(Box::new(
                    move |page: &mut wellfriendpdf_engine::PdfPageBuilder| {
                        let (x, y) = (r.x - origin[0], r.y - origin[1]);
                        match shape {
                            ShapeKind::Rectangle => page.draw_rect(x, y, r.width, r.height, &style),
                            ShapeKind::Ellipse => page.draw_ellipse(
                                x + r.width / 2.0,
                                y + r.height / 2.0,
                                r.width / 2.0,
                                r.height / 2.0,
                                &style,
                            ),
                        };
                        Ok(())
                    },
                ))
            })?
        }
        EditorOperation::AddLine {
            page,
            from,
            to,
            color,
            line_width,
        } => {
            report.changed_region = Some([
                from[0].min(to[0]),
                from[1].min(to[1]),
                from[0].max(to[0]),
                from[1].max(to[1]),
            ]);
            let style = GraphicsStyle::stroke(rgb(color), *line_width);
            let (from, to) = (*from, *to);
            author_onto(input, *page, &spots([Some(color)]), |_, origin| {
                Ok(Box::new(
                    move |page: &mut wellfriendpdf_engine::PdfPageBuilder| {
                        page.draw_line(
                            from[0] - origin[0],
                            from[1] - origin[1],
                            to[0] - origin[0],
                            to[1] - origin[1],
                            &style,
                        );
                        Ok(())
                    },
                ))
            })?
        }
        EditorOperation::AddWhiteout { page, rect, color } => {
            report.changed_region = Some(bounds(rect));
            report
                .notes
                .push("Whiteout covers content; use Redact to remove it.".into());
            let style = GraphicsStyle {
                stroke: None,
                fill: Some(rgb(color)),
                ..GraphicsStyle::default()
            };
            let r = *rect;
            author_onto(input, *page, &spots([Some(color)]), |_, origin| {
                Ok(Box::new(
                    move |page: &mut wellfriendpdf_engine::PdfPageBuilder| {
                        page.draw_rect(r.x - origin[0], r.y - origin[1], r.width, r.height, &style);
                        Ok(())
                    },
                ))
            })?
        }
        EditorOperation::AddLink {
            page,
            rect,
            url,
            target_page,
        } => {
            report.changed_region = Some(bounds(rect));
            overlay::add_link(input, *page, bounds(rect), url.as_deref(), *target_page)?
        }
        EditorOperation::Redact { page, rect, fill } => {
            redact(input, *page, rect, fill.clone(), &mut report)?
        }
        EditorOperation::RestyleText {
            page,
            start,
            end,
            expected,
            font,
            size,
            color,
            align,
            line_height,
            overflow,
        } => {
            let (out, resolved) = crate::overflow::resolve(
                *overflow,
                || match size {
                    Some(size) => Ok(*size),
                    None => crate::restyle::text_size(input, *page, *start, *end),
                },
                |fit| {
                    let mut attempt = report.clone();
                    let restyle = crate::restyle::Restyle {
                        page: *page,
                        start: *start,
                        end: *end,
                        expected,
                        font,
                        size: *size,
                        color: color.clone(),
                        align: *align,
                        line_height: *line_height,
                        replacement: None,
                        fit,
                    };
                    let out = crate::restyle::restyle_text(input, &restyle, fonts, &mut attempt)?;
                    Ok((out, attempt))
                },
            )?;
            report = resolved;
            out
        }
        EditorOperation::CropImage { selector, crop } => {
            let (_, out) = scratch::run(input, None, |p| {
                edit::crop_image(&edit::CropImageRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    selector: selector.clone(),
                    destination_crop: edit::PdfNormalizedCrop {
                        left: crop.left,
                        bottom: crop.bottom,
                        right: crop.right,
                        top: crop.top,
                    },
                    allow_signature_invalidation: allow,
                })
            })?;
            report.changed_region = Some(matrix_bounds(&selector.expected_matrix));
            out
        }
        EditorOperation::SetImageOpacity { selector, opacity } => {
            let (_, out) = scratch::run(input, None, |p| {
                edit::set_image_opacity(&edit::SetImageOpacityRequest {
                    input_path: p.input.to_path_buf(),
                    output_path: p.output.to_path_buf(),
                    selector: selector.clone(),
                    destination_opacity: *opacity,
                    allow_signature_invalidation: allow,
                })
            })?;
            report.changed_region = Some(matrix_bounds(&selector.expected_matrix));
            out
        }
        EditorOperation::RotatePage { page, rotation } => crate::pages::rotate_page(input, *page, *rotation)?,
        EditorOperation::DeletePage { page } => crate::pages::delete_page(input, *page)?,
        EditorOperation::MovePage { page, to } => crate::pages::move_page(input, *page, *to)?,
        EditorOperation::InsertBlankPage { after, width, height } => {
            crate::pages::insert_blank_page(input, *after, *width, *height)?
        }
        EditorOperation::InsertPages { after, pdf, pages } => {
            crate::pages::insert_pages(input, *after, &decode_pdf(pdf)?, pages)?
        }
        EditorOperation::AddAnnotation {
            page,
            kind,
            rect,
            quads,
            ink,
            contents,
            label,
            color,
            author,
        } => {
            report.changed_region = Some(bounds(rect));
            crate::annotations::add_annotation(
                input,
                *page,
                *kind,
                rect,
                quads,
                ink,
                contents.as_deref(),
                label.as_deref(),
                color,
                author.as_deref(),
            )?
        }
        EditorOperation::SetFormField { name, value, kind } => {
            let mut editor = PdfEditor::open_bytes(input.to_vec())?;
            match kind {
                FieldValueKind::Text => editor.set_form_text(name.clone(), value.clone()),
                FieldValueKind::Choice => editor.set_form_choice(name.clone(), value.clone()),
                FieldValueKind::Checkbox => editor.set_form_checkbox(name.clone(), value == "true"),
            };
            editor.save_to_bytes(EditMode::FullRewrite)?
        }
    };
    // No text operation may lose the words it was asked to show.
    crate::overflow::ensure_kept(input, &bytes, operation, fonts, &report)?;
    Ok((bytes, report))
}

/// Draw an `addText` box (its overflow policy is `crate::overflow::add_text`).
pub(crate) fn add_text(
    input: &[u8],
    item: &crate::overflow::TextBox<'_>,
    fonts: &dyn FontSource,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let crate::overflow::TextBox { page, rect, text, font, size, color, align, line_height } = *item;
    let font_bytes = fonts.font_bytes(font).ok_or_else(|| {
        EditorError::new(
            "EDITOR_FONT_UNAVAILABLE",
            format!("font {font} is not in the font library"),
        )
    })?;
    let fallback = if covers(&font_bytes, text) {
        None
    } else {
        ["noto-sans-regular", "dejavu-sans-regular"]
            .iter()
            .find_map(|id| {
                fonts
                    .font_bytes(id)
                    .filter(|b| covers(b, text))
                    .map(|b| (*id, b))
            })
    };
    if let Some((id, _)) = &fallback {
        report
            .notes
            .push(format!("Characters missing from {font} are set in {id}."));
    } else if !covers(&font_bytes, text) {
        report.notes.push(format!(
            "Some characters are missing from {font} and every fallback font."
        ));
    }
    report.changed_region = Some([rect.x, rect.y, rect.x + rect.width, rect.y + rect.height]);
    report.fonts_added.push(font.to_owned());
    author_onto(input, page, &spots([Some(color)]), |builder, origin| {
        let face = builder
            .register_font_bytes(postscript_name(&font_bytes, font), font_bytes.clone())?;
        let face = match &fallback {
            Some((id, bytes)) => {
                let extra = builder
                    .register_font_bytes(postscript_name(bytes, id), bytes.clone())?;
                builder.register_font_stack(&[face, extra])?
            }
            None => face,
        };
        Ok(Box::new(
            move |page: &mut wellfriendpdf_engine::PdfPageBuilder| {
                draw_text_box(
                    page,
                    origin,
                    &rect,
                    text,
                    face,
                    size,
                    color,
                    align,
                    line_height,
                    ascent_ratio(&font_bytes),
                )
            },
        ))
    })
}

/// The font's PostScript name (so the PDF names the real typeface), else the id.
fn postscript_name(bytes: &[u8], id: &str) -> String {
    ttf_parser::Face::parse(bytes, 0)
        .ok()
        .and_then(|face| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .find_map(|n| n.to_string())
        })
        .filter(|name| !name.is_empty() && name.is_ascii())
        .unwrap_or_else(|| id.to_owned())
}

fn rgb(color: &Rgb) -> Color {
    color.engine()
}

/// Spot colours an operation paints with (rewritten to separations on merge).
fn spots<'a>(colors: impl IntoIterator<Item = Option<&'a Rgb>>) -> Vec<SpotColor> {
    colors.into_iter().flatten().filter_map(Rgb::spot).collect()
}

fn bounds(rect: &Rect) -> [f64; 4] {
    [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// The unit square under an image matrix.
pub fn matrix_bounds(m: &edit::PdfMatrix) -> [f64; 4] {
    let points = [
        (m.e, m.f),
        (m.a + m.e, m.b + m.f),
        (m.c + m.e, m.d + m.f),
        (m.a + m.c + m.e, m.b + m.d + m.f),
    ];
    let xs = points.iter().map(|p| p.0);
    let ys = points.iter().map(|p| p.1);
    [
        xs.clone().fold(f64::INFINITY, f64::min),
        ys.clone().fold(f64::INFINITY, f64::min),
        xs.fold(f64::NEG_INFINITY, f64::max),
        ys.fold(f64::NEG_INFINITY, f64::max),
    ]
}

type DrawFn<'a> = Box<
    dyn FnOnce(
            &mut wellfriendpdf_engine::PdfPageBuilder,
        ) -> Result<(), wellfriendpdf_engine::WellfriendError>
        + 'a,
>;

/// Author a one-page overlay the size of the target page's MediaBox and
/// merge it onto `page`. `prepare` registers fonts/images and returns the
/// drawing closure; drawing coordinates are relative to the MediaBox origin.
fn author_onto<'a>(
    input: &[u8],
    page: u32,
    spot_colors: &[SpotColor],
    prepare: impl FnOnce(
        &mut PdfBuilder,
        [f64; 2],
    ) -> Result<DrawFn<'a>, wellfriendpdf_engine::WellfriendError>,
) -> Result<Vec<u8>, EditorError> {
    let document = overlay::load(input)?;
    let page_object = overlay::page_id(&document, page)?;
    let [x0, y0, x1, y1] = overlay::media_box(&document, page_object)?;
    drop(document);
    let mut builder = PdfBuilder::new();
    builder.set_compress_content(true);
    let draw = prepare(&mut builder, [x0, y0])?;
    let page_builder = builder.add_page(AuthorPageSize::custom(x1 - x0, y1 - y0));
    draw(page_builder)?;
    let authored = builder.to_bytes()?;
    overlay::merge(input, page, &authored, spot_colors)
}

#[allow(clippy::too_many_arguments)]
fn draw_text_box(
    page: &mut wellfriendpdf_engine::PdfPageBuilder,
    origin: [f64; 2],
    rect: &Rect,
    text: &str,
    face: FontFace,
    size: f64,
    color: &Rgb,
    align: Align,
    line_height: f64,
    ascent: f64,
) -> Result<(), wellfriendpdf_engine::WellfriendError> {
    let style = TextStyle::new(face, size).fill(rgb(color));
    let paragraph = ParagraphStyle::new()
        .align(match align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
        })
        .line_height(line_height);
    let pitch = size * line_height;
    let x = rect.x - origin[0];
    let mut baseline = rect.y + rect.height - origin[1] - size * ascent;
    for block in text.split('\n') {
        if block.trim().is_empty() {
            baseline -= pitch;
            continue;
        }
        let lines = page.draw_paragraph(block, x, baseline, rect.width, &style, &paragraph)?;
        baseline -= pitch * lines.len().max(1) as f64;
    }
    Ok(())
}

/// In-place text replacement through the engine's style-preserving reflow
/// transaction (the same request [`wellfriendpdf_engine::TextEditSession`]
/// builds), with the library font as the approved substitute when the
/// embedded font cannot draw the new text.
#[allow(clippy::too_many_arguments)]
fn replace_text(
    input: &[u8],
    page: u32,
    start: usize,
    end: usize,
    expected: &str,
    text: &str,
    fallback_font: Option<&str>,
    fonts: &dyn FontSource,
    fit: crate::overflow::Fit,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let page = page as usize;
    let before = PageTextLayout::from_bytes(input, page)?;
    if end > before.len() || start > end || before.text_in([start, end]) != expected {
        return Err(EditorError::new(
            "EDITOR_TEXT_STALE",
            "the selected text no longer matches the page; reopen the current version",
        ));
    }
    // Text inside a placed form is edited in the form's own content.
    if let Some(form) = crate::forms::target(&before, start, end)? {
        let base = form.range[0];
        let fit = fit.without_next_page();
        return crate::forms::edit_in_form(input, page as u32, &form, report, |bytes, scratch, report| {
            replace_text(bytes, scratch, start - base, end - base, expected, text, fallback_font, fonts, fit, report)
        });
    }
    if !before.editable {
        return Err(EditorError::new(
            "EDITOR_TEXT_NOT_EDITABLE",
            "this page's text cannot be edited in place (its fonts do not decode to exact text)",
        ));
    }
    // A smaller size re-sets the whole paragraph (the in-place transaction
    // keeps the source size).
    if fit.size.is_some() {
        return crate::restyle::reset_replacement(
            input, &before, page as u32, start, end, text, fallback_font, fonts, fit, report,
        );
    }
    // A pure insertion or deletion is widened by one neighbouring scalar that
    // is written back unchanged (the reflow transaction rewrites a non-empty
    // source selection).
    let (selection, replacement, caret) = if start < end && !text.is_empty() {
        ([start, end], text.to_string(), start + text.chars().count())
    } else if start > 0 {
        let kept = before.text_in([start - 1, start]);
        (
            [start - 1, end],
            format!("{kept}{text}"),
            start + text.chars().count(),
        )
    } else if end < before.len() {
        let kept = before.text_in([end, end + 1]);
        (
            [start, end + 1],
            format!("{text}{kept}"),
            start + text.chars().count(),
        )
    } else {
        return Err(EditorError::new(
            "EDITOR_TEXT_STALE",
            "a text edit needs page text to anchor the change",
        ));
    };
    let request: GeometricReflowRequest = serde_json::from_value(json!({
        "page": page,
        "source_text": before.text_in(selection),
        "replacement_text": replacement,
        "target_logical_scalar_range": selection,
    }))
    .map_err(|e| EditorError::new("EDITOR_OPERATION_INVALID", e.to_string()))?;

    let source_fonts = font_resources(input, page as u32);
    let library = fallback_font.and_then(|id| fonts.font_bytes(id).map(|bytes| (id, bytes)));
    let library_font = library.as_ref().map(|(id, _)| id.to_string());

    // 1. Keep the source font (the engine's per-run serializer, or its
    //    generated substitute when the source cannot encode the text).
    let first = attempt(input, &request, page, &before, &source_fonts);
    let accepted = match first {
        Ok(result) if result.fonts_added.is_empty() => Ok((result, None)),
        // 2. A substitute was needed (or the source-font result failed the
        //    layout check): use the library font matching the source family,
        //    so the browser preview and the server replay pick the same face.
        first => match &library {
            Some((id, bytes)) => {
                let mut approved = request.clone();
                approved.font_policy = format!("approved_substitute:{id}");
                approved.approved_font_asset = Some(ApprovedFontAsset {
                    lookup_name: (*id).to_owned(),
                    bytes: bytes.clone(),
                });
                match attempt(input, &approved, page, &before, &source_fonts) {
                    Ok(result) => Ok((result, library_font.clone())),
                    Err(library_error) => first
                        .map(|result| (result, None))
                        .map_err(|_| library_error),
                }
            }
            None => first.map(|result| (result, None)),
        },
    };
    let (result, used_library) = match accepted {
        Ok(accepted) => accepted,
        // A paragraph that grows more than the transaction can reflow in
        // place is re-set as new text; whether it then fits is the overflow
        // policy's to decide (the transaction's refusal stands otherwise).
        Err(error) if text.chars().count() > expected.chars().count() => {
            return crate::restyle::reset_replacement(
                input, &before, page as u32, start, end, text, fallback_font, fonts, fit, report,
            )
            .map_err(|reset| if reset.code == crate::overflow::OVERFLOW { reset } else { error });
        }
        Err(error) => return Err(error),
    };
    if !result.fonts_added.is_empty() {
        report.notes.push(match &used_library {
            Some(id) => format!(
                "The embedded font cannot draw every typed character (it is a subset); the edited text uses {id} from the font library."
            ),
            None => "The embedded font cannot draw every typed character; the engine's substitute font was used.".to_string(),
        });
    }
    report.fonts_added = result.fonts_added;
    report.changed_region = changed_region(&before, &result.layout);
    report.caret = Some(caret);
    // A paragraph that grew moves the text below it in its column down.
    let paragraphs: Vec<usize> = before
        .paragraphs
        .iter()
        .enumerate()
        .filter(|(_, p)| p.range[0] < selection[1].max(selection[0] + 1) && p.range[1] > selection[0])
        .map(|(i, _)| i)
        .collect();
    let placed = crate::restyle::make_room(input, result.bytes, &before, &paragraphs, None, fit.without_next_page(), report);
    match placed {
        // The paragraph is re-set so the words that fit stay and the rest
        // continues on the next page.
        Err(error) if error.code == crate::overflow::OVERFLOW && fit.mode == crate::ops::TextOverflow::NextPage => {
            crate::restyle::reset_replacement(
                input, &before, page as u32, start, end, text, fallback_font, fonts, fit, report,
            )
        }
        other => other,
    }
}

struct Attempt {
    bytes: Vec<u8>,
    layout: PageTextLayout,
    fonts_added: Vec<String>,
}

/// One reflow attempt, accepted only when the edited text reads back exactly
/// and no glyphs on the edited lines overlap (a substitute run that was not
/// re-spaced draws on top of its neighbours).
fn attempt(
    input: &[u8],
    request: &GeometricReflowRequest,
    page: usize,
    before: &PageTextLayout,
    source_fonts: &[String],
) -> Result<Attempt, EditorError> {
    let (bytes, _) = apply_reflow_region(input, request)?;
    let layout = PageTextLayout::from_bytes(&bytes, page)?;
    let [start, end] = request.target_logical_scalar_range.unwrap_or([0, 0]);
    let prefix = before.text_in([0, start]);
    let suffix = before.text_in([end, before.len()]);
    let expected = format!("{prefix}{}{suffix}", request.replacement_text);
    if layout.text != expected {
        return Err(EditorError::new(
            "EDITOR_TEXT_LAYOUT_FAILED",
            "the edited page does not read back as the requested text",
        ));
    }
    if let Some(region) = changed_region(before, &layout) {
        if overlapping_pairs(&layout, region) > overlapping_pairs(before, region) {
            return Err(EditorError::new(
                "EDITOR_TEXT_LAYOUT_FAILED",
                "the edited line would draw glyphs on top of each other",
            ));
        }
    }
    let fonts_added = font_resources(&bytes, page as u32)
        .into_iter()
        .filter(|name| !source_fonts.contains(name))
        .collect();
    Ok(Attempt {
        bytes,
        layout,
        fonts_added,
    })
}

/// Page font resource names (the source fonts before an edit).
fn font_resources(bytes: &[u8], page: u32) -> Vec<String> {
    lopdf::Document::load_mem(bytes)
        .map(|document| {
            crate::fonts::page_fonts(&document, page)
                .into_iter()
                .map(|f| f.resource)
                .collect()
        })
        .unwrap_or_default()
}

/// Consecutive visible glyphs, on the lines crossing `region`, that overlap
/// by more than a third of the narrower one. Compared before and after an
/// edit: kerning and italics overlap a little in the source already.
fn overlapping_pairs(layout: &PageTextLayout, region: [f64; 4]) -> usize {
    let mut count = 0;
    for line in &layout.lines {
        if line.bbox[1] > region[3] || line.bbox[3] < region[1] {
            continue;
        }
        let glyphs: Vec<_> = layout.glyphs[line.glyphs[0]..line.glyphs[1]]
            .iter()
            .filter(|g| !g.text.trim().is_empty() && !g.invisible && !g.vertical)
            .collect();
        for pair in glyphs.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let narrower = (a.bbox[2] - a.bbox[0]).min(b.bbox[2] - b.bbox[0]).max(0.01);
            let overlap = a.bbox[2].min(b.bbox[2]) - a.bbox[0].max(b.bbox[0]);
            if overlap > narrower / 3.0 {
                count += 1;
            }
        }
    }
    count
}

fn changed_region(before: &PageTextLayout, after: &PageTextLayout) -> Option<[f64; 4]> {
    let same = |a: &wellfriendpdf_engine::EditorGlyph, b: &wellfriendpdf_engine::EditorGlyph| {
        a.text == b.text
            && a.bbox
                .iter()
                .zip(b.bbox.iter())
                .all(|(x, y)| (x - y).abs() < 0.01)
    };
    let (a, b) = (&before.glyphs, &after.glyphs);
    let mut head = 0;
    while head < a.len() && head < b.len() && same(&a[head], &b[head]) {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && same(&a[a.len() - 1 - tail], &b[b.len() - 1 - tail])
    {
        tail += 1;
    }
    let changed = a[head..a.len() - tail]
        .iter()
        .chain(b[head..b.len() - tail].iter());
    let mut region: Option<[f64; 4]> = None;
    for glyph in changed {
        region = Some(match region {
            None => glyph.bbox,
            Some(r) => union(r, glyph.bbox),
        });
    }
    region
}

fn redact(
    input: &[u8],
    page: u32,
    rect: &Rect,
    fill: Rgb,
    report: &mut OperationReport,
) -> Result<Vec<u8>, EditorError> {
    let area = bounds(rect);
    let inside = |layout: &PageTextLayout| {
        layout
            .glyphs
            .iter()
            .filter(|g| !g.text.trim().is_empty())
            .filter(|g| {
                g.bbox[0] < area[2]
                    && g.bbox[2] > area[0]
                    && g.bbox[1] < area[3]
                    && g.bbox[3] > area[1]
            })
            .count()
    };
    let before = PageTextLayout::from_bytes(input, page as usize)
        .ok()
        .map(|l| inside(&l));
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    editor.redact(
        page as usize,
        ImageRect::new(rect.x, rect.y, rect.width, rect.height),
        RedactionOptions {
            fill: rgb(&fill),
            ..RedactionOptions::default()
        },
    )?;
    let out = editor.save_to_bytes(EditMode::FullRewrite)?;
    let after = PageTextLayout::from_bytes(&out, page as usize)
        .ok()
        .map(|l| inside(&l));
    if let Some(remaining) = after.filter(|count| *count > 0) {
        return Err(EditorError::new(
            "EDITOR_REDACTION_INCOMPLETE",
            format!("{remaining} glyphs remain under the redaction area; nothing was saved"),
        ));
    }
    report.redacted_glyphs = before;
    report.changed_region = Some(area);
    report
        .notes
        .push("Content under the area was removed from the page, not covered.".into());
    Ok(out)
}
