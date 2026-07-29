#ifndef WELLFRIENDPDF_H
#define WELLFRIENDPDF_H

#include <stddef.h>
#include <stdint.h>

#ifdef _WIN32
#  ifdef WELLFRIENDPDF_BUILDING_DLL
#    define WELLFRIENDPDF_API __declspec(dllexport)
#  else
#    define WELLFRIENDPDF_API __declspec(dllimport)
#  endif
#else
#  define WELLFRIENDPDF_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

enum {
  WELLFRIENDPDF_STATUS_OK = 0,
  WELLFRIENDPDF_STATUS_NULL = 1,
  WELLFRIENDPDF_STATUS_ERROR = 2,
  WELLFRIENDPDF_STATUS_PANIC = 3
};

typedef struct WellfriendDocument WellfriendDocument;
typedef struct WellfriendSignatureValidationOptions WellfriendSignatureValidationOptions;
typedef struct WellfriendSignatureTrustStore WellfriendSignatureTrustStore;
typedef struct WellfriendSignatureIntermediateStore WellfriendSignatureIntermediateStore;
typedef struct WellfriendSignatureEvidenceStore WellfriendSignatureEvidenceStore;
typedef struct WellfriendSignatureRetrievalPolicy WellfriendSignatureRetrievalPolicy;
typedef struct WellfriendSignatureValidationCancellation WellfriendSignatureValidationCancellation;

typedef struct WellfriendBuffer {
  uint8_t *data;
  size_t len;
} WellfriendBuffer;

/* --- OCR backend (pluggable seam) ---------------------------------------- */

/* Sink Wellfriend passes to your `recognize`; call it once per recognized word.
 * `text` is a NUL-terminated UTF-8 string owned by the caller for the duration
 * of the call. `bbox` is [x0,y0,x1,y1] in image-pixel space (y-down, the same
 * frame as `gray`). `line_id` groups words into text lines; pass a negative
 * value if unknown. */
typedef void (*WellfriendOcrEmitWordFn)(
    void *sink,
    const char *text,
    double x0,
    double y0,
    double x1,
    double y1,
    float confidence,
    int32_t line_id);

/* You implement this. Return 0 on success, non-zero to signal a recognition
 * failure (Wellfriend degrades that page to the placeholder). `gray` is
 * width*height 8-bit grayscale, row-major, top-left origin. Report each word by
 * calling `emit(sink, ...)`. */
typedef int (*WellfriendOcrRecognizeFn)(
    void *userdata,
    const uint8_t *gray,
    uint32_t width,
    uint32_t height,
    uint32_t dpi,
    void *sink,
    WellfriendOcrEmitWordFn emit);

/* Backend descriptor passed to `wellfriendpdf_document_set_ocr_backend`. */
typedef struct WellfriendOcrBackend {
  void *userdata;                  /* opaque, passed back to recognize        */
  WellfriendOcrRecognizeFn recognize;   /* required; NULL clears the backend       */
  uint32_t max_concurrency;        /* 0 => 1; pages OCR'd in parallel up to N  */
  const char *name;                /* optional provenance label; may be NULL   */
} WellfriendOcrBackend;

WELLFRIENDPDF_API WellfriendDocument *wellfriendpdf_document_open_from_bytes(
    const uint8_t *data,
    size_t len,
    char **error_out);

/* Opens a document from bytes with an optional UTF-8 password.
 * password == NULL && password_len == 0 means no password was supplied.
 * password != NULL && password_len == 0 means an explicit empty password.
 * The password buffer is read only for the duration of this call and is not
 * retained by the C ABI wrapper. */
WELLFRIENDPDF_API WellfriendDocument *wellfriendpdf_document_open_from_bytes_with_password(
    const uint8_t *data,
    size_t len,
    const uint8_t *password,
    size_t password_len,
    char **error_out);

/* Opens a public-key encrypted PDF from bytes with explicit certificate and
 * private-key buffers. Certificate and private key may be PEM or DER. The
 * private key buffer is read only for the duration of this call and is not
 * retained by the C ABI wrapper. */
WELLFRIENDPDF_API WellfriendDocument *wellfriendpdf_document_open_pubsec_from_bytes(
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
WELLFRIENDPDF_API WellfriendDocument *wellfriendpdf_document_open_pubsec_pfx_from_bytes(
    const uint8_t *data,
    size_t len,
    const uint8_t *pfx,
    size_t pfx_len,
    const uint8_t *password,
    size_t password_len,
    char **error_out);

WELLFRIENDPDF_API void wellfriendpdf_document_free(WellfriendDocument *document);

WELLFRIENDPDF_API void wellfriendpdf_string_free(char *value);

WELLFRIENDPDF_API void wellfriendpdf_error_free(char *value);

WELLFRIENDPDF_API void wellfriendpdf_buffer_free(WellfriendBuffer buffer);

WELLFRIENDPDF_API int wellfriendpdf_document_page_count(
    const WellfriendDocument *document,
    size_t *out_count,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_extract_text(
    const WellfriendDocument *document,
    size_t page,
    char **out_text,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_parse_markdown(
    const WellfriendDocument *document,
    char **out_markdown,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_parse_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Registers a C-function-pointer OCR backend on the document. The
 * `*_ocr` parse functions then route scanned pages through it. Pass a backend
 * whose `recognize` is NULL to clear a previously-registered backend. The
 * function pointers and `userdata` must stay valid until the document is freed
 * or the backend is cleared/replaced. */
WELLFRIENDPDF_API int wellfriendpdf_document_set_ocr_backend(
    WellfriendDocument *document,
    WellfriendOcrBackend backend,
    char **error_out);

/* Parse to canonical Markdown WITH OCR for scanned pages, using the backend
 * registered via wellfriendpdf_document_set_ocr_backend. With no backend registered,
 * behaves like wellfriendpdf_document_parse_markdown (scanned pages → placeholder). */
WELLFRIENDPDF_API int wellfriendpdf_document_parse_markdown_ocr(
    const WellfriendDocument *document,
    char **out_markdown,
    char **error_out);

/* Parse to canonical JSON WITH OCR for scanned pages. See above. */
WELLFRIENDPDF_API int wellfriendpdf_document_parse_json_ocr(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_extract_fields_json(
    const WellfriendDocument *document,
    const char *doc_type,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_extract_semantic_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_info_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_render_page_png(
    const WellfriendDocument *document,
    size_t page,
    uint32_t dpi,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_render_page_jpeg(
    const WellfriendDocument *document,
    size_t page,
    uint32_t dpi,
    uint8_t quality,
    WellfriendBuffer *out_buffer,
    char **error_out);

/* Ordered page extraction/organization. `pages` are 1-based and duplicates are
 * kept. Pass NULL/0 for all pages. */
WELLFRIENDPDF_API int wellfriendpdf_document_extract_pages_pdf(
    const WellfriendDocument *document,
    const size_t *pages,
    size_t pages_len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_organize_pdf(
    const WellfriendDocument *document,
    const size_t *pages,
    size_t pages_len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_rotate_pdf(
    const WellfriendDocument *document,
    const size_t *pages,
    size_t pages_len,
    int angle,
    int relative,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_optimize_pdf(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_linearize_pdf(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_decrypt_pdf(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **error_out);

/* Encrypts an opened document with /Adobe.PubSec for one recipient
 * certificate. The certificate may be PEM or DER. Multi-recipient workflows
 * are available through Rust/CLI/Python; this C ABI entry point is deliberately
 * single-recipient to keep ownership and buffer lifetimes explicit. */
WELLFRIENDPDF_API int wellfriendpdf_document_pubsec_encrypt_pdf(
    const WellfriendDocument *document,
    const uint8_t *recipient_certificate,
    size_t recipient_certificate_len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_encrypt_aes256_pdf(
    const WellfriendDocument *document,
    const char *user_password,
    const char *owner_password,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_to_html(
    const WellfriendDocument *document,
    char **out_html,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_to_xlsx(
    const WellfriendDocument *document,
    const char *layout,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_to_pptx(
    const WellfriendDocument *document,
    int include_images,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_to_docx(
    const WellfriendDocument *document,
    int include_images,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_docx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_xlsx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_pptx_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_fonts_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_signatures_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 24 signature-validation handles. All stores are owned and explicit:
 * only an WellfriendSignatureTrustStore grants anchor trust; intermediate and
 * evidence stores remain untrusted inputs until the shared validator proves
 * their applicability. All byte inputs are copied during the call; no caller
 * buffers are retained. Retrieval policy starts offline. */
WELLFRIENDPDF_API WellfriendSignatureTrustStore *wellfriendpdf_signature_trust_store_new(
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_trust_store_free(
    WellfriendSignatureTrustStore *store);
WELLFRIENDPDF_API int wellfriendpdf_signature_trust_store_add_anchor_der(
    WellfriendSignatureTrustStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_trust_store_add_distrusted_certificate_sha256(
    WellfriendSignatureTrustStore *store,
    const char *fingerprint,
    char **error_out);

WELLFRIENDPDF_API WellfriendSignatureIntermediateStore *wellfriendpdf_signature_intermediate_store_new(
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_intermediate_store_free(
    WellfriendSignatureIntermediateStore *store);
WELLFRIENDPDF_API int wellfriendpdf_signature_intermediate_store_add_der(
    WellfriendSignatureIntermediateStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);

WELLFRIENDPDF_API WellfriendSignatureEvidenceStore *wellfriendpdf_signature_evidence_store_new(
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_evidence_store_free(
    WellfriendSignatureEvidenceStore *store);
WELLFRIENDPDF_API int wellfriendpdf_signature_evidence_store_add_ocsp_der(
    WellfriendSignatureEvidenceStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_evidence_store_add_crl_der(
    WellfriendSignatureEvidenceStore *store,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_evidence_store_set_bundle_json(
    WellfriendSignatureEvidenceStore *store,
    const char *bundle_json,
    char **error_out);

WELLFRIENDPDF_API WellfriendSignatureRetrievalPolicy *wellfriendpdf_signature_retrieval_policy_new(
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_retrieval_policy_free(
    WellfriendSignatureRetrievalPolicy *policy);
WELLFRIENDPDF_API int wellfriendpdf_signature_retrieval_policy_set_json(
    WellfriendSignatureRetrievalPolicy *policy,
    const char *policy_json,
    char **error_out);

WELLFRIENDPDF_API WellfriendSignatureValidationCancellation *wellfriendpdf_signature_validation_cancellation_new(
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_cancellation_cancel(
    const WellfriendSignatureValidationCancellation *cancellation,
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_validation_cancellation_free(
    WellfriendSignatureValidationCancellation *cancellation);

WELLFRIENDPDF_API WellfriendSignatureValidationOptions *wellfriendpdf_signature_validation_options_new(
    char **error_out);
WELLFRIENDPDF_API void wellfriendpdf_signature_validation_options_free(
    WellfriendSignatureValidationOptions *options);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_apply_trust_store(
    WellfriendSignatureValidationOptions *options,
    const WellfriendSignatureTrustStore *store,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_apply_intermediate_store(
    WellfriendSignatureValidationOptions *options,
    const WellfriendSignatureIntermediateStore *store,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_apply_evidence_store(
    WellfriendSignatureValidationOptions *options,
    const WellfriendSignatureEvidenceStore *store,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_apply_retrieval_policy(
    WellfriendSignatureValidationOptions *options,
    const WellfriendSignatureRetrievalPolicy *policy,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_cancellation(
    WellfriendSignatureValidationOptions *options,
    const WellfriendSignatureValidationCancellation *cancellation,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_add_trust_anchor_der(
    WellfriendSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_add_intermediate_der(
    WellfriendSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
/* Adds a SHA-256 certificate fingerprint to the selected-path deny list. */
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_add_distrusted_certificate_sha256(
    WellfriendSignatureValidationOptions *options,
    const char *fingerprint,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_add_ocsp_der(
    WellfriendSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_add_crl_der(
    WellfriendSignatureValidationOptions *options,
    const uint8_t *data,
    size_t len,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_validation_time_unix(
    WellfriendSignatureValidationOptions *options,
    uint64_t validation_time_unix,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_clear_validation_time(
    WellfriendSignatureValidationOptions *options,
    char **error_out);
/* mode: 0 = not checked, 1 = offline strict, 2 = offline best effort,
 * 3 = online strict, 4 = online best effort. Online modes still require an
 * explicit bounded retrieval policy and never enable network access alone. */
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_revocation_mode(
    WellfriendSignatureValidationOptions *options,
    int mode,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_retrieval_policy_json(
    WellfriendSignatureValidationOptions *options,
    const char *policy_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_algorithm_policy_json(
    WellfriendSignatureValidationOptions *options,
    const char *policy_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_evidence_bundle_json(
    WellfriendSignatureValidationOptions *options,
    const char *bundle_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_signature_validation_options_set_path_limits(
    WellfriendSignatureValidationOptions *options,
    size_t max_chain_depth,
    size_t max_path_candidates,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_signatures_with_options_handle(
    const WellfriendDocument *document,
    const WellfriendSignatureValidationOptions *options,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_signature_validation_with_evidence_handle(
    const WellfriendDocument *document,
    const WellfriendSignatureValidationOptions *options,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_timestamp_token_validation_json(
    const uint8_t *token,
    size_t token_len,
    const uint8_t *signature_value,
    size_t signature_value_len,
    const char *options_json,
    char **out_json,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_watermark_text_pdf(
    const WellfriendDocument *document,
    const char *text,
    double opacity,
    double rotation_degrees,
    double font_size,
    WellfriendBuffer *out_buffer,
    char **error_out);

WELLFRIENDPDF_API int wellfriendpdf_document_add_page_numbers_pdf(
    const WellfriendDocument *document,
    const char *format,
    WellfriendBuffer *out_buffer,
    char **error_out);

/* Build a PDF from JPG/PNG image byte buffers. */
WELLFRIENDPDF_API int wellfriendpdf_images_to_pdf(
    const uint8_t *const *images,
    const size_t *lengths,
    size_t count,
    WellfriendBuffer *out_buffer,
    char **error_out);

/* Merge PDF byte buffers in order. */
WELLFRIENDPDF_API int wellfriendpdf_merge_pdfs_from_bytes(
    const uint8_t *const *inputs,
    const size_t *lengths,
    size_t count,
    WellfriendBuffer *out_buffer,
    char **error_out);

/* --- Report surfaces (versioned-JSON envelopes) --------------------------- *
 *
 * Every function below returns a versioned-JSON envelope string of the shape
 *
 *     {"schema_version": <int>, "kind": "<report kind>", "report": { ... }}
 *
 * through `out_json`. These are backed by the shared `wellfriendpdf_engine::sdk` facade,
 * so the JSON is byte-identical to what the Python bindings return for the same
 * document. Ownership / lifetime / null / thread-safety rules:
 *
 *   - All functions return an int status code: WELLFRIENDPDF_STATUS_OK (0) on success,
 *     WELLFRIENDPDF_STATUS_ERROR (2) on a handled error (message in *error_out),
 *     WELLFRIENDPDF_STATUS_PANIC (3) on an internal panic (never UB).
 *   - `document` must be a valid handle from wellfriendpdf_document_open_from_bytes
 *     or wellfriendpdf_document_open_from_bytes_with_password.
 *     A null handle yields WELLFRIENDPDF_STATUS_ERROR and a message; never a crash.
 *   - On success, `*out_json` is a heap-allocated NUL-terminated UTF-8 string
 *     OWNED BY THE CALLER: free it with wellfriendpdf_string_free. On error it is left
 *     untouched (still null if you initialized it so).
 *   - `*error_out` (if non-null on entry) receives an owned message on error;
 *     free it with wellfriendpdf_error_free. It is cleared to null on success.
 *   - Output-producing operations additionally write the produced PDF to an
 *     WellfriendBuffer OWNED BY THE CALLER: free it with wellfriendpdf_buffer_free.
 *   - Documents are Send + Sync; a single handle may be read concurrently from
 *     multiple threads. These report calls do not mutate the handle. Do not,
 *     however, free a handle while another thread is using it.
 *   - String parameters marked "may be NULL" fall back to a documented default.
 */

/* Security report: encryption, signatures, risky active content, findings. */
WELLFRIENDPDF_API int wellfriendpdf_document_security_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Parser diagnostics. `mode` is "strict"|"repair"|"audit" (NULL => "repair"). */
WELLFRIENDPDF_API int wellfriendpdf_document_parser_report_json(
    const WellfriendDocument *document,
    const char *mode,
    char **out_json,
    char **error_out);

/* Color / prepress report. `profile` is "generic"|"pdfa"|"pdfx" (NULL =>
 * "generic"). */
WELLFRIENDPDF_API int wellfriendpdf_document_color_report_json(
    const WellfriendDocument *document,
    const char *profile,
    char **out_json,
    char **error_out);

/* Standards-profile validation. `profile` is "pdfa"|"pdfua"|"pdfx"|"security"|
 * "all" (NULL => "all"). */
WELLFRIENDPDF_API int wellfriendpdf_document_validate_json(
    const WellfriendDocument *document,
    const char *profile,
    char **out_json,
    char **error_out);

/* AcroForm field inventory. */
WELLFRIENDPDF_API int wellfriendpdf_document_forms_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 26 clause-mapped standards validation reports. `target` may be NULL
 * (detected/claimed profile) or a label such as "PDF/A-2B", "PDF/UA-1",
 * "PDF/X-4". Each returns a versioned JSON envelope; free with
 * wellfriendpdf_string_free. */
WELLFRIENDPDF_API int wellfriendpdf_document_pdfa_standards_json(
    const WellfriendDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pdfua_standards_json(
    const WellfriendDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pdfx_standards_json(
    const WellfriendDocument *document,
    const char *target,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_standards_all_json(
    const WellfriendDocument *document,
    const char *target,
    char **out_json,
    char **error_out);

/* Prompt 26 append-only incremental signing. `key_pem`/`cert_pem` are the
 * signer material (never logged). `certify` in 1..=3 creates a certification
 * (DocMDP) signature; any other value creates an approval signature. The plan
 * function writes no output; the sign function returns the signed PDF via
 * out_buffer (free with wellfriendpdf_buffer_free) and an IncrementalSignResult JSON
 * via out_json (free with wellfriendpdf_string_free). `field_name`/`reason` may be
 * NULL. The signed PDF is reopened and validated before it is returned. */
WELLFRIENDPDF_API int wellfriendpdf_document_sign_plan_json(
    const WellfriendDocument *document,
    const char *key_pem,
    const char *cert_pem,
    size_t placeholder_size,
    int certify,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_sign_pdf(
    const WellfriendDocument *document,
    const char *key_pem,
    const char *cert_pem,
    size_t placeholder_size,
    int certify,
    const char *field_name,
    const char *reason,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Prompt 16 bounded XFA packet/static/script/security reports. */
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_report_json(
    const WellfriendDocument *document, char **out_json, char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_extract_json(
    const WellfriendDocument *document, char **out_json, char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_script_report_json(
    const WellfriendDocument *document, char **out_json, char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_security_report_json(
    const WellfriendDocument *document, char **out_json, char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_runtime_report_json(
    const WellfriendDocument *document,
    const char *script_policy,
    int execute_events,
    char **out_json,
    char **error_out);

/* Annotation inventory. */
WELLFRIENDPDF_API int wellfriendpdf_document_annotations_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Page-operations report (boxes, labels, destinations, preservation risk). */
WELLFRIENDPDF_API int wellfriendpdf_document_pages_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Combined interactive report (forms + annotations + page operations). */
WELLFRIENDPDF_API int wellfriendpdf_document_interactive_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* RAG-ready semantic chunk set. */
WELLFRIENDPDF_API int wellfriendpdf_document_chunks_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 15 provenance-aware RAG chunks. */
WELLFRIENDPDF_API int wellfriendpdf_document_advanced_chunks_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 15 semantic model, tables, tokens, search metadata, and RAG chunks. */
WELLFRIENDPDF_API int wellfriendpdf_document_semantic_bundle_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);

/* Prompt 15 provenance-aware semantic and dictionary-token search. */
WELLFRIENDPDF_API int wellfriendpdf_document_semantic_search_json(
    const WellfriendDocument *document,
    const char *query,
    char **out_json,
    char **error_out);

/* Prompt 16 output operations. Returned buffers and JSON strings are owned by
 * the caller and must be freed with wellfriendpdf_buffer_free/wellfriendpdf_string_free. */
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_render_json(
    const WellfriendDocument *document,
    const char *script_policy,
    int execute_events,
    uint32_t dpi,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_flatten_json(
    const WellfriendDocument *document,
    const char *mode,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_xfa_sanitize_json(
    const WellfriendDocument *document,
    const char *mode,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Prompt 17 read-only annotation/media/redaction reports. JSON option strings
 * are UTF-8 and NUL-terminated; options_json may be NULL only where noted. */
WELLFRIENDPDF_API int wellfriendpdf_document_rich_media_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt17_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt18_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt18b_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20b_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt21_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt21_raster_vector_report_json(
    const WellfriendDocument *document,
    size_t page,
    const char *options_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt21_font_reconstruction_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_prompt21_history_report_json(
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt21_object_stream_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt21_pack_object_streams_pdf(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt22_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt23_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_writer_determinism_audit_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_writer_external_diff_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_writer_closeout_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pubsec_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_aes_gcm_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pdf_mac_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pdf_mac_verify_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_pdf_mac_create_pdf(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_crypto_tamper_test_json(
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt22_optimize_pdf(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_prompt22_office_inspect_json(
    const uint8_t *data,
    uintptr_t len,
    const char *format,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_prompt22_office_to_pdf(
    const uint8_t *data,
    uintptr_t len,
    const char *format,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20b_text_range_analyze_json(
    const WellfriendDocument *document,
    size_t page,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20b_text_range_edit_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_provenance_json(
    const WellfriendDocument *document,
    size_t page,
    const char *source_text,
    const char *replacement_text,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_edit_eligibility_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_operator_text_edit_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_path_provenance_json(
    const WellfriendDocument *document,
    size_t page,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt31_path_edit_json(
    const WellfriendDocument *document,
    size_t page,
    const char *stable_id,
    const char *operation_json,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_scene_report_json(
    const WellfriendDocument *document,
    const char *pages_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_scene_select_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_transaction_plan_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_transaction_apply_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_text_map_json(
    const WellfriendDocument *document,
    const char *text,
    const char *direction,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_shape_text_json(
    const WellfriendDocument *document,
    const char *text,
    const char *direction,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_font_subset_plan_json(
    const WellfriendDocument *document,
    const char *text,
    const char *direction,
    const char *policy,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt32_font_substitution_report_json(
    const WellfriendDocument *document,
    const char *requested_family,
    const char *text,
    const char *policy,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_layout_analyze_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_semantic_layout_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reading_order_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_flow_graph_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reflow_preview_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_overflow_report_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_constraints_report_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_confidence_report_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
/* Validate an explicitly supplied Prompt 33 output against this immutable
 * source document. `output_pdf` remains caller-owned for the whole call. */
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_validate_reflow_output_json(
    const WellfriendDocument *document,
    const uint8_t *output_pdf,
    size_t output_pdf_len,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reflow_region_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reflow_document_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
/* Execute the canonical Prompt 33 undo by replaying `request_json` against
 * this immutable preimage and verifying `output_pdf` before restoring bytes. */
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_undo_reflow_json(
    const WellfriendDocument *document,
    const uint8_t *output_pdf,
    size_t output_pdf_len,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reflow_approve_structure_json(
    const WellfriendDocument *document,
    const char *correction_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt33_reflow_operation_report_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
/* Prompt 34 source-linked table, math, OCR, annotation, form, and XFA APIs. */
WELLFRIENDPDF_API int wellfriendpdf_document_prompt34_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt34_analyze_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt34_plan_json(
    const WellfriendDocument *document,
    const char *request_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt34_apply_json(
    const WellfriendDocument *document,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt34_undo_json(
    const WellfriendDocument *document,
    const uint8_t *output_pdf,
    size_t output_pdf_len,
    const char *request_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20_vector_list_json(
    const WellfriendDocument *document,
    size_t page,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20_text_edit_json(
    const WellfriendDocument *document,
    size_t page,
    const char *old_text,
    const char *new_text,
    const char *mode,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20_vector_edit_json(
    const WellfriendDocument *document,
    size_t page,
    const char *stable_id,
    const char *operation_json,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_prompt20_ink_fit_json(
    const WellfriendDocument *document,
    size_t page,
    size_t annotation_index,
    const char *options_json,
    int signature_policy_override,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_mask_redaction_report_json(
    const WellfriendDocument *document,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_edit_policy_report_json(
    const WellfriendDocument *document,
    const char *operation,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_annotation_appearance_report_json(
    const WellfriendDocument *document,
    const char *options_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_nonaxis_redaction_plan_json(
    const WellfriendDocument *document,
    const char *options_json,
    char **out_json,
    char **error_out);

/* Prompt 17 output operations. `xfdf` is a byte buffer with an explicit
 * length; the other string inputs are NUL-terminated UTF-8. On success the
 * caller owns both `out_buffer` and `out_json` and must release them with
 * wellfriendpdf_buffer_free and wellfriendpdf_string_free. */
WELLFRIENDPDF_API int wellfriendpdf_document_annotation_xfdf_export_json(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_annotation_xfdf_import_json(
    const WellfriendDocument *document,
    const uint8_t *xfdf,
    size_t xfdf_len,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_annotation_appearance_generate_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_rich_media_sanitize_json(
    const WellfriendDocument *document,
    const char *mode,
    const char *custom_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_rich_media_flatten_poster_json(
    const WellfriendDocument *document,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_nonaxis_redaction_apply_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_redact_image_mask_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_redact_inline_image_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_add_json(
    const WellfriendDocument *document,
    const uint8_t *payload,
    size_t payload_len,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_update_owner_json(
    const WellfriendDocument *document,
    const uint8_t *payload,
    size_t payload_len,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_remove_owner_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_incremental_form_edit_json(
    const WellfriendDocument *document,
    const char *field_name,
    const char *value,
    bool signature_policy_override,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_signature_preserving_form_plan_json(
    const WellfriendDocument *document,
    const char *field_name,
    const char *value,
    const char *options_json,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_signature_preserving_form_edit_json(
    const WellfriendDocument *document,
    const char *field_name,
    const char *value,
    const char *options_json,
    bool explicit_invalidation_override,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_incremental_annotation_edit_json(
    const WellfriendDocument *document,
    const char *options_json,
    bool signature_policy_override,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_incremental_page_property_edit_json(
    const WellfriendDocument *document,
    const char *options_json,
    bool signature_policy_override,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_extract_json(
    const WellfriendDocument *document,
    const char *stable_id,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_remove_json(
    const WellfriendDocument *document,
    const char *stable_ids_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);
WELLFRIENDPDF_API int wellfriendpdf_document_associated_files_sanitize_json(
    const WellfriendDocument *document,
    const char *options_json,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Sanitize. `policy` is "strict"|"balanced"|"preserve-visual" (NULL =>
 * "balanced"). Writes the sanitized PDF to `out_buffer` and a JSON report to
 * `out_json`. Free both. */
WELLFRIENDPDF_API int wellfriendpdf_document_sanitize_json(
    const WellfriendDocument *document,
    const char *policy,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Canonicalize deterministically. Set `has_date_epoch` non-zero to fix the
 * source date epoch to `date_epoch`; pass 0 to leave it unset. Writes the
 * canonical PDF to `out_buffer` and an audit JSON report to `out_json`. */
WELLFRIENDPDF_API int wellfriendpdf_document_canonicalize_json(
    const WellfriendDocument *document,
    int64_t date_epoch,
    int has_date_epoch,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* Redact every occurrence of the NUL-terminated UTF-8 strings in `terms`
 * (case-insensitive), full-rewrite, and verify absence. `strict` non-zero fails
 * the call if any term survives. Writes the redacted PDF to `out_buffer` and a
 * JSON report (with verification) to `out_json`. */
WELLFRIENDPDF_API int wellfriendpdf_document_redact_terms_json(
    const WellfriendDocument *document,
    const char *const *terms,
    size_t terms_len,
    int strict,
    WellfriendBuffer *out_buffer,
    char **out_json,
    char **error_out);

/* --- Version / capability query (no document needed) ---------------------- */

/* SDK feature / capability report JSON: engine version, envelope version, and
 * compiled capabilities. Free `*out_json` with wellfriendpdf_string_free. */
WELLFRIENDPDF_API int wellfriendpdf_feature_report_json(
    char **out_json,
    char **error_out);

/* Codec isolation probe/decode report. `filter` is a PDF stream filter name,
 * `data` points to the encoded stream bytes, and `policy` is
 * "in_process"|"isolated_preferred"|"isolated_required"|"report_only"|
 * "disabled" (NULL => "in_process"). Free `*out_json` with
 * wellfriendpdf_string_free. */
WELLFRIENDPDF_API int wellfriendpdf_codec_isolation_report_json(
    const char *filter,
    const uint8_t *data,
    size_t len,
    const char *policy,
    char **out_json,
    char **error_out);

/* The wellfriendpdf-engine semantic version as a NUL-terminated string owned by the
 * caller (free with wellfriendpdf_string_free). NULL only on allocation failure. */
WELLFRIENDPDF_API char *wellfriendpdf_version(void);

/* The C-ABI report envelope version. Bumps signal an envelope-shape change. */
WELLFRIENDPDF_API uint32_t wellfriendpdf_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif
