# Prompt 22B Resource Dedup and Office Benchmark Closure

Starting checkpoint: `dda4406f021bc13455acbf9c4d01e690810c6ce5`

Verified starting HEAD: `dda4406f021bc13455acbf9c4d01e690810c6ce5`

Generation-time HEAD: `f68cd36c92d910607e16676f66c4ef84f6830410`

Status: `implemented_with_limits`

Blocked Prompt 22B rows: `0`

Prompt 22B closes the evidence gap left after Prompt 22 by making resource-family
deduplication explicit and by publishing benchmark, binding-runtime, reference,
and historical-gate artifacts under `target\prompt22-writer-office-benchmark`.

The production Office conversion path remains Oxide's native OOXML inspection
and shared model/PDF writer path. Microsoft Office, LibreOffice, Poppler,
PDFium, MuPDF, and qpdf are reference tools only. Reference availability is
recorded separately from pass status.

Dedup never merges from a hash alone. The planner uses SHA-256 as a bucket
prefilter, then compares resource family, canonical dictionary, decoded content
where safely decodable, owner/mutability posture, encryption/revision context,
mask/profile/resource dependencies, and exact semantic equality. Ambiguous
equality is a nonmerge.

Available reference tools: `microsoft_word, microsoft_powerpoint, microsoft_excel, poppler, qpdf`

Unavailable references not counted: `libreoffice_writer, libreoffice_impress, libreoffice_calc, pdfium, mupdf`
