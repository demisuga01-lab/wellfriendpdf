use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    AnnotationMutation, EditAnnotationRequest, PdfAnnotationKind, PdfAnnotationSpec, PdfBox,
    PdfEditError, PdfPoint, PdfQuadPoints, PdfRgbColor, SignatureEffect, edit_annotation,
    inspect_pdf,
};

fn annotation_fixture(path: &std::path::Path, signed: bool, foreign_annotation: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let content = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => Dictionary::new(),
    });
    if foreign_annotation {
        let annotation = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Square",
            "Rect" => vec![72.into(), 600.into(), 160.into(), 640.into()],
            "P" => page,
        });
        document
            .get_dictionary_mut(page)
            .unwrap()
            .set("Annots", Object::Array(vec![annotation.into()]));
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page.into()],
            "Count" => 1,
        }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    if signed {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(vec![0; 64], lopdf::StringFormat::Hexadecimal),
        });
    }
    document.save(path).unwrap();
}

fn rectangle(left: f64, bottom: f64, right: f64, top: f64) -> PdfBox {
    PdfBox {
        left,
        bottom,
        right,
        top,
    }
}

fn base_spec(kind: PdfAnnotationKind, offset: f64) -> PdfAnnotationSpec {
    let bounds = rectangle(
        72.0 + offset,
        600.0 - offset,
        192.0 + offset,
        650.0 - offset,
    );
    let mut spec = PdfAnnotationSpec {
        kind,
        rectangle: bounds,
        contents: format!("Wellfriend {kind:?} annotation"),
        color: PdfRgbColor {
            red: 0.0,
            green: 0.5,
            blue: 1.0,
        },
        opacity: 0.5,
        quad_points: Vec::new(),
        ink_paths: Vec::new(),
        line_start: None,
        line_end: None,
        stamp_name: None,
    };
    match kind {
        PdfAnnotationKind::Highlight
        | PdfAnnotationKind::Underline
        | PdfAnnotationKind::Strikeout => {
            spec.quad_points = vec![PdfQuadPoints {
                x1: bounds.left + 5.0,
                y1: bounds.top - 5.0,
                x2: bounds.right - 5.0,
                y2: bounds.top - 5.0,
                x3: bounds.left + 5.0,
                y3: bounds.bottom + 5.0,
                x4: bounds.right - 5.0,
                y4: bounds.bottom + 5.0,
            }];
        }
        PdfAnnotationKind::Ink => {
            spec.ink_paths = vec![vec![
                PdfPoint {
                    x: bounds.left + 5.0,
                    y: bounds.bottom + 5.0,
                },
                PdfPoint {
                    x: bounds.left + 40.0,
                    y: bounds.top - 5.0,
                },
                PdfPoint {
                    x: bounds.right - 5.0,
                    y: bounds.bottom + 10.0,
                },
            ]];
        }
        PdfAnnotationKind::Arrow => {
            spec.line_start = Some(PdfPoint {
                x: bounds.left + 5.0,
                y: bounds.bottom + 5.0,
            });
            spec.line_end = Some(PdfPoint {
                x: bounds.right - 5.0,
                y: bounds.top - 5.0,
            });
        }
        PdfAnnotationKind::Stamp => spec.stamp_name = Some("APPROVED".to_string()),
        PdfAnnotationKind::Text | PdfAnnotationKind::Rectangle => {}
    }
    spec
}

#[test]
fn all_required_annotation_kinds_create_real_objects_with_appearance_streams() {
    let directory = tempdir().unwrap();
    let mut input = directory.path().join("input.pdf");
    annotation_fixture(&input, false, false);
    let kinds = [
        PdfAnnotationKind::Text,
        PdfAnnotationKind::Highlight,
        PdfAnnotationKind::Underline,
        PdfAnnotationKind::Strikeout,
        PdfAnnotationKind::Ink,
        PdfAnnotationKind::Rectangle,
        PdfAnnotationKind::Arrow,
        PdfAnnotationKind::Stamp,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let inspection = inspect_pdf(&input).unwrap();
        let output = directory.path().join(format!("created-{index}.pdf"));
        let result = edit_annotation(&EditAnnotationRequest {
            input_path: input,
            output_path: output.clone(),
            mutation: AnnotationMutation::Create {
                page: inspection.editable_annotation_pages[0].selector.clone(),
                spec: base_spec(kind, index as f64 * 3.0),
            },
            allow_signature_invalidation: false,
        })
        .unwrap();
        assert_eq!(result.operation, "EDIT_ANNOTATION");
        assert_eq!(result.capability, "PDF_ANNOTATION_EDIT");
        assert!(result.validation.passed());
        assert!(result.validation.appearance_stream_valid);
        assert!(result.validation.no_page_rasterization);
        input = output;
    }
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_annotations.len(), 8);
    assert_eq!(inspection.unsupported_annotations, 0);
    assert_eq!(inspection.object_graph.annotations, 8);
    let document = Document::load(&input).unwrap();
    for annotation in inspection.editable_annotations {
        let annotation_id = (
            annotation.selector.annotation_object.number,
            annotation.selector.annotation_object.generation,
        );
        let appearance_id = (
            annotation.selector.appearance_object.number,
            annotation.selector.appearance_object.generation,
        );
        assert!(document.get_dictionary(annotation_id).is_ok());
        assert!(
            document
                .get_object(appearance_id)
                .and_then(Object::as_stream)
                .is_ok()
        );
    }
}

#[test]
fn owned_annotations_update_delete_and_stale_or_invalid_edits_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let created = directory.path().join("created.pdf");
    let updated = directory.path().join("updated.pdf");
    let deleted = directory.path().join("deleted.pdf");
    annotation_fixture(&input, false, false);
    let source = inspect_pdf(&input).unwrap();
    edit_annotation(&EditAnnotationRequest {
        input_path: input,
        output_path: created.clone(),
        mutation: AnnotationMutation::Create {
            page: source.editable_annotation_pages[0].selector.clone(),
            spec: base_spec(PdfAnnotationKind::Rectangle, 0.0),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    let created_annotation = inspect_pdf(&created)
        .unwrap()
        .editable_annotations
        .remove(0);
    let mut replacement = base_spec(PdfAnnotationKind::Stamp, 12.0);
    replacement.contents = "Approved by native annotation test".to_string();
    let result = edit_annotation(&EditAnnotationRequest {
        input_path: created.clone(),
        output_path: updated.clone(),
        mutation: AnnotationMutation::Update {
            selector: Box::new(created_annotation.selector.clone()),
            spec: replacement.clone(),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.result_spec, Some(replacement));
    assert!(result.validation.passed());
    let updated_annotation = inspect_pdf(&updated)
        .unwrap()
        .editable_annotations
        .remove(0);

    let mut stale = updated_annotation.selector.clone();
    stale.expected_annotation_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_annotation(&EditAnnotationRequest {
            input_path: updated.clone(),
            output_path: directory.path().join("stale.pdf"),
            mutation: AnnotationMutation::Delete {
                selector: Box::new(stale),
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleAnnotation)
    ));

    let mut invalid = updated_annotation.spec.clone();
    invalid.opacity = 0.0;
    assert!(matches!(
        edit_annotation(&EditAnnotationRequest {
            input_path: updated.clone(),
            output_path: directory.path().join("invalid.pdf"),
            mutation: AnnotationMutation::Update {
                selector: Box::new(updated_annotation.selector.clone()),
                spec: invalid,
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidAnnotationEdit)
    ));

    let deleted_result = edit_annotation(&EditAnnotationRequest {
        input_path: updated,
        output_path: deleted.clone(),
        mutation: AnnotationMutation::Delete {
            selector: Box::new(updated_annotation.selector),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(deleted_result.validation.passed());
    let deleted_inspection = inspect_pdf(&deleted).unwrap();
    assert_eq!(deleted_inspection.editable_annotations.len(), 0);
    assert_eq!(deleted_inspection.object_graph.annotations, 0);
}

#[test]
fn contents_only_update_preserves_an_unchanged_appearance_stream() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let created = directory.path().join("created.pdf");
    let updated = directory.path().join("updated.pdf");
    annotation_fixture(&input, false, false);
    let source = inspect_pdf(&input).unwrap();
    edit_annotation(&EditAnnotationRequest {
        input_path: input,
        output_path: created.clone(),
        mutation: AnnotationMutation::Create {
            page: source.editable_annotation_pages[0].selector.clone(),
            spec: base_spec(PdfAnnotationKind::Text, 0.0),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    let created_annotation = inspect_pdf(&created)
        .unwrap()
        .editable_annotations
        .remove(0);
    let mut replacement = created_annotation.spec.clone();
    replacement.contents = "Updated accessible annotation contents".to_string();
    let result = edit_annotation(&EditAnnotationRequest {
        input_path: created,
        output_path: updated.clone(),
        mutation: AnnotationMutation::Update {
            selector: Box::new(created_annotation.selector.clone()),
            spec: replacement.clone(),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.changed_objects.len(), 1);
    assert_eq!(
        result.changed_objects[0],
        created_annotation.selector.annotation_object
    );
    assert_eq!(result.result_spec, Some(replacement.clone()));
    assert!(result.validation.passed());
    let updated_annotation = inspect_pdf(&updated)
        .unwrap()
        .editable_annotations
        .remove(0);
    assert_eq!(updated_annotation.spec, replacement);
    assert_eq!(
        updated_annotation.selector.expected_appearance_sha256,
        created_annotation.selector.expected_appearance_sha256
    );
}

#[test]
fn foreign_annotations_are_read_only_but_do_not_block_safe_owned_creation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("foreign.pdf");
    let output = directory.path().join("owned.pdf");
    annotation_fixture(&input, false, true);
    let inspection = inspect_pdf(&input).unwrap();
    assert!(inspection.editable_annotations.is_empty());
    assert_eq!(inspection.unsupported_annotations, 1);
    edit_annotation(&EditAnnotationRequest {
        input_path: input,
        output_path: output.clone(),
        mutation: AnnotationMutation::Create {
            page: inspection.editable_annotation_pages[0].selector.clone(),
            spec: base_spec(PdfAnnotationKind::Text, 0.0),
        },
        allow_signature_invalidation: false,
    })
    .unwrap();
    let result = inspect_pdf(&output).unwrap();
    assert_eq!(result.editable_annotations.len(), 1);
    assert_eq!(result.unsupported_annotations, 1);
    assert_eq!(result.object_graph.annotations, 2);
}

#[test]
fn signed_annotation_edits_require_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    annotation_fixture(&input, true, false);
    let inspection = inspect_pdf(&input).unwrap();
    let request = EditAnnotationRequest {
        input_path: input.clone(),
        output_path: directory.path().join("unconfirmed.pdf"),
        mutation: AnnotationMutation::Create {
            page: inspection.editable_annotation_pages[0].selector.clone(),
            spec: base_spec(PdfAnnotationKind::Highlight, 0.0),
        },
        allow_signature_invalidation: false,
    };
    assert!(matches!(
        edit_annotation(&request),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_annotation(&EditAnnotationRequest {
        output_path: directory.path().join("confirmed.pdf"),
        allow_signature_invalidation: true,
        ..request
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
