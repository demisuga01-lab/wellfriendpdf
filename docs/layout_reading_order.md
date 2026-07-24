# Layout And Reading Order

PDF pages contain positioned drawing operations, not words, paragraphs, or reading order. Wellfriend's Prompt 06 text model uses a staged deterministic reconstruction path.

## Strategy Order

1. Existing tagged-PDF extraction path when callers request semantic extraction and tags are usable.
2. XY-cut geometry for untagged text blocks and columns.
3. Reading-order reconstruction for RTL and vertical text runs.
4. Stable visual fallback with low-confidence diagnostics.

The model records the strategy per page:

- `tagged_pdf`
- `xy_cut_geometry`
- `vertical_writing`
- `visual_fallback`

## Word And Line Grouping

Words are reconstructed from the contributing `TextChunk` and character quads. When a word break is implied by geometry rather than an encoded space, the model inserts a synthetic space character with `synthetic_layout` provenance so search and word grouping stay aligned.

Line grouping uses existing baseline and XY-cut logic. The Prompt 06 model does not change the flat text extractor.

## Paragraphs

Paragraphs are grouped from consecutive lines using:

- line gap
- indentation delta
- role consistency

Paragraph confidence is intentionally conservative because untagged PDF paragraph boundaries are heuristic.

## Headers, Footers, Captions, Footnotes

The Prompt 06 model marks candidates only:

- short repeated-looking blocks near top or bottom can become header/footer candidates
- small low-page text can become footnote candidates
- `Figure`, `Fig.`, and `Table` prefixes can become caption candidates
- bullet and numbered prefixes can become list candidates

These roles do not remove text from extraction.

## Bidi, CJK, And Vertical

- RTL flags from `TextChunk` are preserved into lines, spans, and characters.
- CJK no-space text is tokenized at character level when no dictionary segmenter exists.
- Vertical writing is kept separate from rotated horizontal text through the font writing-mode signal.

Known limit: full Unicode Bidirectional Algorithm output modes and dictionary CJK segmentation are not added in Prompt 06.
