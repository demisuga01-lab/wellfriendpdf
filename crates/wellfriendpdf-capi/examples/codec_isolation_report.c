/* Prompt 03 codec-isolation report demo.
 *
 * Build (MSVC, after `cargo build -p wellfriendpdf-capi`):
 *   cl /I crates\wellfriendpdf-capi\include crates\wellfriendpdf-capi\examples\codec_isolation_report.c target\debug\wellfriendpdf_capi.dll.lib
 *
 * Run:
 *   codec_isolation_report.exe
 */

#include "wellfriendpdf.h"

#include <stdio.h>
#include <string.h>

int main(void) {
  const unsigned char flate_hello_wellfriendpdf[] = {
      0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0xaf,
      0xc8, 0x4c, 0x49, 0x05, 0x00, 0x19, 0xdd, 0x04, 0x4e};

  char *json = NULL;
  char *error = NULL;
  int status = wellfriendpdf_codec_isolation_report_json(
      "FlateDecode", flate_hello_wellfriendpdf, sizeof(flate_hello_wellfriendpdf),
      "in_process", &json, &error);
  if (status != WELLFRIENDPDF_STATUS_OK) {
    fprintf(stderr, "codec isolation report failed: %s\n",
            error ? error : "(no detail)");
    wellfriendpdf_error_free(error);
    return 1;
  }

  printf("%s\n", json);
  int ok = strstr(json, "\"status\":\"success\"") != NULL ||
           strstr(json, "\"status\": \"success\"") != NULL;
  wellfriendpdf_string_free(json);
  return ok ? 0 : 1;
}
