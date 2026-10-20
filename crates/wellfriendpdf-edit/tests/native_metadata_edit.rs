use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    EditMetadataRequest, PdfEditError, PdfMetadataSpec, PdfXmpMode, PdfXmpState, edit_metadata,
    inspect_pdf,
};

fn fixture(with_foreign_xmp: bool, with_signature: bool) -> Document {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let content = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    let page = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content,
        "Resources" => Dictionary::new(),
    });
    document.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page.into()],
            "Count" => 1,
        }
        .into(),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages };
    if with_foreign_xmp {
        let metadata = document.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<foreign>preserve me</foreign>".to_vec(),
        ));
        catalog.set("Metadata", metadata);
    }
    let catalog = document.add_object(catalog);
    document.trailer.set("Root", catalog);
    let info = document.add_object(dictionary! {
        "Title" => Object::string_literal("Original"),
        "Producer" => Object::string_literal("Fixture Producer"),
    });
    document.trailer.set("Info", info);
    if with_signature {
        document.add_object(dictionary! {
            "Type" => "Sig",
            "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
        });
    }
    document
}

#[test]
fn edits_info_and_creates_owned_xmp_without_touching_page_content() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(false, false).save(&input).unwrap();
    let source = inspect_pdf(&input).unwrap();
    let metadata = source.editable_metadata.unwrap();
    assert_eq!(metadata.selector.xmp_state, PdfXmpState::Absent);
    assert!(metadata.can_sync_xmp);
    let values = PdfMetadataSpec {
        title: Some("Quarterly <report> & review".to_string()),
        author: Some("Wellfriend QA".to_string()),
        subject: Some("Object-preserving metadata".to_string()),
        keywords: vec!["evidence".to_string(), "native-pdf".to_string()],
    };
    let result = edit_metadata(&EditMetadataRequest {
        input_path: input.clone(),
        output_path: output.clone(),
        selector: metadata.selector,
        values: values.clone(),
        xmp_mode: PdfXmpMode::SyncBasicFields,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(result.capability, "PDF_METADATA_EDIT");
    assert_eq!(result.result_values, values);
    assert_eq!(
        result.result_selector.xmp_state,
        PdfXmpState::WellfriendOwned
    );
    assert!(result.validation.passed());
    assert_eq!(result.changed_objects.len(), 3);

    let output_document = Document::load(&output).unwrap();
    let info_id = output_document
        .trailer
        .get(b"Info")
        .unwrap()
        .as_reference()
        .unwrap();
    let info = output_document.get_dictionary(info_id).unwrap();
    assert_eq!(
        lopdf::decode_text_string(info.get(b"Producer").unwrap()).unwrap(),
        "Fixture Producer"
    );
    let xmp_id = result.result_selector.xmp_object.unwrap().into();
    let xmp = output_document
        .get_object(xmp_id)
        .unwrap()
        .as_stream()
        .unwrap()
        .get_plain_content()
        .unwrap();
    let xmp = String::from_utf8(xmp).unwrap();
    assert!(xmp.contains("Quarterly &lt;report&gt; &amp; review"));
    assert!(!xmp.contains("Quarterly <report>"));
    assert_eq!(inspect_pdf(&output).unwrap().page_count, 1);
}

#[test]
fn preserves_foreign_xmp_byte_for_byte_when_requested() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    let output = directory.path().join("output.pdf");
    fixture(true, false).save(&input).unwrap();
    let metadata = inspect_pdf(&input).unwrap().editable_metadata.unwrap();
    assert_eq!(metadata.selector.xmp_state, PdfXmpState::ForeignReadOnly);
    assert!(!metadata.can_sync_xmp);
    let source_xmp_hash = metadata.selector.expected_xmp_sha256.clone();
    let result = edit_metadata(&EditMetadataRequest {
        input_path: input,
        output_path: output,
        selector: metadata.selector,
        values: PdfMetadataSpec {
            title: Some("Updated title".to_string()),
            ..metadata.values
        },
        xmp_mode: PdfXmpMode::Preserve,
        allow_signature_invalidation: false,
    })
    .unwrap();
    assert_eq!(
        result.result_selector.xmp_state,
        PdfXmpState::ForeignReadOnly
    );
    assert_eq!(result.result_selector.expected_xmp_sha256, source_xmp_hash);
    assert_eq!(result.changed_objects.len(), 1);
    assert!(result.validation.xmp_policy_preserved);
}

#[test]
fn rejects_foreign_xmp_sync_and_requires_signature_confirmation() {
    let directory = tempdir().unwrap();
    let foreign_input = directory.path().join("foreign.pdf");
    fixture(true, false).save(&foreign_input).unwrap();
    let foreign = inspect_pdf(&foreign_input)
        .unwrap()
        .editable_metadata
        .unwrap();
    let error = edit_metadata(&EditMetadataRequest {
        input_path: foreign_input,
        output_path: directory.path().join("foreign-output.pdf"),
        selector: foreign.selector,
        values: PdfMetadataSpec {
            title: Some("Blocked".to_string()),
            ..foreign.values
        },
        xmp_mode: PdfXmpMode::SyncBasicFields,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::ForeignXmpReadOnly));

    let signed_input = directory.path().join("signed.pdf");
    fixture(false, true).save(&signed_input).unwrap();
    let signed = inspect_pdf(&signed_input)
        .unwrap()
        .editable_metadata
        .unwrap();
    let error = edit_metadata(&EditMetadataRequest {
        input_path: signed_input,
        output_path: directory.path().join("signed-output.pdf"),
        selector: signed.selector,
        values: PdfMetadataSpec {
            title: Some("Confirmed only".to_string()),
            ..signed.values
        },
        xmp_mode: PdfXmpMode::Preserve,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(error, PdfEditError::SignatureConfirmationRequired));
}

#[test]
fn rejects_stale_or_unchanged_metadata_requests() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("input.pdf");
    fixture(false, false).save(&input).unwrap();
    let metadata = inspect_pdf(&input).unwrap().editable_metadata.unwrap();

    let unchanged = edit_metadata(&EditMetadataRequest {
        input_path: input.clone(),
        output_path: directory.path().join("unchanged.pdf"),
        selector: metadata.selector.clone(),
        values: metadata.values.clone(),
        xmp_mode: PdfXmpMode::Preserve,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(unchanged, PdfEditError::InvalidMetadataEdit));

    let mut stale_selector = metadata.selector;
    stale_selector.expected_info_sha256 = Some("0".repeat(64));
    let stale = edit_metadata(&EditMetadataRequest {
        input_path: input,
        output_path: directory.path().join("stale.pdf"),
        selector: stale_selector,
        values: PdfMetadataSpec {
            title: Some("Changed".to_string()),
            ..metadata.values
        },
        xmp_mode: PdfXmpMode::Preserve,
        allow_signature_invalidation: false,
    })
    .unwrap_err();
    assert!(matches!(stale, PdfEditError::StaleMetadata));
}
