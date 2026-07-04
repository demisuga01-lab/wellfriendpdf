/* Cross-language SDK facade demo (C side).
 *
 * Opens a PDF and calls the Prompt-01 report surfaces, printing the envelope
 * `kind` for each. This is the C counterpart of the Rust `sdk_reports` example
 * and the Python `sdk_reports.py` — all three call the SAME shared facade and
 * receive the SAME versioned-JSON envelopes.
 *
 * Build (MSVC, after `cargo build -p oxide-capi`):
 *   cl /I include examples\sdk_reports.c target\debug\oxide_capi.dll.lib
 *
 * Build (gcc/clang):
 *   cc -I include examples/sdk_reports.c -Ltarget/debug -loxide_capi -o sdk_reports
 *
 * Run:
 *   sdk_reports input.pdf
 */

#include "oxide.h"

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
                int (*fn)(const OxideDocument *, char **, char **),
                const OxideDocument *doc) {
  char *json = NULL;
  char *error = NULL;
  int status = fn(doc, &json, &error);
  if (status != OXIDE_STATUS_OK) {
    fprintf(stderr, "  %-14s ERROR: %s\n", label, error ? error : "(none)");
    oxide_error_free(error);
    return 1;
  }
  size_t n = strlen(json);
  if (n > 72) n = 72;
  printf("  %-14s %.*s...\n", label, (int)n, json);
  oxide_string_free(json);
  return 0;
}

/* Append a "<name>": <json-envelope> pair to an open smoke-dump file. */
static void dump_pair(FILE *out, int *first, const char *name,
                      int (*fn)(const OxideDocument *, char **, char **),
                      const OxideDocument *doc) {
  if (!out) return;
  char *json = NULL;
  char *error = NULL;
  if (fn(doc, &json, &error) != OXIDE_STATUS_OK) {
    oxide_error_free(error);
    return;
  }
  fprintf(out, "%s\n  \"%s\": %s", *first ? "" : ",", name, json);
  *first = 0;
  oxide_string_free(json);
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s input.pdf [out.json]\n", argv[0]);
    return 2;
  }
  const char *out_path = (argc > 2) ? argv[2] : NULL;

  /* Version / capability query needs no document. */
  char *version = oxide_version();
  printf("oxide-engine version: %s (abi %u)\n", version ? version : "?",
         oxide_abi_version());
  oxide_string_free(version);

  char *feature = NULL;
  char *ferr = NULL;
  if (oxide_feature_report_json(&feature, &ferr) == OXIDE_STATUS_OK) {
    printf("feature report: %.72s...\n", feature);
    oxide_string_free(feature);
  } else {
    oxide_error_free(ferr);
  }

  size_t len = 0;
  unsigned char *data = read_file(argv[1], &len);
  if (!data) {
    fprintf(stderr, "cannot read %s\n", argv[1]);
    return 1;
  }

  char *error = NULL;
  OxideDocument *doc = oxide_document_open_from_bytes(data, len, &error);
  free(data);
  if (!doc) {
    fprintf(stderr, "open failed: %s\n", error ? error : "(none)");
    oxide_error_free(error);
    return 1;
  }

  printf("oxide SDK facade — %s\n", argv[1]);
  int rc = 0;
  rc |= show("security", oxide_document_security_report_json, doc);
  rc |= show("forms", oxide_document_forms_report_json, doc);
  rc |= show("annotations", oxide_document_annotations_report_json, doc);
  rc |= show("pages", oxide_document_pages_report_json, doc);
  rc |= show("interactive", oxide_document_interactive_report_json, doc);
  rc |= show("chunks", oxide_document_chunks_json, doc);

  /* Parametrized reports. */
  {
    char *json = NULL;
    char *err = NULL;
    if (oxide_document_parser_report_json(doc, "audit", &json, &err) ==
        OXIDE_STATUS_OK) {
      printf("  %-14s %.72s...\n", "parser", json);
      oxide_string_free(json);
    } else {
      oxide_error_free(err);
      rc = 1;
    }
  }
  {
    char *json = NULL;
    char *err = NULL;
    if (oxide_document_validate_json(doc, "all", &json, &err) ==
        OXIDE_STATUS_OK) {
      printf("  %-14s %.72s...\n", "validate", json);
      oxide_string_free(json);
    } else {
      oxide_error_free(err);
      rc = 1;
    }
  }

  /* Output-producing: sanitize → owned buffer + report. */
  {
    OxideBuffer buf = {0};
    char *json = NULL;
    char *err = NULL;
    if (oxide_document_sanitize_json(doc, "balanced", &buf, &json, &err) ==
        OXIDE_STATUS_OK) {
      printf("  %-14s %zu bytes, report %.48s...\n", "sanitize", buf.len, json);
      oxide_buffer_free(buf);
      oxide_string_free(json);
    } else {
      oxide_error_free(err);
      rc = 1;
    }
  }

  /* Optional: write a single aggregate smoke JSON of the report envelopes. */
  if (out_path) {
    FILE *out = fopen(out_path, "wb");
    if (out) {
      int first = 1;
      fprintf(out, "{\n  \"envelope_version\": %u,", oxide_abi_version());
      fprintf(out, "\n  \"source\": \"%s\"", argv[1]);
      first = 0;
      dump_pair(out, &first, "security", oxide_document_security_report_json, doc);
      dump_pair(out, &first, "forms", oxide_document_forms_report_json, doc);
      dump_pair(out, &first, "annotations", oxide_document_annotations_report_json, doc);
      dump_pair(out, &first, "pages", oxide_document_pages_report_json, doc);
      dump_pair(out, &first, "interactive", oxide_document_interactive_report_json, doc);
      dump_pair(out, &first, "chunks", oxide_document_chunks_json, doc);
      char *feat = NULL, *ferr2 = NULL;
      if (oxide_feature_report_json(&feat, &ferr2) == OXIDE_STATUS_OK) {
        fprintf(out, ",\n  \"feature\": %s", feat);
        oxide_string_free(feat);
      } else {
        oxide_error_free(ferr2);
      }
      fprintf(out, "\n}\n");
      fclose(out);
      printf("wrote %s\n", out_path);
    }
  }

  oxide_document_free(doc);
  return rc;
}
