/* Cross-language SDK facade demo (C side).
 *
 * Opens a PDF and calls the binding-surface report surfaces, printing the envelope
 * `kind` for each. This is the C counterpart of the Rust `sdk_reports` example
 * and the Python `sdk_reports.py` — all three call the SAME shared facade and
 * receive the SAME versioned-JSON envelopes.
 *
 * Build (MSVC, after `cargo build -p wellfriendpdf-capi`):
 *   cl /I include examples\sdk_reports.c target\debug\wellfriendpdf_capi.dll.lib
 *
 * Build (gcc/clang):
 *   cc -I include examples/sdk_reports.c -Ltarget/debug -lwellfriendpdf_capi -o sdk_reports
 *
 * Run:
 *   sdk_reports input.pdf
 */

#include "wellfriendpdf.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

/* Fetch a report, print its first ~72 chars (the envelope prefix), then free. */
static int show(const char *label,
                int (*fn)(const WellfriendDocument *, char **, char **),
                const WellfriendDocument *doc) {
  char *json = NULL;
  char *error = NULL;
  int status = fn(doc, &json, &error);
  if (status != WELLFRIENDPDF_STATUS_OK) {
    fprintf(stderr, "  %-14s ERROR: %s\n", label, error ? error : "(none)");
    wellfriendpdf_error_free(error);
    return 1;
  }
  size_t n = strlen(json);
  if (n > 72) n = 72;
  printf("  %-14s %.*s...\n", label, (int)n, json);
  wellfriendpdf_string_free(json);
  return 0;
}

/* Append a "<name>": <json-envelope> pair to an open smoke-dump file. */
static void dump_pair(FILE *out, int *first, const char *name,
                      int (*fn)(const WellfriendDocument *, char **, char **),
                      const WellfriendDocument *doc) {
  if (!out) return;
  char *json = NULL;
  char *error = NULL;
  if (fn(doc, &json, &error) != WELLFRIENDPDF_STATUS_OK) {
    wellfriendpdf_error_free(error);
    return;
  }
  fprintf(out, "%s\n  \"%s\": %s", *first ? "" : ",", name, json);
  *first = 0;
  wellfriendpdf_string_free(json);
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s input.pdf [out.json]\n", argv[0]);
    return 2;
  }
  const char *out_path = (argc > 2) ? argv[2] : NULL;

  /* Version / capability query needs no document. */
  char *version = wellfriendpdf_version();
  printf("wellfriendpdf-engine version: %s (abi %u)\n", version ? version : "?",
         wellfriendpdf_abi_version());
  wellfriendpdf_string_free(version);

  char *feature = NULL;
  char *ferr = NULL;
  if (wellfriendpdf_feature_report_json(&feature, &ferr) == WELLFRIENDPDF_STATUS_OK) {
    printf("feature report: %.72s...\n", feature);
    wellfriendpdf_string_free(feature);
  } else {
    wellfriendpdf_error_free(ferr);
  }

  size_t len = 0;
  unsigned char *data = read_file(argv[1], &len);
  if (!data) {
    fprintf(stderr, "cannot read %s\n", argv[1]);
    return 1;
  }

  char *error = NULL;
  WellfriendDocument *doc = wellfriendpdf_document_open_from_bytes(data, len, &error);
  free(data);
  if (!doc) {
    fprintf(stderr, "open failed: %s\n", error ? error : "(none)");
    wellfriendpdf_error_free(error);
    return 1;
  }

  printf("wellfriendpdf SDK facade — %s\n", argv[1]);
  int rc = 0;
  rc |= show("security", wellfriendpdf_document_security_report_json, doc);
  rc |= show("forms", wellfriendpdf_document_forms_report_json, doc);
  rc |= show("annotations", wellfriendpdf_document_annotations_report_json, doc);
  rc |= show("pages", wellfriendpdf_document_pages_report_json, doc);
  rc |= show("interactive", wellfriendpdf_document_interactive_report_json, doc);
  rc |= show("chunks", wellfriendpdf_document_chunks_json, doc);

  /* Parametrized reports. */
  {
    char *json = NULL;
    char *err = NULL;
    if (wellfriendpdf_document_parser_report_json(doc, "audit", &json, &err) ==
        WELLFRIENDPDF_STATUS_OK) {
      printf("  %-14s %.72s...\n", "parser", json);
      wellfriendpdf_string_free(json);
    } else {
      wellfriendpdf_error_free(err);
      rc = 1;
    }
  }
  {
    char *json = NULL;
    char *err = NULL;
    if (wellfriendpdf_document_validate_json(doc, "all", &json, &err) ==
        WELLFRIENDPDF_STATUS_OK) {
      printf("  %-14s %.72s...\n", "validate", json);
      wellfriendpdf_string_free(json);
    } else {
      wellfriendpdf_error_free(err);
      rc = 1;
    }
  }

  /* Output-producing: sanitize → owned buffer + report. */
  {
    WellfriendBuffer buf = {0};
    char *json = NULL;
    char *err = NULL;
    if (wellfriendpdf_document_sanitize_json(doc, "balanced", &buf, &json, &err) ==
        WELLFRIENDPDF_STATUS_OK) {
      printf("  %-14s %zu bytes, report %.48s...\n", "sanitize", buf.len, json);
      wellfriendpdf_buffer_free(buf);
      wellfriendpdf_string_free(json);
    } else {
      wellfriendpdf_error_free(err);
      rc = 1;
    }
  }

  /* Optional: write a single aggregate smoke JSON of the report envelopes. */
  if (out_path) {
    FILE *out = fopen(out_path, "wb");
    if (out) {
      int first = 1;
      fprintf(out, "{\n  \"envelope_version\": %u,", wellfriendpdf_abi_version());
      fprintf(out, "\n  \"source\": \"%s\"", argv[1]);
      first = 0;
      dump_pair(out, &first, "security", wellfriendpdf_document_security_report_json, doc);
      dump_pair(out, &first, "forms", wellfriendpdf_document_forms_report_json, doc);
      dump_pair(out, &first, "annotations", wellfriendpdf_document_annotations_report_json, doc);
      dump_pair(out, &first, "pages", wellfriendpdf_document_pages_report_json, doc);
      dump_pair(out, &first, "interactive", wellfriendpdf_document_interactive_report_json, doc);
      dump_pair(out, &first, "chunks", wellfriendpdf_document_chunks_json, doc);
      char *feat = NULL, *ferr2 = NULL;
      if (wellfriendpdf_feature_report_json(&feat, &ferr2) == WELLFRIENDPDF_STATUS_OK) {
        fprintf(out, ",\n  \"feature\": %s", feat);
        wellfriendpdf_string_free(feat);
      } else {
        wellfriendpdf_error_free(ferr2);
      }
      fprintf(out, "\n}\n");
      fclose(out);
      printf("wrote %s\n", out_path);
    }
  }

  wellfriendpdf_document_free(doc);
  return rc;
}
