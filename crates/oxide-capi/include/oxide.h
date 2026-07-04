#ifndef OXIDE_H
#define OXIDE_H

#include <stddef.h>
#include <stdint.h>

#ifdef _WIN32
#  ifdef OXIDE_BUILDING_DLL
#    define OXIDE_API __declspec(dllexport)
#  else
#    define OXIDE_API __declspec(dllimport)
#  endif
#else
#  define OXIDE_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

enum {
  OXIDE_STATUS_OK = 0,
  OXIDE_STATUS_NULL = 1,
  OXIDE_STATUS_ERROR = 2,
  OXIDE_STATUS_PANIC = 3
};

typedef struct OxideDocument OxideDocument;

typedef struct OxideBuffer {
  uint8_t *data;
  size_t len;
} OxideBuffer;

/* --- OCR backend (pluggable seam) ---------------------------------------- */

/* Sink Oxide passes to your `recognize`; call it once per recognized word.
 * `text` is a NUL-terminated UTF-8 string owned by the caller for the duration
 * of the call. `bbox` is [x0,y0,x1,y1] in image-pixel space (y-down, the same
 * frame as `gray`). `line_id` groups words into text lines; pass a negative
 * value if unknown. */
typedef void (*OxideOcrEmitWordFn)(
    void *sink,
    const char *text,
    double x0,
    double y0,
    double x1,
    double y1,
    float confidence,
    int32_t line_id);

/* You implement this. Return 0 on success, non-zero to signal a recognition
 * failure (Oxide degrades that page to the placeholder). `gray` is
 * width*height 8-bit grayscale, row-major, top-left origin. Report each word by
 * calling `emit(sink, ...)`. */
typedef int (*OxideOcrRecognizeFn)(
    void *userdata,
    const uint8_t *gray,
    uint32_t width,
    uint32_t height,
    uint32_t dpi,
    void *sink,
    OxideOcrEmitWordFn emit);

/* Backend descriptor passed to `oxide_document_set_ocr_backend`. */
typedef struct OxideOcrBackend {
  void *userdata;                  /* opaque, passed back to recognize        */
  OxideOcrRecognizeFn recognize;   /* required; NULL clears the backend       */
  uint32_t max_concurrency;        /* 0 => 1; pages OCR'd in parallel up to N  */
  const char *name;                /* optional provenance label; may be NULL   */
} OxideOcrBackend;

OXIDE_API OxideDocument *oxide_document_open_from_bytes(
    const uint8_t *data,
    size_t len,
    char **error_out);

/* Opens a document from bytes with an optional UTF-8 password.
 * password == NULL && password_len == 0 means no password was supplied.
 * password != NULL && password_len == 0 means an explicit empty password.
 * The password buffer is read only for the duration of this call and is not
 * retained by the C ABI wrapper. */
OXIDE_API OxideDocument *oxide_document_open_from_bytes_with_password(
    const uint8_t *data,
    size_t len,
    const uint8_t *password,
    size_t password_len,
    char **error_out);

OXIDE_API void oxide_document_free(OxideDocument *document);

OXIDE_API void oxide_string_free(char *value);

OXIDE_API void oxide_error_free(char *value);

OXIDE_API void oxide_buffer_free(OxideBuffer buffer);

OXIDE_API int oxide_document_page_count(
    const OxideDocument *document,
    size_t *out_count,
    char **error_out);

OXIDE_API int oxide_document_extract_text(
    const OxideDocument *document,
    size_t page,
    char **out_text,
    char **error_out);

OXIDE_API int oxide_document_parse_markdown(
    const OxideDocument *document,
    char **out_markdown,
    char **error_out);

OXIDE_API int oxide_document_parse_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Registers a C-function-pointer OCR backend on the document. The
 * `*_ocr` parse functions then route scanned pages through it. Pass a backend
 * whose `recognize` is NULL to clear a previously-registered backend. The
 * function pointers and `userdata` must stay valid until the document is freed
 * or the backend is cleared/replaced. */
OXIDE_API int oxide_document_set_ocr_backend(
    OxideDocument *document,
    OxideOcrBackend backend,
    char **error_out);

/* Parse to canonical Markdown WITH OCR for scanned pages, using the backend
 * registered via oxide_document_set_ocr_backend. With no backend registered,
 * behaves like oxide_document_parse_markdown (scanned pages → placeholder). */
OXIDE_API int oxide_document_parse_markdown_ocr(
    const OxideDocument *document,
    char **out_markdown,
    char **error_out);

/* Parse to canonical JSON WITH OCR for scanned pages. See above. */
OXIDE_API int oxide_document_parse_json_ocr(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_extract_fields_json(
    const OxideDocument *document,
    const char *doc_type,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_extract_semantic_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_info_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_render_page_png(
    const OxideDocument *document,
    size_t page,
    uint32_t dpi,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_render_page_jpeg(
    const OxideDocument *document,
    size_t page,
    uint32_t dpi,
    uint8_t quality,
    OxideBuffer *out_buffer,
    char **error_out);

/* Ordered page extraction/organization. `pages` are 1-based and duplicates are
 * kept. Pass NULL/0 for all pages. */
OXIDE_API int oxide_document_extract_pages_pdf(
    const OxideDocument *document,
    const size_t *pages,
    size_t pages_len,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_organize_pdf(
    const OxideDocument *document,
    const size_t *pages,
    size_t pages_len,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_rotate_pdf(
    const OxideDocument *document,
    const size_t *pages,
    size_t pages_len,
    int angle,
    int relative,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_optimize_pdf(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_linearize_pdf(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_decrypt_pdf(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_encrypt_aes256_pdf(
    const OxideDocument *document,
    const char *user_password,
    const char *owner_password,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_to_html(
    const OxideDocument *document,
    char **out_html,
    char **error_out);

OXIDE_API int oxide_document_to_xlsx(
    const OxideDocument *document,
    const char *layout,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_to_pptx(
    const OxideDocument *document,
    int include_images,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_to_docx(
    const OxideDocument *document,
    int include_images,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_docx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_xlsx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_pptx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_fonts_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_signatures_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

OXIDE_API int oxide_document_watermark_text_pdf(
    const OxideDocument *document,
    const char *text,
    double opacity,
    double rotation_degrees,
    double font_size,
    OxideBuffer *out_buffer,
    char **error_out);

OXIDE_API int oxide_document_add_page_numbers_pdf(
    const OxideDocument *document,
    const char *format,
    OxideBuffer *out_buffer,
    char **error_out);

/* Build a PDF from JPG/PNG image byte buffers. */
OXIDE_API int oxide_images_to_pdf(
    const uint8_t *const *images,
    const size_t *lengths,
    size_t count,
    OxideBuffer *out_buffer,
    char **error_out);

/* Merge PDF byte buffers in order. */
OXIDE_API int oxide_merge_pdfs_from_bytes(
    const uint8_t *const *inputs,
    const size_t *lengths,
    size_t count,
    OxideBuffer *out_buffer,
    char **error_out);

/* --- Report surfaces (versioned-JSON envelopes) --------------------------- *
 *
 * Every function below returns a versioned-JSON envelope string of the shape
 *
 *     {"schema_version": <int>, "kind": "<report kind>", "report": { ... }}
 *
 * through `out_json`. These are backed by the shared `oxide_engine::sdk` facade,
 * so the JSON is byte-identical to what the Python bindings return for the same
 * document. Ownership / lifetime / null / thread-safety rules:
 *
 *   - All functions return an int status code: OXIDE_STATUS_OK (0) on success,
 *     OXIDE_STATUS_ERROR (2) on a handled error (message in *error_out),
 *     OXIDE_STATUS_PANIC (3) on an internal panic (never UB).
 *   - `document` must be a valid handle from oxide_document_open_from_bytes
 *     or oxide_document_open_from_bytes_with_password.
 *     A null handle yields OXIDE_STATUS_ERROR and a message; never a crash.
 *   - On success, `*out_json` is a heap-allocated NUL-terminated UTF-8 string
 *     OWNED BY THE CALLER: free it with oxide_string_free. On error it is left
 *     untouched (still null if you initialized it so).
 *   - `*error_out` (if non-null on entry) receives an owned message on error;
 *     free it with oxide_error_free. It is cleared to null on success.
 *   - Output-producing operations additionally write the produced PDF to an
 *     OxideBuffer OWNED BY THE CALLER: free it with oxide_buffer_free.
 *   - Documents are Send + Sync; a single handle may be read concurrently from
 *     multiple threads. These report calls do not mutate the handle. Do not,
 *     however, free a handle while another thread is using it.
 *   - String parameters marked "may be NULL" fall back to a documented default.
 */

/* Security report: encryption, signatures, risky active content, findings. */
OXIDE_API int oxide_document_security_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Parser diagnostics. `mode` is "strict"|"repair"|"audit" (NULL => "repair"). */
OXIDE_API int oxide_document_parser_report_json(
    const OxideDocument *document,
    const char *mode,
    char **out_json,
    char **error_out);

/* Color / prepress report. `profile` is "generic"|"pdfa"|"pdfx" (NULL =>
 * "generic"). */
OXIDE_API int oxide_document_color_report_json(
    const OxideDocument *document,
    const char *profile,
    char **out_json,
    char **error_out);

/* Standards-profile validation. `profile` is "pdfa"|"pdfua"|"pdfx"|"security"|
 * "all" (NULL => "all"). */
OXIDE_API int oxide_document_validate_json(
    const OxideDocument *document,
    const char *profile,
    char **out_json,
    char **error_out);

/* AcroForm field inventory. */
OXIDE_API int oxide_document_forms_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Annotation inventory. */
OXIDE_API int oxide_document_annotations_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Page-operations report (boxes, labels, destinations, preservation risk). */
OXIDE_API int oxide_document_pages_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Combined interactive report (forms + annotations + page operations). */
OXIDE_API int oxide_document_interactive_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* RAG-ready semantic chunk set. */
OXIDE_API int oxide_document_chunks_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Sanitize. `policy` is "strict"|"balanced"|"preserve-visual" (NULL =>
 * "balanced"). Writes the sanitized PDF to `out_buffer` and a JSON report to
 * `out_json`. Free both. */
OXIDE_API int oxide_document_sanitize_json(
    const OxideDocument *document,
    const char *policy,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Canonicalize deterministically. Set `has_date_epoch` non-zero to fix the
 * source date epoch to `date_epoch`; pass 0 to leave it unset. Writes the
 * canonical PDF to `out_buffer` and an audit JSON report to `out_json`. */
OXIDE_API int oxide_document_canonicalize_json(
    const OxideDocument *document,
    int64_t date_epoch,
    int has_date_epoch,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Redact every occurrence of the NUL-terminated UTF-8 strings in `terms`
 * (case-insensitive), full-rewrite, and verify absence. `strict` non-zero fails
 * the call if any term survives. Writes the redacted PDF to `out_buffer` and a
 * JSON report (with verification) to `out_json`. */
OXIDE_API int oxide_document_redact_terms_json(
    const OxideDocument *document,
    const char *const *terms,
    size_t terms_len,
    int strict,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* --- Version / capability query (no document needed) ---------------------- */

/* SDK feature / capability report JSON: engine version, envelope version, and
 * compiled capabilities. Free `*out_json` with oxide_string_free. */
OXIDE_API int oxide_feature_report_json(
    char **out_json,
    char **error_out);

/* The oxide-engine semantic version as a NUL-terminated string owned by the
 * caller (free with oxide_string_free). NULL only on allocation failure. */
OXIDE_API char *oxide_version(void);

/* The C-ABI report envelope version. Bumps signal an envelope-shape change. */
OXIDE_API uint32_t oxide_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif
