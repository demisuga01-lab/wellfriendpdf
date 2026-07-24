/*
 * Demonstrates the Wellfriend C ABI parser surface:
 *   - parse a PDF into the canonical document model and print it as Markdown,
 *   - extract structured key-value fields (invoice/receipt/form) as JSON.
 *
 * Build (after building the cdylib/staticlib — see docs/bindings.md), e.g. MSVC:
 *   cl /I crates\wellfriendpdf-capi\include parse_document.c wellfriendpdf_capi.lib
 * or gcc/clang:
 *   cc -I crates/wellfriendpdf-capi/include parse_document.c -L target/release -lwellfriendpdf_capi
 *
 * Usage: parse_document input.pdf [doc_type]
 *   doc_type is optional: auto (default) | invoice | receipt | form | generic
 */
#include "wellfriendpdf.h"

#include <stdio.h>
#include <stdlib.h>

static unsigned char *read_file(const char *path, size_t *len) {
  FILE *file = fopen(path, "rb");
  if (!file) {
    return NULL;
  }
  fseek(file, 0, SEEK_END);
  long size = ftell(file);
  if (size < 0) {
    fclose(file);
    return NULL;
  }
  fseek(file, 0, SEEK_SET);
  unsigned char *data = (unsigned char *)malloc((size_t)size);
  if (!data) {
    fclose(file);
    return NULL;
  }
  if (fread(data, 1, (size_t)size, file) != (size_t)size) {
    free(data);
    fclose(file);
    return NULL;
  }
  fclose(file);
  *len = (size_t)size;
  return data;
}

int main(int argc, char **argv) {
  if (argc < 2 || argc > 3) {
    fprintf(stderr, "usage: %s input.pdf [doc_type]\n", argv[0]);
    return 2;
  }
  const char *doc_type = (argc == 3) ? argv[2] : NULL; /* NULL = auto-detect */

  size_t len = 0;
  unsigned char *bytes = read_file(argv[1], &len);
  if (!bytes) {
    fprintf(stderr, "could not read %s\n", argv[1]);
    return 2;
  }

  char *error = NULL;
  WellfriendDocument *doc = wellfriendpdf_document_open_from_bytes(bytes, len, &error);
  free(bytes);
  if (!doc) {
    fprintf(stderr, "open failed: %s\n", error ? error : "unknown error");
    wellfriendpdf_error_free(error);
    return 1;
  }

  /* Parse the whole document into the canonical model -> Markdown. */
  char *markdown = NULL;
  int status = wellfriendpdf_document_parse_markdown(doc, &markdown, &error);
  if (status != WELLFRIENDPDF_STATUS_OK) {
    fprintf(stderr, "parse failed: %s\n", error ? error : "unknown error");
    wellfriendpdf_error_free(error);
    wellfriendpdf_document_free(doc);
    return 1;
  }
  printf("=== Markdown ===\n%s\n", markdown);
  wellfriendpdf_string_free(markdown);

  /* Extract structured key-value fields -> JSON. */
  char *fields = NULL;
  status = wellfriendpdf_document_extract_fields_json(doc, doc_type, &fields, &error);
  if (status != WELLFRIENDPDF_STATUS_OK) {
    fprintf(stderr, "extract-fields failed: %s\n", error ? error : "unknown error");
    wellfriendpdf_error_free(error);
    wellfriendpdf_document_free(doc);
    return 1;
  }
  printf("=== Fields (JSON) ===\n%s\n", fields);
  wellfriendpdf_string_free(fields);

  wellfriendpdf_document_free(doc);
  return 0;
}
