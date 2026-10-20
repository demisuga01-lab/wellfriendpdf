use lopdf::{Dictionary, Document, Object, Stream, dictionary, text_string};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::tempdir;
use wellfriend_pdf_edit::{
    PdfEditError,
    native_metadata::{MetadataPlan, edit_website_metadata, verify_website_metadata},
};

fn fixture(path: &Path, info: bool, xmp: bool, signature: bool, modern: bool) {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"q 1 0 0 1 0 0 cm Q".to_vec(),
    ));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => Dictionary::new(),
    });
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages", "Kids"=>vec![page.into()], "Count"=>1}.into(),
    );
    let mut catalog =
        dictionary! {"Type"=>"Catalog", "Pages"=>pages, "Lang"=>text_string("日本語")};
    if xmp {
        let metadata = doc.add_object(Stream::new(
            dictionary! {"Type"=>"Metadata", "Subtype"=>"XML"},
            b"<original custom='keep'/>".to_vec(),
        ));
        catalog.set("Metadata", metadata);
    }
    let catalog = doc.add_object(catalog);
    doc.trailer.set("Root", catalog);
    doc.trailer.set("PrivateTrailer", text_string("preserve"));
    if info {
        let info = doc.add_object(dictionary! {"Title"=>text_string("Original"), "Producer"=>text_string("Producer"), "Custom"=>text_string("custom 日本語")});
        doc.trailer.set("Info", info);
    }
    if signature {
        doc.add_object(
            dictionary! {"Type"=>"Sig", "ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()]},
        );
    }
    if modern {
        doc.save_modern(&mut fs::File::create(path).unwrap())
            .unwrap();
    } else {
        doc.save(path).unwrap();
    }
}

fn plan(xmp: bool) -> MetadataPlan {
    MetadataPlan {
        fields: BTreeMap::from([
            (
                b"Title".to_vec(),
                Some(text_string("Quarterly 日本語 & report")),
            ),
            (b"Producer".to_vec(), None),
        ]),
        xmp: xmp.then(|| b"<replacement custom='retained'>metadata</replacement>".to_vec()),
    }
}

#[test]
fn accepts_existing_writer_output_without_writing_either_file() {
    for info in [false, true] {
        for xmp in [false, true] {
            for modern in [false, true] {
                let dir = tempdir().unwrap();
                let input = dir.path().join("input.pdf");
                let output = dir.path().join("output.pdf");
                fixture(&input, info, xmp, false, modern);
                let written = edit_website_metadata(&input, &output, plan(xmp), false).unwrap();
                let before = (fs::read(&input).unwrap(), fs::read(&output).unwrap());
                let verified = verify_website_metadata(&input, &output, plan(xmp), false).unwrap();
                assert_eq!(verified.input_sha256, written.input_sha256);
                assert_eq!(verified.output_sha256, written.output_sha256);
                assert_eq!(verified.output_bytes, written.output_bytes);
                assert_eq!(verified.page_count, 1);
                assert!(verified.non_metadata_objects_preserved);
                assert_eq!(
                    before,
                    (fs::read(&input).unwrap(), fs::read(&output).unwrap())
                );
                assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
            }
        }
    }
}

fn rejects_mutation(mutate: impl FnOnce(&mut Document)) {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input.pdf");
    let output = dir.path().join("output.pdf");
    fixture(&input, true, true, false, false);
    edit_website_metadata(&input, &output, plan(true), false).unwrap();
    let mut doc = Document::load(&output).unwrap();
    mutate(&mut doc);
    doc.save(&output).unwrap();
    let before = (fs::read(&input).unwrap(), fs::read(&output).unwrap());
    assert!(matches!(
        verify_website_metadata(&input, &output, plan(true), false),
        Err(PdfEditError::ValidationFailed)
    ));
    assert_eq!(
        before,
        (fs::read(&input).unwrap(), fs::read(&output).unwrap())
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn rejects_wrong_requested_metadata() {
    rejects_mutation(|doc| {
        let id = doc.trailer.get(b"Info").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(id)
            .unwrap()
            .set("Title", text_string("wrong"));
    });
}

#[test]
fn rejects_custom_info_changes_inside_modified_object() {
    rejects_mutation(|doc| {
        let id = doc.trailer.get(b"Info").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(id)
            .unwrap()
            .set("Custom", text_string("changed"));
    });
}

#[test]
fn rejects_catalog_changes_inside_modified_object() {
    rejects_mutation(|doc| {
        let id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(id)
            .unwrap()
            .set("Lang", text_string("wrong"));
    });
}

#[test]
fn rejects_xmp_content_and_stream_dictionary_changes() {
    for content in [false, true] {
        rejects_mutation(|doc| {
            let id = doc
                .catalog()
                .unwrap()
                .get(b"Metadata")
                .unwrap()
                .as_reference()
                .unwrap();
            let stream = doc.get_object_mut(id).unwrap().as_stream_mut().unwrap();
            if content {
                stream.set_content(b"<wrong/>".to_vec());
            } else {
                stream.dict.set("Extra", true);
            }
        });
    }
}

#[test]
fn rejects_page_and_content_changes() {
    for content in [false, true] {
        rejects_mutation(|doc| {
            let id = *doc.get_pages().values().next().unwrap();
            if content {
                let stream = doc
                    .get_dictionary(id)
                    .unwrap()
                    .get(b"Contents")
                    .unwrap()
                    .as_reference()
                    .unwrap();
                doc.get_object_mut(stream)
                    .unwrap()
                    .as_stream_mut()
                    .unwrap()
                    .set_content(b"changed".to_vec());
            } else {
                doc.get_dictionary_mut(id).unwrap().set("Rotate", 90);
            }
        });
    }
}

#[test]
fn rejects_extra_or_missing_objects() {
    rejects_mutation(|doc| {
        doc.add_object(dictionary! {"Extra"=>true});
    });
    rejects_mutation(|doc| {
        let id = doc
            .catalog()
            .unwrap()
            .get(b"Metadata")
            .unwrap()
            .as_reference()
            .unwrap();
        doc.objects.remove(&id);
    });
}

#[test]
fn rejects_version_and_trailer_changes() {
    rejects_mutation(|doc| doc.version = "1.6".into());
    rejects_mutation(|doc| doc.trailer.set("PrivateTrailer", text_string("changed")));
    rejects_mutation(|doc| doc.trailer.set("UnexpectedTrailer", true));
    rejects_mutation(|doc| {
        doc.trailer.remove(b"PrivateTrailer");
    });
}

#[test]
fn refuses_source_alias_and_malformed_candidate() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input.pdf");
    let output = dir.path().join("output.pdf");
    fixture(&input, true, true, false, false);
    assert!(matches!(
        verify_website_metadata(&input, &input, plan(true), false),
        Err(PdfEditError::ValidationFailed)
    ));
    fs::write(&output, b"%PDF-1.7\ntruncated").unwrap();
    assert!(matches!(
        verify_website_metadata(&input, &output, plan(true), false),
        Err(PdfEditError::InvalidPdf)
    ));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn enforces_signature_and_plan_refusals_during_verification() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("input.pdf");
    let output = dir.path().join("output.pdf");
    fixture(&input, true, true, true, false);
    edit_website_metadata(&input, &output, plan(true), true).unwrap();
    assert!(matches!(
        verify_website_metadata(&input, &output, plan(true), false),
        Err(PdfEditError::SignatureConfirmationRequired)
    ));
    assert!(verify_website_metadata(&input, &output, plan(true), true).is_ok());
    let unsupported = MetadataPlan {
        fields: BTreeMap::from([(b"Custom".to_vec(), Some(Object::Null))]),
        xmp: None,
    };
    assert!(matches!(
        verify_website_metadata(&input, &output, unsupported, true),
        Err(PdfEditError::MetadataUnsupported)
    ));
    let oversized = MetadataPlan {
        fields: BTreeMap::new(),
        xmp: Some(vec![0; 4 * 1024 * 1024 + 1]),
    };
    assert!(matches!(
        verify_website_metadata(&input, &output, oversized, true),
        Err(PdfEditError::MetadataUnsupported)
    ));
}
