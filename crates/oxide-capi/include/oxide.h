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

#ifdef __cplusplus
}
#endif

#endif
