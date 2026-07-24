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
typedef struct OxideSignatureValidationOptions OxideSignatureValidationOptions;
typedef struct OxideSignatureTrustStore OxideSignatureTrustStore;
typedef struct OxideSignatureIntermediateStore OxideSignatureIntermediateStore;
typedef struct OxideSignatureEvidenceStore OxideSignatureEvidenceStore;
typedef struct OxideSignatureRetrievalPolicy OxideSignatureRetrievalPolicy;
typedef struct OxideSignatureValidationCancellation OxideSignatureValidationCancellation;

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

/* Opens a public-key encrypted PDF from bytes with explicit certificate and
 * private-key buffers. Certificate and private key may be PEM or DER. The
 * private key buffer is read only for the duration of this call and is not
 * retained by the C ABI wrapper. */
OXIDE_API OxideDocument *oxide_document_open_pubsec_from_bytes(
    const uint8_t *data,
    size_t len,
    const uint8_t *certificate,
    size_t certificate_len,
    const uint8_t *private_key,
    size_t private_key_len,
    char **error_out);

/* Opens a public-key encrypted PDF from bytes with a PKCS #12/PFX provider
 * bundle and explicit password bytes. The PFX and password buffers are read
 * only for the duration of this call and are not retained or serialized. */
OXIDE_API OxideDocument *oxide_document_open_pubsec_pfx_from_bytes(
    const uint8_t *data,
    size_t len,
    const uint8_t *pfx,
    size_t pfx_len,
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

/* Encrypts an opened document with /Adobe.PubSec for one recipient
 * certificate. The certificate may be PEM or DER. Multi-recipient workflows
 * are available through Rust/CLI/Python; this C ABI entry point is deliberately
 * single-recipient to keep ownership and buffer lifetimes explicit. */
OXIDE_API int oxide_document_pubsec_encrypt_pdf(
    const OxideDocument *document,
    const uint8_t *recipient_certificate,
    size_t recipient_certificate_len,
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

/* Prompt 24 signature-validation handles. All stores are owned and explicit:
 * only an OxideSignatureTrustStore grants anchor trust; intermediate and
 * evidence stores remain untrusted inputs until the shared validator proves
 * their applicability. All byte inputs are copied during the call; no caller
 * buffers are retained. Retrieval policy starts offline. */
OXIDE_API OxideSignatureTrustStore *oxide_signature_trust_store_new(
    char **error_out);
OXIDE_API void oxide_signature_trust_store_free(
    OxideSignatureTrustStore *store);
OXIDE_API int oxide_signature_trust_store_add_anchor_der(
    OxideSignatureTrustStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_trust_store_add_distrusted_certificate_sha256(
    OxideSignatureTrustStore *store,
    const char *fingerprint,
    char **error_out);

OXIDE_API OxideSignatureIntermediateStore *oxide_signature_intermediate_store_new(
    char **error_out);
OXIDE_API void oxide_signature_intermediate_store_free(
    OxideSignatureIntermediateStore *store);
OXIDE_API int oxide_signature_intermediate_store_add_der(
    OxideSignatureIntermediateStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);

OXIDE_API OxideSignatureEvidenceStore *oxide_signature_evidence_store_new(
    char **error_out);
OXIDE_API void oxide_signature_evidence_store_free(
    OxideSignatureEvidenceStore *store);
OXIDE_API int oxide_signature_evidence_store_add_ocsp_der(
    OxideSignatureEvidenceStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_evidence_store_add_crl_der(
    OxideSignatureEvidenceStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_evidence_store_set_bundle_json(
    OxideSignatureEvidenceStore *store,
    const char *bundle_json,
    char **error_out);

OXIDE_API OxideSignatureRetrievalPolicy *oxide_signature_retrieval_policy_new(
    char **error_out);
OXIDE_API void oxide_signature_retrieval_policy_free(
    OxideSignatureRetrievalPolicy *policy);
OXIDE_API int oxide_signature_retrieval_policy_set_json(
    OxideSignatureRetrievalPolicy *policy,
    const char *policy_json,
    char **error_out);

OXIDE_API OxideSignatureValidationCancellation *oxide_signature_validation_cancellation_new(
    char **error_out);
OXIDE_API int oxide_signature_validation_cancellation_cancel(
    const OxideSignatureValidationCancellation *cancellation,
    char **error_out);
OXIDE_API void oxide_signature_validation_cancellation_free(
    OxideSignatureValidationCancellation *cancellation);

OXIDE_API OxideSignatureValidationOptions *oxide_signature_validation_options_new(
    char **error_out);
OXIDE_API void oxide_signature_validation_options_free(
    OxideSignatureValidationOptions *options);
OXIDE_API int oxide_signature_validation_options_apply_trust_store(
    OxideSignatureValidationOptions *options,
    const OxideSignatureTrustStore *store,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_apply_intermediate_store(
    OxideSignatureValidationOptions *options,
    const OxideSignatureIntermediateStore *store,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_apply_evidence_store(
    OxideSignatureValidationOptions *options,
    const OxideSignatureEvidenceStore *store,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_apply_retrieval_policy(
    OxideSignatureValidationOptions *options,
    const OxideSignatureRetrievalPolicy *policy,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_cancellation(
    OxideSignatureValidationOptions *options,
    const OxideSignatureValidationCancellation *cancellation,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_add_trust_anchor_der(
    OxideSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_add_intermediate_der(
    OxideSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
/* Adds a SHA-256 certificate fingerprint to the selected-path deny list. */
OXIDE_API int oxide_signature_validation_options_add_distrusted_certificate_sha256(
    OxideSignatureValidationOptions *options,
    const char *fingerprint,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_add_ocsp_der(
    OxideSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_add_crl_der(
    OxideSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_validation_time_unix(
    OxideSignatureValidationOptions *options,
    uint64_t validation_time_unix,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_clear_validation_time(
    OxideSignatureValidationOptions *options,
    char **error_out);
/* mode: 0 = not checked, 1 = offline strict, 2 = offline best effort,
 * 3 = online strict, 4 = online best effort. Online modes still require an
 * explicit bounded retrieval policy and never enable network access alone. */
OXIDE_API int oxide_signature_validation_options_set_revocation_mode(
    OxideSignatureValidationOptions *options,
    int mode,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_retrieval_policy_json(
    OxideSignatureValidationOptions *options,
    const char *policy_json,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_algorithm_policy_json(
    OxideSignatureValidationOptions *options,
    const char *policy_json,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_evidence_bundle_json(
    OxideSignatureValidationOptions *options,
    const char *bundle_json,
    char **error_out);
OXIDE_API int oxide_signature_validation_options_set_path_limits(
    OxideSignatureValidationOptions *options,
    size_t max_chain_depth,
    size_t max_path_candidates,
    char **error_out);
OXIDE_API int oxide_document_signatures_with_options_handle(
    const OxideDocument *document,
    const OxideSignatureValidationOptions *options,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_signature_validation_with_evidence_handle(
    const OxideDocument *document,
    const OxideSignatureValidationOptions *options,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_timestamp_token_validation_json(
    const uint8_t *token,
    size_t token_len,
    const uint8_t *signature_value,
    size_t signature_value_len,
    const char *options_json,
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

/* Prompt 26 clause-mapped standards validation reports. `target` may be NULL
 * (detected/claimed profile) or a label such as "PDF/A-2B", "PDF/UA-1",
 * "PDF/X-4". Each returns a versioned JSON envelope; free with
 * oxide_string_free. */
OXIDE_API int oxide_document_pdfa_standards_json(
    const OxideDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pdfua_standards_json(
    const OxideDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pdfx_standards_json(
    const OxideDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_standards_all_json(
    const OxideDocument *document,
    const char *target,
    char **out_json,
    char **error_out);

/* Prompt 26 append-only incremental signing. `key_pem`/`cert_pem` are the
 * signer material (never logged). `certify` in 1..=3 creates a certification
 * (DocMDP) signature; any other value creates an approval signature. The plan
 * function writes no output; the sign function returns the signed PDF via
 * out_buffer (free with oxide_buffer_free) and an IncrementalSignResult JSON
 * via out_json (free with oxide_string_free). `field_name`/`reason` may be
 * NULL. The signed PDF is reopened and validated before it is returned. */
OXIDE_API int oxide_document_sign_plan_json(
    const OxideDocument *document,
    const char *key_pem,
    const char *cert_pem,
    size_t placeholder_size,
    int certify,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_sign_pdf(
    const OxideDocument *document,
    const char *key_pem,
    const char *cert_pem,
    size_t placeholder_size,
    int certify,
    const char *field_name,
    const char *reason,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Prompt 16 bounded XFA packet/static/script/security reports. */
OXIDE_API int oxide_document_xfa_report_json(
    const OxideDocument *document, char **out_json, char **error_out);
OXIDE_API int oxide_document_xfa_extract_json(
    const OxideDocument *document, char **out_json, char **error_out);
OXIDE_API int oxide_document_xfa_script_report_json(
    const OxideDocument *document, char **out_json, char **error_out);
OXIDE_API int oxide_document_xfa_security_report_json(
    const OxideDocument *document, char **out_json, char **error_out);
OXIDE_API int oxide_document_xfa_runtime_report_json(
    const OxideDocument *document,
    const char *script_policy,
    int execute_events,
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

/* Prompt 15 provenance-aware RAG chunks. */
OXIDE_API int oxide_document_advanced_chunks_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 15 semantic model, tables, tokens, search metadata, and RAG chunks. */
OXIDE_API int oxide_document_semantic_bundle_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 15 provenance-aware semantic and dictionary-token search. */
OXIDE_API int oxide_document_semantic_search_json(
    const OxideDocument *document,
    const char *query,
    char **out_json,
    char **error_out);

/* Prompt 16 output operations. Returned buffers and JSON strings are owned by
 * the caller and must be freed with oxide_buffer_free/oxide_string_free. */
OXIDE_API int oxide_document_xfa_render_json(
    const OxideDocument *document,
    const char *script_policy,
    int execute_events,
    uint32_t dpi,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_xfa_flatten_json(
    const OxideDocument *document,
    const char *mode,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_xfa_sanitize_json(
    const OxideDocument *document,
    const char *mode,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Prompt 17 read-only annotation/media/redaction reports. JSON option strings
 * are UTF-8 and NUL-terminated; options_json may be NULL only where noted. */
OXIDE_API int oxide_document_rich_media_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt17_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt18_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt18b_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20b_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt21_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt21_raster_vector_report_json(
    const OxideDocument *document,
    size_t page,
    const char *options_json,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt21_font_reconstruction_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_prompt21_history_report_json(
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt21_object_stream_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt21_pack_object_streams_pdf(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt22_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt23_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_writer_determinism_audit_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_writer_external_diff_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_writer_closeout_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pubsec_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_aes_gcm_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pdf_mac_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pdf_mac_verify_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_pdf_mac_create_pdf(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_crypto_tamper_test_json(
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt22_optimize_pdf(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_prompt22_office_inspect_json(
    const uint8_t *data,
    uintptr_t len,
    const char *format,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_prompt22_office_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    const char *format,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20b_text_range_analyze_json(
    const OxideDocument *document,
    size_t page,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20b_text_range_edit_json(
    const OxideDocument *document,
    const char *request_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20_vector_list_json(
    const OxideDocument *document,
    size_t page,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20_text_edit_json(
    const OxideDocument *document,
    size_t page,
    const char *old_text,
    const char *new_text,
    const char *mode,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20_vector_edit_json(
    const OxideDocument *document,
    size_t page,
    const char *stable_id,
    const char *operation_json,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_prompt20_ink_fit_json(
    const OxideDocument *document,
    size_t page,
    size_t annotation_index,
    const char *options_json,
    int signature_policy_override,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_mask_redaction_report_json(
    const OxideDocument *document,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_edit_policy_report_json(
    const OxideDocument *document,
    const char *operation,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_annotation_appearance_report_json(
    const OxideDocument *document,
    const char *options_json,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_nonaxis_redaction_plan_json(
    const OxideDocument *document,
    const char *options_json,
    char **out_json,
    char **error_out);

/* Prompt 17 output operations. `xfdf` is a byte buffer with an explicit
 * length; the other string inputs are NUL-terminated UTF-8. On success the
 * caller owns both `out_buffer` and `out_json` and must release them with
 * oxide_buffer_free and oxide_string_free. */
OXIDE_API int oxide_document_annotation_xfdf_export_json(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_annotation_xfdf_import_json(
    const OxideDocument *document,
    const uint8_t *xfdf,
    size_t xfdf_len,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_annotation_appearance_generate_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_rich_media_sanitize_json(
    const OxideDocument *document,
    const char *mode,
    const char *custom_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_rich_media_flatten_poster_json(
    const OxideDocument *document,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_nonaxis_redaction_apply_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_redact_image_mask_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_redact_inline_image_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_add_json(
    const OxideDocument *document,
    const uint8_t *payload,
    size_t payload_len,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_update_owner_json(
    const OxideDocument *document,
    const uint8_t *payload,
    size_t payload_len,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_remove_owner_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_incremental_form_edit_json(
    const OxideDocument *document,
    const char *field_name,
    const char *value,
    bool signature_policy_override,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_signature_preserving_form_plan_json(
    const OxideDocument *document,
    const char *field_name,
    const char *value,
    const char *options_json,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_signature_preserving_form_edit_json(
    const OxideDocument *document,
    const char *field_name,
    const char *value,
    const char *options_json,
    bool explicit_invalidation_override,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_incremental_annotation_edit_json(
    const OxideDocument *document,
    const char *options_json,
    bool signature_policy_override,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_incremental_page_property_edit_json(
    const OxideDocument *document,
    const char *options_json,
    bool signature_policy_override,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_extract_json(
    const OxideDocument *document,
    const char *stable_id,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_remove_json(
    const OxideDocument *document,
    const char *stable_ids_json,
    OxideBuffer *out_buffer,
    char **out_json,
    char **error_out);
OXIDE_API int oxide_document_associated_files_sanitize_json(
    const OxideDocument *document,
    const char *options_json,
    OxideBuffer *out_buffer,
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

/* Codec isolation probe/decode report. `filter` is a PDF stream filter name,
 * `data` points to the encoded stream bytes, and `policy` is
 * "in_process"|"isolated_preferred"|"isolated_required"|"report_only"|
 * "disabled" (NULL => "in_process"). Free `*out_json` with
 * oxide_string_free. */
OXIDE_API int oxide_codec_isolation_report_json(
    const char *filter,
    const uint8_t *data,
    size_t len,
    const char *policy,
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
