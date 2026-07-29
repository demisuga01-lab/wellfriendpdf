use wellfriendpdf_engine::writer::{OutputObject, PdfWriter};
use wellfriendpdf_engine::{
    edit_text_operator, ContentEngine, OperatorTextEditRequest, PdfDictionary, PdfObject,
};

fn main() -> wellfriendpdf_engine::Result<()> {
    let input = source_text_fixture();
    let request = OperatorTextEditRequest {
        page: 1,
        source_text: "ABC".to_string(),
        replacement_text: "DEF".to_string(),
        signature_policy_override: false,
    };

    let (edited, report) = edit_text_operator(&input, &request)?;
    let reopened = ContentEngine::open_bytes(edited)?;
    let text = reopened.get_page_text(1)?;

    assert!(report.unaffected_content_proof["overlay_used"] == false);
    assert!(text.contains("DEF"));
    assert!(!text.contains("ABC"));
    println!(
        "changed_pages={:?} validation={}",
        report.changed_pages, report.validation
    );
    Ok(())
}

fn source_text_fixture() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n".to_vec();
    let mut catalog = PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
    catalog.insert(
        "Pages",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );

    let mut pages = PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".to_string()));
    pages.insert("Count", PdfObject::Integer(1));
    pages.insert(
        "Kids",
        PdfObject::Array(vec![PdfObject::Reference {
            number: 3,
            generation: 0,
        }]),
    );

    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));

    let mut fonts = PdfDictionary::empty();
    fonts.insert(
        "F1",
        PdfObject::Reference {
            number: 5,
            generation: 0,
        },
    );
    let mut resources = PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));

    let mut page = PdfDictionary::empty();
    page.insert("Type", PdfObject::Name("Page".to_string()));
    page.insert(
        "Parent",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );
    page.insert(
        "MediaBox",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(200),
            PdfObject::Integer(200),
        ]),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    page.insert(
        "Contents",
        PdfObject::Reference {
            number: 4,
            generation: 0,
        },
    );

    let mut stream_dict = PdfDictionary::empty();
    stream_dict.insert("Length", PdfObject::Integer(content.len() as i64));
    PdfWriter::new(
        vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: stream_dict,
                    raw: content,
                },
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(font),
            },
        ],
        1,
    )
    .write()
    .expect("fixture PDF")
}
