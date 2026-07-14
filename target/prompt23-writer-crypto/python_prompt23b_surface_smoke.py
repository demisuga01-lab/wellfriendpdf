import json, oxide
required = [
    "pubsec_decrypt_pdf",
    "pubsec_decrypt_pdf_pfx",
    "pubsec_encrypt_pdf",
    "pubsec_reencrypt_pdf",
    "pubsec_reencrypt_pdf_pfx",
    "encrypt_pdf",
]
missing = [name for name in required if not hasattr(oxide, name)]
doc_methods = ["pubsec_report", "pdf_mac_report", "pdf_mac_verify", "pdf_mac_create"]
# Instantiate method names only; the installed-wheel runtime smoke exercises document opening separately.
if missing:
    raise AssertionError(f"missing Prompt 23B functions: {missing}")
print(json.dumps({
    "status": "passed",
    "module": oxide.__file__,
    "version": oxide.__version__,
    "functions": required,
    "document_methods": doc_methods,
}, sort_keys=True))
