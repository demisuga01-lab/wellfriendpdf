use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use std::fs;
use tempfile::tempdir;
use wellfriend_pdf_edit::styled_text::{TextStyle, replace_styled_text};
use wellfriend_pdf_edit::{
    PdfEditError, ReplaceTextRequest, SignatureEffect, inspect_pdf, inspect_pdf_for_page,
    replace_text,
};

#[test]
fn styled_unicode_text_reopens_with_each_embedded_font_and_preserves_source() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("style-source.pdf");
    fixture_pdf(&input, false, false);
    let source = fs::read(&input).unwrap();
    let selected = inspect_pdf(&input).unwrap().editable_text_runs.remove(0);
    assert!(selected.style_eligible);
    for family in ["Sans", "Serif", "Mono"] {
        for weight in ["Regular", "Bold", "Italic", "BoldItalic"] {
            let output = directory.path().join(format!("{family}-{weight}.pdf"));
            let style = TextStyle {
                font: format!("Liberation{family}-{weight}"),
                font_size: 17.5,
                color: "#2185D0".into(),
            };
            let request = ReplaceTextRequest {
                input_path: input.clone(),
                output_path: output.clone(),
                selector: selected.selector.clone(),
                replacement_text: "Café Ελληνικά Привет € 0042".into(),
                allow_signature_invalidation: false,
            };
            assert!(matches!(
                replace_text(&request),
                Err(PdfEditError::GlyphUnavailable)
            ));
            let result = replace_styled_text(&request, &style).unwrap();
            assert_eq!(result["validation"]["fontEmbedded"], true);
            assert_eq!(result["validation"]["addedFontObjects"], 6);
            assert_eq!(result["textStyle"]["font"], style.font);
            let reopened = inspect_pdf(&output).unwrap();
            assert_eq!(reopened.object_graph.images, 0);
            assert_eq!(
                reopened.editable_text_runs[0].selector.expected_text,
                request.replacement_text
            );
            assert_eq!(reopened.editable_text_runs[0].font_size, Some(17.5));
            let saved = Document::load(&output).unwrap();
            let stream = saved.get_page_contents(saved.get_pages()[&1])[0];
            let content = Content::decode(
                &saved
                    .get_object(stream)
                    .unwrap()
                    .as_stream()
                    .unwrap()
                    .get_plain_content()
                    .unwrap(),
            )
            .unwrap();
            let rgb = content
                .operations
                .iter()
                .find(|op| op.operator == "rg")
                .unwrap();
            for (actual, expected) in
                rgb.operands
                    .iter()
                    .zip([33.0 / 255.0, 133.0 / 255.0, 208.0 / 255.0])
            {
                let actual = actual.as_float().unwrap();
                assert!((actual - expected).abs() < 0.000_001);
            }
            assert!(reopened.editable_text_runs[0].style_eligible);
            let next = ReplaceTextRequest {
                input_path: output,
                output_path: directory
                    .path()
                    .join(format!("{family}-{weight}-again.pdf")),
                selector: reopened.editable_text_runs[0].selector.clone(),
                replacement_text: "Saved again Ω 0043".into(),
                allow_signature_invalidation: false,
            };
            replace_styled_text(
                &next,
                &TextStyle {
                    color: "#AA2222".into(),
                    ..style
                },
            )
            .unwrap();
        }
    }
    assert_eq!(fs::read(input).unwrap(), source);
}

#[test]
fn styled_text_refuses_invalid_fonts_shaping_overflow_stale_source_and_unconfirmed_signatures() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("source.pdf");
    fixture_pdf(&input, false, false);
    let selector = inspect_pdf(&input)
        .unwrap()
        .editable_text_runs
        .remove(0)
        .selector;
    let request = ReplaceTextRequest {
        input_path: input.clone(),
        output_path: directory.path().join("invalid.pdf"),
        selector,
        replacement_text: "Unchanged".into(),
        allow_signature_invalidation: false,
    };
    let style = TextStyle {
        font: "LiberationSans-Regular".into(),
        font_size: 18.0,
        color: "#123456".into(),
    };
    for bad in [
        TextStyle {
            font: "../../font.ttf".into(),
            ..style.clone()
        },
        TextStyle {
            color: "#12zz56".into(),
            ..style.clone()
        },
        TextStyle {
            font_size: f64::NAN,
            ..style.clone()
        },
        TextStyle {
            font_size: 513.0,
            ..style.clone()
        },
    ] {
        assert!(replace_styled_text(&request, &bad).is_err());
        assert!(!request.output_path.exists());
    }
    for text in ["नमस्ते", "مرحبا", "A\u{301}", "line\nline"] {
        assert!(
            replace_styled_text(
                &ReplaceTextRequest {
                    replacement_text: text.into(),
                    ..request.clone()
                },
                &style
            )
            .is_err()
        );
        assert!(!request.output_path.exists());
    }
    assert!(
        replace_styled_text(
            &ReplaceTextRequest {
                replacement_text: "Overflow".repeat(80),
                ..request.clone()
            },
            &style
        )
        .is_err()
    );
    let mut stale = request.clone();
    stale.selector.expected_text = "Not the source".into();
    assert!(replace_styled_text(&stale, &style).is_err());
    fixture_pdf(&input, true, false);
    assert!(matches!(
        replace_styled_text(&request, &style),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    assert!(!request.output_path.exists());
}

#[test]
fn styled_text_preserves_subsequent_inherited_font_and_refuses_cross_stream_geometry_state() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("inherited.pdf");
    fixture_pdf(&input, false, false);
    let mut document = Document::load(&input).unwrap();
    let page = document.get_pages()[&1];
    let stream_id = document.get_page_contents(page)[0];
    let stream = document
        .get_object_mut(stream_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    let mut bytes = stream.get_plain_content().unwrap();
    bytes.extend_from_slice(b"\nBT 1 0 0 1 72 680 Tm (Untouched) Tj ET\n");
    stream.set_plain_content(bytes);
    document.save(&input).unwrap();
    let source = inspect_pdf(&input).unwrap();
    let style = TextStyle {
        font: "LiberationSerif-Bold".into(),
        font_size: 22.0,
        color: "#ab1234".into(),
    };
    let request = ReplaceTextRequest {
        input_path: input.clone(),
        output_path: directory.path().join("styled.pdf"),
        selector: source.editable_text_runs[0].selector.clone(),
        replacement_text: "Invoice changed".into(),
        allow_signature_invalidation: false,
    };
    replace_styled_text(&request, &style).unwrap();
    let saved = inspect_pdf(&request.output_path).unwrap();
    let neighbor = saved
        .editable_text_runs
        .iter()
        .find(|r| r.selector.expected_text == "Untouched")
        .unwrap();
    assert_eq!(neighbor.font_size, Some(18.0));
    assert_eq!(neighbor.selector.expected_font_resource, "F1");
    for operator in ["2 0 0 2 0 0 cm", "2 Tc", "20 Tw", "200 Tz", "15 Ts", "3 Tr"] {
        let mut document = Document::load(&input).unwrap();
        let prefix =
            document.add_object(Stream::new(Dictionary::new(), operator.as_bytes().to_vec()));
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "Contents",
                vec![Object::Reference(prefix), Object::Reference(stream_id)],
            );
        let unsafe_input = directory.path().join("unsafe.pdf");
        document.save(&unsafe_input).unwrap();
        assert!(
            inspect_pdf(&unsafe_input)
                .unwrap()
                .editable_text_runs
                .iter()
                .all(|r| !r.style_eligible)
        );
        let unsafe_request = ReplaceTextRequest {
            input_path: unsafe_input,
            output_path: directory.path().join("must-not-exist.pdf"),
            ..request.clone()
        };
        assert!(replace_styled_text(&unsafe_request, &style).is_err());
        assert!(!unsafe_request.output_path.exists());
    }
}

fn fixture_pdf(path: &std::path::Path, signed: bool, modern_object_streams: bool) {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![Object::Name(b"F1".to_vec()), Object::Integer(18)],
            ),
            Operation::new(
                "Tm",
                vec![
                    1.into(),
                    0.into(),
                    0.into(),
                    1.into(),
                    72.into(),
                    720.into(),
                ],
            ),
            Operation::new("Tj", vec![Object::string_literal("Invoice 00042")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = document.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        },
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    document.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    if signed {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
            "Contents" => Object::String(vec![0; 64], lopdf::StringFormat::Hexadecimal),
        });
    }
    document.compress();
    if modern_object_streams {
        let mut bytes = Vec::new();
        document.save_modern(&mut bytes).unwrap();
        fs::write(path, bytes).unwrap();
    } else {
        document.save(path).unwrap();
    }
}

#[test]
fn level_one_replacement_modifies_the_real_text_operator_without_rasterizing() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture_pdf(&input, false, false);

    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.page_count, 1);
    assert_eq!(inspection.object_graph.images, 0);
    assert_eq!(inspection.editable_text_runs.len(), 1);
    let selector = inspection.editable_text_runs[0].selector.clone();
    assert_eq!(selector.expected_text, "Invoice 00042");
    assert!(inspection.editable_text_runs[0].exact_round_trip);

    let result = replace_text(&ReplaceTextRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: selector.clone(),
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.validation.passed());
    assert!(result.validation.no_page_rasterization);
    assert_eq!(result.changed_objects, vec![selector.content_stream_object]);
    assert_eq!(result.source_object_count, result.output_object_count);
    assert_eq!(result.signature_effect, SignatureEffect::None);
    assert_ne!(result.input_sha256, result.output_sha256);

    let edited = Document::load(&output).unwrap();
    assert_eq!(
        edited.extract_text_with_limit(&[1], 1024 * 1024).unwrap(),
        "Invoice 00043\n"
    );
    let page_id = edited.get_pages()[&1];
    let stream_id: lopdf::ObjectId = selector.content_stream_object.into();
    let stream = edited.get_object(stream_id).unwrap().as_stream().unwrap();
    let decoded =
        Content::decode(&stream.get_plain_content_with_limit(1024 * 1024).unwrap()).unwrap();
    assert_eq!(decoded.operations[selector.operation_index].operator, "Tj");
    assert_eq!(
        decoded
            .operations
            .iter()
            .filter(|operation| operation.operator == "Do")
            .count(),
        0
    );
    assert!(edited.get_page_images(page_id).unwrap().is_empty());
}

#[test]
fn modern_object_stream_housekeeping_does_not_fail_semantic_preservation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("modern-input.pdf");
    let output = directory.path().join("modern-output.pdf");
    fixture_pdf(&input, false, true);

    let inspection = inspect_pdf(&input).unwrap();
    let selector = inspection.editable_text_runs[0].selector.clone();
    let result = replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: output.clone(),
        selector,
        replacement_text: "Browser-native edit proof".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert!(result.validation.passed());
    assert!(result.validation.object_count_preserved);
    assert!(result.validation.non_target_objects_preserved);
    assert_ne!(result.source_object_count, result.output_object_count);
    assert_eq!(
        Document::load(&output)
            .unwrap()
            .extract_text_with_limit(&[1], 1024 * 1024)
            .unwrap(),
        "Browser-native edit proof\n"
    );
}

#[test]
fn replacement_refuses_unrepresentable_glyphs_and_stale_selectors() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();

    let glyph_error = replace_text(&ReplaceTextRequest {
        input_path: input.clone(),
        output_path: directory.path().join("glyph.pdf"),
        selector: selector.clone(),
        replacement_text: "Invoice 😀".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(glyph_error, PdfEditError::GlyphUnavailable));

    let mut stale = selector;
    stale.expected_text = "Invoice 99999".to_string();
    let stale_error = replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: directory.path().join("stale.pdf"),
        selector: stale,
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(stale_error, PdfEditError::StaleTextRun));
}

#[test]
fn signed_pdf_requires_explicit_invalidation_confirmation() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("signed.pdf");
    fixture_pdf(&input, true, false);
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.signatures.len(), 1);
    let selector = inspection.editable_text_runs[0].selector.clone();
    let output = directory.path().join("edited.pdf");

    let error = replace_text(&ReplaceTextRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: selector.clone(),
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
    assert!(!output.exists());

    let result = replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: output,
        selector,
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: true,
    })
    .unwrap();
    assert_eq!(
        result.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
}

#[test]
fn identical_inputs_and_commands_produce_identical_outputs() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();
    let output_a = directory.path().join("a.pdf");
    let output_b = directory.path().join("b.pdf");

    let result_a = replace_text(&ReplaceTextRequest {
        input_path: input.clone(),
        output_path: output_a.clone(),
        selector: selector.clone(),
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap();
    let result_b = replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: output_b.clone(),
        selector,
        replacement_text: "Invoice 00043".to_string(),
        allow_signature_invalidation: false,
    })
    .unwrap();

    assert_eq!(result_a.output_sha256, result_b.output_sha256);
    assert_eq!(fs::read(output_a).unwrap(), fs::read(output_b).unwrap());
}

fn replace_fixture_show(path: &std::path::Path, operation: Operation) {
    let mut document = Document::load(path).unwrap();
    let stream_id = document.get_page_contents(document.get_pages()[&1])[0];
    let stream = document
        .get_object_mut(stream_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    let mut content =
        Content::decode(&stream.get_plain_content_with_limit(1024 * 1024).unwrap()).unwrap();
    content.operations[3] = operation;
    stream.set_plain_content(content.encode().unwrap());
    document.save(path).unwrap();
}

fn read_show(path: &std::path::Path, index: usize) -> Operation {
    let document = Document::load(path).unwrap();
    let stream_id = document.get_page_contents(document.get_pages()[&1])[0];
    let stream = document.get_object(stream_id).unwrap().as_stream().unwrap();
    Content::decode(&stream.get_plain_content_with_limit(1024 * 1024).unwrap())
        .unwrap()
        .operations[index]
        .clone()
}

#[test]
fn positioned_text_and_quote_operators_are_inspectable_and_replaced_in_place() {
    for operation in [
        Operation::new(
            "TJ",
            vec![Object::Array(vec![
                Object::string_literal("Invoice "),
                (-20).into(),
                Object::string_literal("00042"),
            ])],
        ),
        Operation::new("'", vec![Object::string_literal("Invoice 00042")]),
        Operation::new(
            "\"",
            vec![3.into(), 1.into(), Object::string_literal("Invoice 00042")],
        ),
    ] {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.pdf");
        let output = directory.path().join("output.pdf");
        fixture_pdf(&input, false, false);
        replace_fixture_show(&input, operation.clone());
        let inspection = inspect_pdf(&input).unwrap();
        assert_eq!(inspection.unsupported_text_show_operations, 0);
        let run = &inspection.editable_text_runs[0];
        assert_eq!(run.operator, operation.operator);
        assert!(!run.reflow_eligible);
        let result = replace_text(&ReplaceTextRequest {
            input_path: input,
            output_path: output.clone(),
            selector: run.selector.clone(),
            replacement_text: "Invoice 00043".into(),
            allow_signature_invalidation: false,
        })
        .unwrap();
        assert!(result.validation.passed());
        assert_eq!(
            result.changed_objects,
            vec![run.selector.content_stream_object]
        );
        let edited = inspect_pdf(&output).unwrap();
        assert_eq!(
            edited.editable_text_runs[0].selector.expected_text,
            "Invoice 00043"
        );
        let actual = read_show(&output, 3);
        assert_eq!(actual.operator, operation.operator);
        if operation.operator == "\"" {
            assert_eq!(
                format!("{:?}", &actual.operands[..2]),
                format!("{:?}", &operation.operands[..2])
            );
        }
        if operation.operator == "TJ" {
            assert_eq!(
                actual.operands[0].as_array().unwrap()[1].as_i64().unwrap(),
                -20
            );
        }
    }
}

#[test]
fn array_kerning_tracks_unchanged_glyphs_not_replacement_byte_offsets() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture_pdf(&input, false, false);
    replace_fixture_show(
        &input,
        Operation::new(
            "TJ",
            vec![Object::Array(vec![
                7.into(),
                Object::string_literal("A"),
                (-12).into(),
                Object::string_literal("B"),
                22.into(),
                Object::string_literal("C"),
                33.into(),
                Object::string_literal("D"),
                9.into(),
            ])],
        ),
    );
    let run = inspect_pdf(&input).unwrap().editable_text_runs[0].clone();
    replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: output.clone(),
        selector: run.selector,
        replacement_text: "AxyD".into(),
        allow_signature_invalidation: false,
    })
    .unwrap();
    let actual = read_show(&output, 3);
    let numbers: Vec<_> = actual.operands[0]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item.as_i64().ok())
        .collect();
    assert_eq!(numbers, vec![7, -12, 33, 9]);
    assert_eq!(
        inspect_pdf(&output).unwrap().editable_text_runs[0]
            .selector
            .expected_text,
        "AxyD"
    );
}

#[test]
fn malformed_array_and_unrepresentable_replacement_never_produce_output() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let selector = inspect_pdf(&input).unwrap().editable_text_runs[0]
        .selector
        .clone();
    replace_fixture_show(
        &input,
        Operation::new(
            "TJ",
            vec![Object::Array(vec![
                Object::string_literal("Invoice 00042"),
                Object::Name(b"not-a-number".to_vec()),
            ])],
        ),
    );
    let inspection = inspect_pdf(&input).unwrap();
    assert_eq!(inspection.unsupported_text_show_operations, 1);
    assert!(inspection.editable_text_runs.is_empty());
    let output = directory.path().join("output.pdf");
    assert!(matches!(
        replace_text(&ReplaceTextRequest {
            input_path: input.clone(),
            output_path: output.clone(),
            selector: selector.clone(),
            replacement_text: "Invoice 00043".into(),
            allow_signature_invalidation: false
        }),
        Err(PdfEditError::TextRunUnsupported)
    ));
    assert!(!output.exists());
    replace_fixture_show(
        &input,
        Operation::new(
            "TJ",
            vec![Object::Array(vec![Object::string_literal("Invoice 00042")])],
        ),
    );
    assert!(matches!(
        replace_text(&ReplaceTextRequest {
            input_path: input,
            output_path: output.clone(),
            selector,
            replacement_text: "Invoice 😀".into(),
            allow_signature_invalidation: false
        }),
        Err(PdfEditError::GlyphUnavailable)
    ));
    assert!(!output.exists());
}

#[test]
fn text_font_state_survives_content_stream_boundaries_and_graphics_restore() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let mut document = Document::load(&input).unwrap();
    let page = document.get_pages()[&1];
    let content_id = document.get_page_contents(page)[0];
    let prefix = document.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F1 18 Tf 1 0 0 1 72 720 Tm q /missing-font 6 Tf Q\n".to_vec(),
    ));
    document
        .get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .set_plain_content(b"[(Invoice ) -5 (00042)] TJ ET\n".to_vec());
    document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Contents",
            vec![Object::Reference(prefix), Object::Reference(content_id)],
        );
    document.save(&input).unwrap();
    let run = inspect_pdf(&input).unwrap().editable_text_runs[0].clone();
    assert_eq!(run.selector.expected_font_resource, "F1");
    assert_eq!(run.font_size, Some(18.0));
    let output = directory.path().join("output.pdf");
    assert!(
        replace_text(&ReplaceTextRequest {
            input_path: input,
            output_path: output,
            selector: run.selector,
            replacement_text: "Invoice 00043".into(),
            allow_signature_invalidation: false
        })
        .unwrap()
        .validation
        .passed()
    );
}

#[test]
fn multibyte_cid_arrays_are_encoded_on_character_boundaries() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let mut document = Document::load(&input).unwrap();
    let page = document.get_pages()[&1];
    let font_id = document
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Font")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F1")
        .unwrap()
        .as_reference()
        .unwrap();
    let cmap = document.add_object(Stream::new(Dictionary::new(), b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Test-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0001> <4E00> <0002> <4E8C> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end".to_vec()));
    let font = document
        .get_object_mut(font_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    font.set("Subtype", "Type0");
    font.set("Encoding", "Identity-H");
    font.set("ToUnicode", cmap);
    document.save(&input).unwrap();
    replace_fixture_show(
        &input,
        Operation::new(
            "TJ",
            vec![Object::Array(vec![
                Object::String(vec![0, 1], lopdf::StringFormat::Hexadecimal),
                (-10).into(),
                Object::String(vec![0, 2], lopdf::StringFormat::Hexadecimal),
            ])],
        ),
    );
    let run = inspect_pdf(&input).unwrap().editable_text_runs[0].clone();
    assert_eq!(run.selector.expected_text, "一二");
    let output = directory.path().join("output.pdf");
    replace_text(&ReplaceTextRequest {
        input_path: input,
        output_path: output.clone(),
        selector: run.selector,
        replacement_text: "一二一".into(),
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(
        inspect_pdf(&output).unwrap().editable_text_runs[0]
            .selector
            .expected_text,
        "一二一"
    );
    for item in read_show(&output, 3).operands[0].as_array().unwrap() {
        if let Object::String(bytes, _) = item {
            assert_eq!(bytes.len() % 2, 0);
        }
    }
}

#[test]
fn page_inspection_can_reach_text_beyond_the_global_run_limit() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture_pdf(&input, false, false);
    let mut document = Document::load(&input).unwrap();
    let first = document.get_pages()[&1];
    let mut second = document
        .get_object(first)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let stream = document.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf (Last page) Tj ET".to_vec(),
    ));
    second.set("Contents", stream);
    let second_id = document.add_object(second);
    let parent = document
        .get_object(first)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let tree = document
        .get_object_mut(parent)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    tree.set(
        "Kids",
        vec![Object::Reference(first), Object::Reference(second_id)],
    );
    tree.set("Count", 2);
    let content_id = document.get_page_contents(first)[0];
    let mut content = b"BT /F1 12 Tf ".to_vec();
    for _ in 0..10_001 {
        content.extend_from_slice(b"(Test) Tj ");
    }
    content.extend_from_slice(b"ET");
    document
        .get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .set_plain_content(content);
    document.save(&input).unwrap();
    assert!(inspect_pdf(&input).unwrap().editable_text_runs_truncated);
    let late = inspect_pdf_for_page(&input, Some(2)).unwrap();
    assert!(!late.editable_text_runs_truncated);
    assert_eq!(late.editable_text_runs.len(), 1);
    assert_eq!(late.editable_text_runs[0].selector.page_number, 2);
    assert!(matches!(
        inspect_pdf_for_page(&input, Some(3)),
        Err(PdfEditError::PageLimitExceeded)
    ));
}
