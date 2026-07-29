import json
import wellfriendpdf


document = wellfriendpdf.open("input.pdf")
print(json.dumps(document.secure_mutation_report(), indent=2))
print(json.dumps(document.edit_policy_report("attachment_remove"), indent=2))

options = json.dumps(
    {
        "filename": "evidence.txt",
        "mime": "text/plain",
        "relationship": "data",
        "deterministic": True,
    }
)
pdf_bytes, report = document.associated_file_add(
    b"bounded evidence", options, output="secure_mutation-associated.pdf"
)
print(len(pdf_bytes), json.dumps(report, indent=2))
