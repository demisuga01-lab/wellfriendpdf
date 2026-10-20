use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
    dictionary,
};
use tempfile::tempdir;
use wellfriend_pdf_edit::{PdfEditError, SignatureEffect, native_unlock};

fn fixture() -> Document {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let content = document.add_object(Stream::new(
        Dictionary::new(),
        b"q 1 0 0 rg 10 20 30 40 re f Q".to_vec(),
    ));
    let annotation = document.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Contents"=>Object::string_literal("Preserve annotation"), "Rect"=>vec![10.into(),20.into(),30.into(),40.into()]});
    let page = document.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages, "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()], "Contents"=>content, "Resources"=>Dictionary::new(), "Annots"=>vec![annotation.into()]});
    document.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages", "Kids"=>vec![page.into()], "Count"=>1}.into(),
    );
    let catalog = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
    let info = document.add_object(dictionary! {"Title"=>Object::string_literal("Original title")});
    document.trailer.set("Root", catalog);
    document.trailer.set("Info", info);
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("synthetic-id"),
            Object::string_literal("synthetic-id"),
        ],
    );
    document
}

fn encrypt(document: &mut Document, user: &str) {
    let state = EncryptionState::try_from(EncryptionVersion::V1 {
        document,
        owner_password: "synthetic-owner",
        user_password: user,
        permissions: Permissions::PRINTABLE,
    })
    .unwrap();
    document.encrypt(&state).unwrap();
}

#[test]
fn loads_full_authenticated_graph_and_preserves_content_and_annotation() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("encrypted.pdf");
    let original = fixture();
    let mut encrypted = original.clone();
    encrypt(&mut encrypted, "synthetic-user");
    encrypted.save(&input).unwrap();
    for (index, password) in ["synthetic-user", "synthetic-owner"].iter().enumerate() {
        let mut decrypted = native_unlock::load(&input, password).unwrap();
        assert_eq!(decrypted.get_pages(), original.get_pages());
        for (id, value) in &original.objects {
            assert_eq!(decrypted.get_object(*id).unwrap(), value);
        }
        assert!(
            decrypted
                .objects
                .iter()
                .all(|(id, value)| original.objects.contains_key(id)
                    || value.type_name().is_ok_and(|kind| kind == b"XRef"))
        );
        assert!(decrypted.encryption_state.is_none());
        let output = dir.path().join(format!("unlocked-{index}.pdf"));
        let proof = native_unlock::write(&mut decrypted, &output).unwrap();
        assert_eq!(proof.page_count, 1);
        assert!(proof.page_content_preserved && proof.decrypted_objects_preserved);
        assert_eq!(proof.signature_effect, SignatureEffect::None);
        let parsed = Document::load(&output).unwrap();
        assert!(!parsed.is_encrypted());
        for (id, value) in &original.objects {
            assert_eq!(parsed.get_object(*id).unwrap(), value);
        }
    }
    assert!(matches!(
        native_unlock::load(&input, "wrong"),
        Err(PdfEditError::EncryptedPdf)
    ));
}

#[test]
fn wrong_supplied_password_is_refused_even_with_empty_user_access() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("empty-user.pdf");
    let mut document = fixture();
    encrypt(&mut document, "");
    document.save(&input).unwrap();
    assert_eq!(
        native_unlock::load(&input, "").unwrap().get_pages().len(),
        1
    );
    assert_eq!(
        native_unlock::load(&input, "synthetic-owner")
            .unwrap()
            .get_pages()
            .len(),
        1
    );
    assert!(matches!(
        native_unlock::load(&input, "wrong"),
        Err(PdfEditError::EncryptedPdf)
    ));
}

#[test]
fn unlock_writer_does_not_replace_existing_output_and_marks_signatures() {
    let dir = tempdir().unwrap();
    let output = dir.path().join("output.pdf");
    let mut document = fixture();
    document.add_object(
        dictionary! {"Type"=>"Sig", "ByteRange"=>vec![0.into(),1.into(),2.into(),3.into()]},
    );
    let proof = native_unlock::write(&mut document, &output).unwrap();
    assert_eq!(
        proof.signature_effect,
        SignatureEffect::InvalidatesExistingSignatures
    );
    let before = std::fs::read(&output).unwrap();
    assert!(native_unlock::write(&mut document, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), before);
}
