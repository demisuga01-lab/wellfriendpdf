use lopdf::{Dictionary, Document, Object, Stream, StringFormat, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    CreateFormFieldRequest, CreateFormFieldWidget, DeleteFormFieldRequest,
    EditFormFieldGeometryRequest, EditFormFieldLabelRequest, EditFormFieldOptionsRequest,
    EditFormFieldValueRequest, PageIdentitySelector, PdfBox, PdfEditError, PdfFormFieldKind,
    RepairFormFieldAppearanceRequest, SignatureEffect, create_form_field, delete_form_field,
    edit_form_field_geometry, edit_form_field_label, edit_form_field_options,
    edit_form_field_value, inspect_pdf, repair_form_field_appearance,
};

fn empty_appearance(document: &mut Document) -> lopdf::ObjectId {
    document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 120.into(), 24.into()],
            "Resources" => Dictionary::new(),
        },
        Vec::new(),
    ))
}

fn button_appearance(
    document: &mut Document,
    on_name: &str,
) -> (Dictionary, lopdf::ObjectId, lopdf::ObjectId) {
    let off = empty_appearance(document);
    let on = empty_appearance(document);
    let mut normal = Dictionary::new();
    normal.set("Off", off);
    normal.set(on_name, on);
    let mut appearance = Dictionary::new();
    appearance.set("N", normal);
    (appearance, off, on)
}

fn form_fixture(path: &std::path::Path, signed: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let content = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let page_id = document.new_object_id();

    let text_appearance = empty_appearance(&mut document);
    let text = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("customer_name"),
        "TU" => Object::string_literal("Customer name"),
        "V" => Object::string_literal("Old value"),
        "Rect" => vec![72.into(), 680.into(), 280.into(), 710.into()],
        "P" => page_id,
        "AP" => dictionary! { "N" => text_appearance },
    });

    let dropdown_appearance = empty_appearance(&mut document);
    let dropdown = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Ch",
        "Ff" => (1_i64 << 17),
        "T" => Object::string_literal("region"),
        "V" => Object::string_literal("North"),
        "Opt" => vec![Object::string_literal("North"), Object::string_literal("South")],
        "Rect" => vec![72.into(), 630.into(), 280.into(), 660.into()],
        "P" => page_id,
        "AP" => dictionary! { "N" => dropdown_appearance },
    });

    let (checkbox_appearance, _, _) = button_appearance(&mut document, "Yes");
    let checkbox = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Btn",
        "T" => Object::string_literal("approved"),
        "V" => Object::Name(b"Off".to_vec()),
        "AS" => Object::Name(b"Off".to_vec()),
        "Rect" => vec![72.into(), 580.into(), 96.into(), 604.into()],
        "P" => page_id,
        "AP" => checkbox_appearance,
    });

    let radio_parent = document.new_object_id();
    let (radio_one_appearance, _, _) = button_appearance(&mut document, "One");
    let radio_one = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Parent" => radio_parent,
        "AS" => Object::Name(b"One".to_vec()),
        "Rect" => vec![72.into(), 530.into(), 96.into(), 554.into()],
        "P" => page_id,
        "AP" => radio_one_appearance,
    });
    let (radio_two_appearance, _, _) = button_appearance(&mut document, "Two");
    let radio_two = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Parent" => radio_parent,
        "AS" => Object::Name(b"Off".to_vec()),
        "Rect" => vec![120.into(), 530.into(), 144.into(), 554.into()],
        "P" => page_id,
        "AP" => radio_two_appearance,
    });
    document.objects.insert(
        radio_parent,
        Object::Dictionary(dictionary! {
            "FT" => "Btn",
            "Ff" => (1_i64 << 15),
            "T" => Object::string_literal("plan"),
            "V" => Object::Name(b"One".to_vec()),
            "Kids" => vec![radio_one.into(), radio_two.into()],
        }),
    );

    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => Dictionary::new(),
            "Annots" => vec![text.into(), dropdown.into(), checkbox.into(), radio_one.into(), radio_two.into()],
        }),
    );
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let acro_form = document.add_object(dictionary! {
        "Fields" => vec![text.into(), dropdown.into(), checkbox.into(), radio_parent.into()],
        "NeedAppearances" => false,
    });
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "AcroForm" => acro_form,
    });
    document.trailer.set("Root", catalog);
    if signed {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(vec![0; 64], StringFormat::Hexadecimal),
        });
    }
    document.save(path).unwrap();
}

fn nested_form_fixture(path: &std::path::Path) {
    form_fixture(path, false);
    let selected = field(path, "customer_name");
    let widget_id = (
        selected.selector.field_object.number,
        selected.selector.field_object.generation,
    );
    let mut document = Document::load(path).unwrap();
    let acro_form_id = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .and_then(|catalog_id| {
            document
                .get_dictionary(catalog_id)
                .unwrap()
                .get(b"AcroForm")
                .unwrap()
                .as_reference()
        })
        .unwrap();
    let child_id = document.new_object_id();
    let parent_id = document.new_object_id();
    {
        let widget = document.get_dictionary_mut(widget_id).unwrap();
        for key in [b"FT".as_slice(), b"T", b"TU", b"V"] {
            widget.remove(key);
        }
        widget.set("Parent", child_id);
    }
    document.objects.insert(
        child_id,
        Object::Dictionary(dictionary! {
            "FT" => "Tx",
            "T" => Object::string_literal("name"),
            "TU" => Object::string_literal("Customer name"),
            "V" => Object::string_literal("Old value"),
            "Parent" => parent_id,
            "Kids" => vec![widget_id.into()],
        }),
    );
    document.objects.insert(
        parent_id,
        Object::Dictionary(dictionary! {
            "T" => Object::string_literal("customer"),
            "Kids" => vec![child_id.into()],
        }),
    );
    let form = document.get_dictionary_mut(acro_form_id).unwrap();
    let fields = form.get_mut(b"Fields").unwrap().as_array_mut().unwrap();
    let index = fields
        .iter()
        .position(|entry| entry.as_reference().ok() == Some(widget_id))
        .unwrap();
    fields[index] = parent_id.into();
    document.save(path).unwrap();
}

fn field(path: &std::path::Path, name: &str) -> wellfriend_pdf_edit::EditableFormField {
    inspect_pdf(path)
        .unwrap()
        .editable_form_fields
        .into_iter()
        .find(|field| field.name == name)
        .unwrap()
}

fn creation_widget(
    path: &std::path::Path,
    rectangle: PdfBox,
    export_value: Option<&str>,
) -> CreateFormFieldWidget {
    let page = inspect_pdf(path).unwrap().editable_pages.remove(0).selector;
    CreateFormFieldWidget {
        page: PageIdentitySelector {
            page_number: page.page_number,
            page_object: page.page_object,
        },
        rectangle,
        export_value: export_value.map(str::to_string),
    }
}

#[test]
fn text_choice_and_button_values_edit_actual_acroform_objects() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.editable_form_fields.len(), 4);
    assert_eq!(inspection.unsupported_form_fields, 0);
    assert_eq!(inspection.object_graph.acro_form_fields, 4);

    for (name, expected_kind, requested) in [
        ("customer_name", PdfFormFieldKind::Text, "Updated customer"),
        ("region", PdfFormFieldKind::Dropdown, "South"),
        ("approved", PdfFormFieldKind::Checkbox, "Yes"),
        ("plan", PdfFormFieldKind::Radio, "Two"),
    ] {
        let output = directory.path().join(format!("{name}.pdf"));
        let source_field = field(&input, name);
        assert_eq!(source_field.kind, expected_kind);
        let result = edit_form_field_value(&EditFormFieldValueRequest {
            input_path: input.clone(),
            output_path: output.clone(),
            selector: source_field.selector,
            value: Some(requested.to_string()),
            allow_signature_invalidation: false,
        })
        .unwrap();
        assert_eq!(result.operation, "EDIT_FORM_FIELD_VALUE");
        assert_eq!(result.capability, "PDF_FORM_EDIT");
        assert_eq!(result.result_value.as_deref(), Some(requested));
        assert!(result.validation.passed());
        assert!(result.validation.no_page_rasterization);
        let output_field = field(&output, name);
        assert_eq!(output_field.value.as_deref(), Some(requested));
        assert_eq!(output_field.kind, expected_kind);
    }
}

#[test]
fn stale_invalid_and_signature_field_values_fail_closed() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let mut stale = field(&input, "customer_name").selector;
    stale.expected_field_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_form_field_value(&EditFormFieldValueRequest {
            input_path: input.clone(),
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            value: Some("Changed".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    assert!(matches!(
        edit_form_field_value(&EditFormFieldValueRequest {
            input_path: input.clone(),
            output_path: directory.path().join("invalid-choice.pdf"),
            selector: field(&input, "region").selector,
            value: Some("Unknown".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldValue)
    ));
    assert!(matches!(
        edit_form_field_value(&EditFormFieldValueRequest {
            input_path: input.clone(),
            output_path: directory.path().join("unicode.pdf"),
            selector: field(&input, "customer_name").selector,
            value: Some("Résumé".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldValue)
    ));
}

#[test]
fn signed_form_edits_require_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    form_fixture(&input, true);
    let selector = field(&input, "approved").selector;
    assert!(matches!(
        edit_form_field_value(&EditFormFieldValueRequest {
            input_path: input.clone(),
            output_path: directory.path().join("unconfirmed.pdf"),
            selector: selector.clone(),
            value: Some("Yes".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_form_field_value(&EditFormFieldValueRequest {
        input_path: input,
        output_path: directory.path().join("confirmed.pdf"),
        selector,
        value: Some("Yes".to_string()),
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn form_widget_geometry_edits_the_real_rect_and_regenerates_text_appearance() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("geometry.pdf");
    form_fixture(&input, false);
    let source = field(&input, "customer_name");
    assert_eq!(source.widgets.len(), 1);
    let widget = source.widgets[0].clone();
    let rectangle = PdfBox {
        left: 84.0,
        bottom: 670.0,
        right: 320.0,
        top: 706.0,
    };
    let result = edit_form_field_geometry(&EditFormFieldGeometryRequest {
        input_path: input,
        output_path: output.clone(),
        selector: source.selector,
        widget: widget.selector,
        rectangle,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.operation, "EDIT_FORM_FIELD_GEOMETRY");
    assert_eq!(result.capability, "PDF_FORM_EDIT");
    assert_eq!(result.source_rectangle, widget.rectangle);
    assert_eq!(result.result_rectangle, rectangle);
    assert!(result.validation.passed());
    let edited = field(&output, "customer_name");
    assert_eq!(edited.widgets[0].rectangle, rectangle);
    assert_eq!(edited.value.as_deref(), Some("Old value"));
}

#[test]
fn form_widget_geometry_is_stale_bound_page_bounded_and_signature_aware() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let radio_field = field(&input, "plan");
    let mut stale_widget = radio_field.widgets[1].selector.clone();
    stale_widget.expected_rectangle.left += 1.0;
    assert!(matches!(
        edit_form_field_geometry(&EditFormFieldGeometryRequest {
            input_path: input.clone(),
            output_path: directory.path().join("stale-geometry.pdf"),
            selector: radio_field.selector.clone(),
            widget: stale_widget,
            rectangle: PdfBox {
                left: 130.0,
                bottom: 520.0,
                right: 158.0,
                top: 548.0
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    assert!(matches!(
        edit_form_field_geometry(&EditFormFieldGeometryRequest {
            input_path: input.clone(),
            output_path: directory.path().join("outside-page.pdf"),
            selector: radio_field.selector,
            widget: radio_field.widgets[1].selector.clone(),
            rectangle: PdfBox {
                left: 600.0,
                bottom: 520.0,
                right: 640.0,
                top: 548.0
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldGeometry)
    ));

    let signed = directory.path().join("signed.pdf");
    form_fixture(&signed, true);
    let signed_field = field(&signed, "approved");
    assert!(matches!(
        edit_form_field_geometry(&EditFormFieldGeometryRequest {
            input_path: signed.clone(),
            output_path: directory.path().join("signed-unconfirmed.pdf"),
            selector: signed_field.selector.clone(),
            widget: signed_field.widgets[0].selector.clone(),
            rectangle: PdfBox {
                left: 78.0,
                bottom: 575.0,
                right: 108.0,
                top: 605.0
            },
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_form_field_geometry(&EditFormFieldGeometryRequest {
        input_path: signed,
        output_path: directory.path().join("signed-confirmed.pdf"),
        selector: signed_field.selector,
        widget: signed_field.widgets[0].selector.clone(),
        rectangle: PdfBox {
            left: 78.0,
            bottom: 575.0,
            right: 108.0,
            top: 605.0,
        },
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn form_field_label_edits_the_real_alternate_name_without_changing_field_state() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("label.pdf");
    form_fixture(&input, false);
    let source = field(&input, "customer_name");
    let result = edit_form_field_label(&EditFormFieldLabelRequest {
        input_path: input,
        output_path: output.clone(),
        selector: source.selector,
        label: Some("Primary customer name".to_string()),
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.operation, "EDIT_FORM_FIELD_LABEL");
    assert_eq!(result.capability, "PDF_FORM_EDIT");
    assert_eq!(result.source_label.as_deref(), Some("Customer name"));
    assert_eq!(
        result.result_label.as_deref(),
        Some("Primary customer name")
    );
    assert!(result.validation.passed());
    assert!(result.validation.widget_objects_preserved);
    let edited = field(&output, "customer_name");
    assert_eq!(edited.label.as_deref(), Some("Primary customer name"));
    assert_eq!(edited.value.as_deref(), Some("Old value"));
    assert_eq!(edited.rectangle, source.rectangle);
}

#[test]
fn form_field_label_is_stale_bound_validated_clearable_and_signature_aware() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let source = field(&input, "customer_name");
    assert!(matches!(
        edit_form_field_label(&EditFormFieldLabelRequest {
            input_path: input.clone(),
            output_path: directory.path().join("no-op.pdf"),
            selector: source.selector.clone(),
            label: source.label.clone(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldLabel)
    ));
    assert!(matches!(
        edit_form_field_label(&EditFormFieldLabelRequest {
            input_path: input.clone(),
            output_path: directory.path().join("invalid.pdf"),
            selector: source.selector.clone(),
            label: Some(" padded ".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldLabel)
    ));
    let mut stale = source.selector.clone();
    stale.expected_field_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_form_field_label(&EditFormFieldLabelRequest {
            input_path: input.clone(),
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            label: None,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    let cleared_path = directory.path().join("cleared.pdf");
    edit_form_field_label(&EditFormFieldLabelRequest {
        input_path: input,
        output_path: cleared_path.clone(),
        selector: source.selector,
        label: None,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(field(&cleared_path, "customer_name").label, None);

    let signed = directory.path().join("signed.pdf");
    form_fixture(&signed, true);
    let signed_field = field(&signed, "approved");
    assert!(matches!(
        edit_form_field_label(&EditFormFieldLabelRequest {
            input_path: signed.clone(),
            output_path: directory.path().join("signed-unconfirmed.pdf"),
            selector: signed_field.selector.clone(),
            label: Some("Approval decision".to_string()),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_form_field_label(&EditFormFieldLabelRequest {
        input_path: signed,
        output_path: directory.path().join("signed-confirmed.pdf"),
        selector: signed_field.selector,
        label: Some("Approval decision".to_string()),
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn dropdown_options_edit_the_real_option_array_value_and_appearance() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("options.pdf");
    form_fixture(&input, false);
    let source = field(&input, "region");
    assert!(source.options_editable);
    let result = edit_form_field_options(&EditFormFieldOptionsRequest {
        input_path: input,
        output_path: output.clone(),
        selector: source.selector,
        options: vec!["West".to_string(), "North".to_string(), "East".to_string()],
        value: "East".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.operation, "EDIT_FORM_FIELD_OPTIONS");
    assert_eq!(result.capability, "PDF_FORM_EDIT");
    assert_eq!(result.source_options, vec!["North", "South"]);
    assert_eq!(result.result_options, vec!["West", "North", "East"]);
    assert_eq!(result.source_value.as_deref(), Some("North"));
    assert_eq!(result.result_value, "East");
    assert!(result.validation.passed());
    let edited = field(&output, "region");
    assert_eq!(edited.options, vec!["West", "North", "East"]);
    assert_eq!(edited.value.as_deref(), Some("East"));
    assert!(edited.options_editable);
}

#[test]
fn dropdown_option_edits_are_stale_bound_strict_and_signature_aware() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let source = field(&input, "region");
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input.clone(),
            output_path: directory.path().join("no-op.pdf"),
            selector: source.selector.clone(),
            options: source.options.clone(),
            value: source.value.clone().unwrap(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldOptions)
    ));
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input.clone(),
            output_path: directory.path().join("duplicate.pdf"),
            selector: source.selector.clone(),
            options: vec!["North".to_string(), "North".to_string()],
            value: "North".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldOptions)
    ));
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input.clone(),
            output_path: directory.path().join("missing-selection.pdf"),
            selector: source.selector.clone(),
            options: vec!["East".to_string(), "West".to_string()],
            value: "North".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldOptions)
    ));
    let mut stale = source.selector.clone();
    stale.expected_field_sha256 = "0".repeat(64);
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input.clone(),
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            options: vec!["East".to_string(), "West".to_string()],
            value: "East".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input,
            output_path: directory.path().join("text.pdf"),
            selector: field(&directory.path().join("input.pdf"), "customer_name").selector,
            options: vec!["One".to_string(), "Two".to_string()],
            value: "One".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::FormFieldUnsupported)
    ));

    let signed = directory.path().join("signed.pdf");
    form_fixture(&signed, true);
    let signed_field = field(&signed, "region");
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: signed.clone(),
            output_path: directory.path().join("signed-unconfirmed.pdf"),
            selector: signed_field.selector.clone(),
            options: vec!["East".to_string(), "West".to_string()],
            value: "East".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = edit_form_field_options(&EditFormFieldOptionsRequest {
        input_path: signed,
        output_path: directory.path().join("signed-confirmed.pdf"),
        selector: signed_field.selector,
        options: vec!["East".to_string(), "West".to_string()],
        value: "East".to_string(),
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn export_display_option_pairs_remain_read_only_instead_of_being_flattened() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("paired.pdf");
    form_fixture(&input, false);
    let selected = field(&input, "region");
    let field_id = (
        selected.selector.field_object.number,
        selected.selector.field_object.generation,
    );
    let mut document = Document::load(&input).unwrap();
    document.get_dictionary_mut(field_id).unwrap().set(
        "Opt",
        Object::Array(vec![
            Object::Array(vec![
                Object::string_literal("N"),
                Object::string_literal("North"),
            ]),
            Object::Array(vec![
                Object::string_literal("S"),
                Object::string_literal("South"),
            ]),
        ]),
    );
    document
        .get_dictionary_mut(field_id)
        .unwrap()
        .set("V", Object::string_literal("N"));
    document.save(&input).unwrap();
    let paired = field(&input, "region");
    assert_eq!(paired.options, vec!["N", "S"]);
    assert!(!paired.options_editable);
    assert!(matches!(
        edit_form_field_options(&EditFormFieldOptionsRequest {
            input_path: input,
            output_path: directory.path().join("paired-output.pdf"),
            selector: paired.selector,
            options: vec!["East".to_string(), "West".to_string()],
            value: "East".to_string(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::FormFieldUnsupported)
    ));
}

#[test]
fn form_appearance_repair_regenerates_text_and_normalizes_button_state() {
    let directory = tempdir().unwrap();
    let text_input = directory.path().join("broken-text.pdf");
    let text_output = directory.path().join("repaired-text.pdf");
    form_fixture(&text_input, false);
    let text_field = field(&text_input, "customer_name");
    let text_id = (
        text_field.selector.field_object.number,
        text_field.selector.field_object.generation,
    );
    let mut broken_text = Document::load(&text_input).unwrap();
    broken_text
        .get_dictionary_mut(text_id)
        .unwrap()
        .remove(b"AP");
    let acro_form_id = (
        text_field.selector.form.acro_form_object.number,
        text_field.selector.form.acro_form_object.generation,
    );
    broken_text
        .get_dictionary_mut(acro_form_id)
        .unwrap()
        .set("NeedAppearances", true);
    broken_text.save(&text_input).unwrap();

    let broken_field = field(&text_input, "customer_name");
    assert!(!broken_field.appearance_valid);
    assert!(broken_field.appearance_repairable);
    let repaired = repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
        input_path: text_input.clone(),
        output_path: text_output.clone(),
        selector: broken_field.selector,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(repaired.operation, "REPAIR_FORM_FIELD_APPEARANCE");
    assert_eq!(repaired.field_value.as_deref(), Some("Old value"));
    assert!(repaired.validation.passed());
    assert!(repaired.validation.source_appearance_invalid);
    assert!(repaired.validation.requested_appearance_repaired);
    let repaired_field = field(&text_output, "customer_name");
    assert!(repaired_field.appearance_valid);
    assert!(!repaired_field.appearance_repairable);

    let button_input = directory.path().join("broken-button.pdf");
    let button_output = directory.path().join("repaired-button.pdf");
    form_fixture(&button_input, false);
    let checkbox = field(&button_input, "approved");
    let checkbox_id = (
        checkbox.selector.field_object.number,
        checkbox.selector.field_object.generation,
    );
    let mut broken_button = Document::load(&button_input).unwrap();
    broken_button
        .get_dictionary_mut(checkbox_id)
        .unwrap()
        .set("AS", Object::Name(b"Yes".to_vec()));
    broken_button.save(&button_input).unwrap();
    let broken_checkbox = field(&button_input, "approved");
    assert!(!broken_checkbox.appearance_valid);
    assert!(broken_checkbox.appearance_repairable);
    let repaired_button = repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
        input_path: button_input.clone(),
        output_path: button_output.clone(),
        selector: broken_checkbox.selector,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(repaired_button.validation.passed());
    assert!(field(&button_output, "approved").appearance_valid);
}

#[test]
fn form_appearance_repair_is_stale_bound_no_op_safe_and_signature_aware() {
    let directory = tempdir().unwrap();
    let valid_input = directory.path().join("valid.pdf");
    form_fixture(&valid_input, false);
    let valid_field = field(&valid_input, "customer_name");
    assert!(valid_field.appearance_valid);
    assert!(!valid_field.appearance_repairable);
    assert!(matches!(
        repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
            input_path: valid_input.clone(),
            output_path: directory.path().join("no-op.pdf"),
            selector: valid_field.selector,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::FormFieldAppearanceNotRepairable)
    ));

    let signed_input = directory.path().join("signed-broken.pdf");
    form_fixture(&signed_input, true);
    let original = field(&signed_input, "customer_name");
    let field_id = (
        original.selector.field_object.number,
        original.selector.field_object.generation,
    );
    let mut signed = Document::load(&signed_input).unwrap();
    signed.get_dictionary_mut(field_id).unwrap().remove(b"AP");
    signed.save(&signed_input).unwrap();
    let repairable = field(&signed_input, "customer_name");
    let mut stale = repairable.selector.clone();
    stale.expected_field_sha256 = "0".repeat(64);
    assert!(matches!(
        repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
            input_path: signed_input.clone(),
            output_path: directory.path().join("stale-refused.pdf"),
            selector: stale,
            allow_signature_invalidation: true,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    assert!(matches!(
        repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
            input_path: signed_input.clone(),
            output_path: directory.path().join("signed-refused.pdf"),
            selector: repairable.selector.clone(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let result = repair_form_field_appearance(&RepairFormFieldAppearanceRequest {
        input_path: signed_input,
        output_path: directory.path().join("signed-repaired.pdf"),
        selector: repairable.selector,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
    assert!(result.validation.passed());
}

#[test]
fn creates_text_checkbox_radio_dropdown_and_signature_fields_as_real_acroform_objects() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let cases = [
        (
            "created_text",
            PdfFormFieldKind::Text,
            Some("Draft".to_string()),
            Vec::new(),
            vec![creation_widget(
                &input,
                PdfBox {
                    left: 320.0,
                    bottom: 680.0,
                    right: 520.0,
                    top: 710.0,
                },
                None,
            )],
        ),
        (
            "created_checkbox",
            PdfFormFieldKind::Checkbox,
            Some("Accepted".to_string()),
            vec!["Accepted".to_string()],
            vec![creation_widget(
                &input,
                PdfBox {
                    left: 320.0,
                    bottom: 630.0,
                    right: 344.0,
                    top: 654.0,
                },
                Some("Accepted"),
            )],
        ),
        (
            "created_radio",
            PdfFormFieldKind::Radio,
            Some("Standard".to_string()),
            vec!["Premium".to_string(), "Standard".to_string()],
            vec![
                creation_widget(
                    &input,
                    PdfBox {
                        left: 320.0,
                        bottom: 580.0,
                        right: 344.0,
                        top: 604.0,
                    },
                    Some("Premium"),
                ),
                creation_widget(
                    &input,
                    PdfBox {
                        left: 368.0,
                        bottom: 580.0,
                        right: 392.0,
                        top: 604.0,
                    },
                    Some("Standard"),
                ),
            ],
        ),
        (
            "created_dropdown",
            PdfFormFieldKind::Dropdown,
            Some("EUR".to_string()),
            vec!["EUR".to_string(), "USD".to_string()],
            vec![creation_widget(
                &input,
                PdfBox {
                    left: 320.0,
                    bottom: 530.0,
                    right: 520.0,
                    top: 560.0,
                },
                None,
            )],
        ),
        (
            "created_signature",
            PdfFormFieldKind::Signature,
            None,
            Vec::new(),
            vec![creation_widget(
                &input,
                PdfBox {
                    left: 320.0,
                    bottom: 460.0,
                    right: 520.0,
                    top: 510.0,
                },
                None,
            )],
        ),
    ];
    for (name, kind, value, options, widgets) in cases {
        let output = directory.path().join(format!("{name}.pdf"));
        let inspection = inspect_pdf(&input).unwrap();
        let result = create_form_field(&CreateFormFieldRequest {
            input_path: input.clone(),
            output_path: output.clone(),
            selector: inspection.form_creation.unwrap(),
            name: name.to_string(),
            label: Some(format!("Label for {name}")),
            kind,
            value: value.clone(),
            options: options.clone(),
            widgets,
            required: true,
            allow_signature_invalidation: false,
        })
        .unwrap();
        assert_eq!(result.operation, "CREATE_FORM_FIELD");
        assert_eq!(result.field_kind, kind);
        assert!(result.validation.passed());
        let created = field(&output, name);
        assert_eq!(created.kind, kind);
        assert_eq!(created.value, result.result_value);
        assert_eq!(created.options, options);
        assert!(created.required);
        assert!(!created.read_only);
    }
}

#[test]
fn form_field_creation_supports_missing_acroform_and_rejects_stale_invalid_or_unsigned_authority() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let mut without_form = Document::load(&input).unwrap();
    let catalog_id = without_form
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    without_form
        .get_dictionary_mut(catalog_id)
        .unwrap()
        .remove(b"AcroForm");
    let plain = directory.path().join("plain.pdf");
    without_form.save(&plain).unwrap();
    let inspection = inspect_pdf(&plain).unwrap();
    assert!(inspection.editable_form_fields.is_empty());
    let selector = inspection.form_creation.unwrap();
    let output = directory.path().join("created-on-plain.pdf");
    create_form_field(&CreateFormFieldRequest {
        input_path: plain.clone(),
        output_path: output.clone(),
        selector: selector.clone(),
        name: "new_field".to_string(),
        label: None,
        kind: PdfFormFieldKind::Text,
        value: Some("New value".to_string()),
        options: Vec::new(),
        widgets: vec![creation_widget(
            &plain,
            PdfBox {
                left: 320.0,
                bottom: 680.0,
                right: 520.0,
                top: 710.0,
            },
            None,
        )],
        required: false,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(
        field(&output, "new_field").value.as_deref(),
        Some("New value")
    );

    let mut stale = selector.clone();
    stale.catalog_object.number += 1;
    assert!(matches!(
        create_form_field(&CreateFormFieldRequest {
            input_path: plain.clone(),
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            name: "stale".to_string(),
            label: None,
            kind: PdfFormFieldKind::Text,
            value: None,
            options: Vec::new(),
            widgets: vec![creation_widget(
                &plain,
                PdfBox {
                    left: 320.0,
                    bottom: 630.0,
                    right: 520.0,
                    top: 660.0
                },
                None,
            )],
            required: false,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));
    assert!(matches!(
        create_form_field(&CreateFormFieldRequest {
            input_path: plain,
            output_path: directory.path().join("invalid-radio.pdf"),
            selector,
            name: "invalid.radio".to_string(),
            label: None,
            kind: PdfFormFieldKind::Radio,
            value: None,
            options: vec!["Only".to_string()],
            widgets: vec![creation_widget(
                &input,
                PdfBox {
                    left: 320.0,
                    bottom: 630.0,
                    right: 344.0,
                    top: 654.0
                },
                Some("Only"),
            )],
            required: false,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::InvalidFormFieldCreation)
    ));

    let signed = directory.path().join("signed.pdf");
    form_fixture(&signed, true);
    let signed_inspection = inspect_pdf(&signed).unwrap();
    assert!(matches!(
        create_form_field(&CreateFormFieldRequest {
            input_path: signed.clone(),
            output_path: directory.path().join("signed-unconfirmed.pdf"),
            selector: signed_inspection.form_creation.unwrap(),
            name: "signed_new".to_string(),
            label: None,
            kind: PdfFormFieldKind::Signature,
            value: None,
            options: Vec::new(),
            widgets: vec![creation_widget(
                &signed,
                PdfBox {
                    left: 320.0,
                    bottom: 460.0,
                    right: 520.0,
                    top: 510.0
                },
                None,
            )],
            required: false,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
}

#[test]
fn deletes_top_level_and_nested_fields_from_the_real_tree_and_page_annotations() {
    let directory = tempdir().unwrap();
    let top_level = directory.path().join("top-level.pdf");
    form_fixture(&top_level, false);
    let top_field = field(&top_level, "approved");
    let top_output = directory.path().join("top-level-deleted.pdf");
    let top_result = delete_form_field(&DeleteFormFieldRequest {
        input_path: top_level,
        output_path: top_output.clone(),
        selector: top_field.selector,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(top_result.operation, "DELETE_FORM_FIELD");
    assert_eq!(top_result.capability, "PDF_FORM_EDIT");
    assert!(top_result.validation.passed());
    assert_eq!(
        inspect_pdf(&top_output).unwrap().editable_form_fields.len(),
        3
    );
    assert!(
        inspect_pdf(&top_output)
            .unwrap()
            .editable_form_fields
            .iter()
            .all(|candidate| candidate.name != "approved")
    );

    let nested = directory.path().join("nested.pdf");
    nested_form_fixture(&nested);
    let nested_field = field(&nested, "customer.name");
    let nested_output = directory.path().join("nested-deleted.pdf");
    let nested_result = delete_form_field(&DeleteFormFieldRequest {
        input_path: nested,
        output_path: nested_output.clone(),
        selector: nested_field.selector,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert!(nested_result.validation.passed());
    assert_eq!(nested_result.removed_tree_objects.len(), 2);
    assert_eq!(nested_result.widget_objects.len(), 1);
    let nested_inspection = inspect_pdf(&nested_output).unwrap();
    assert_eq!(nested_inspection.editable_form_fields.len(), 3);
    assert!(
        nested_inspection
            .editable_form_fields
            .iter()
            .all(|candidate| candidate.name != "customer.name")
    );
}

#[test]
fn form_field_deletion_is_stale_bound_and_signature_aware() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    form_fixture(&input, false);
    let mut stale = field(&input, "region").selector;
    stale.expected_field_sha256 = "0".repeat(64);
    assert!(matches!(
        delete_form_field(&DeleteFormFieldRequest {
            input_path: input,
            output_path: directory.path().join("stale.pdf"),
            selector: stale,
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::StaleFormField)
    ));

    let signed = directory.path().join("signed.pdf");
    form_fixture(&signed, true);
    let selector = field(&signed, "region").selector;
    assert!(matches!(
        delete_form_field(&DeleteFormFieldRequest {
            input_path: signed.clone(),
            output_path: directory.path().join("signed-unconfirmed.pdf"),
            selector: selector.clone(),
            allow_signature_invalidation: false,
        }),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    let confirmed = delete_form_field(&DeleteFormFieldRequest {
        input_path: signed,
        output_path: directory.path().join("signed-confirmed.pdf"),
        selector,
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        confirmed.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}
