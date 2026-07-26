# Public PDF corpus policy

Prompt 30 downloads only the small allow-list in
`scripts/download_prompt30_public_pdf_corpus.py`: public United States government
forms and publications. The downloader verifies a PDF header, per-file and total
size caps, SHA-256, and resumable result manifests.

Downloaded content stays below the VPS temporary `public-pdf-corpus/` directory.
It is not committed. Each manifest records URL, source classification, timestamp,
byte size, hash, category, and public-log posture. Unclear, credentialed, paywalled,
private, or randomly sourced copyrighted material is excluded.
