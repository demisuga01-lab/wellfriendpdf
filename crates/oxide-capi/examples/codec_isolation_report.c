/* Prompt 03 codec-isolation report demo.
 *
 * Build (MSVC, after `cargo build -p oxide-capi`):
 *   cl /I crates\oxide-capi\include crates\oxide-capi\examples\codec_isolation_report.c target\debug\oxide_capi.dll.lib
 *
 * Run:
 *   codec_isolation_report.exe
 */

#include "oxide.h"

#include <stdio.h>
#include <string.h>

int main(void) {
  const unsigned char flate_hello_oxide[] = {
      0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0xaf,
      0xc8, 0x4c, 0x49, 0x05, 0x00, 0x19, 0xdd, 0x04, 0x4e};

  char *json = NULL;
  char *error = NULL;
  int status = oxide_codec_isolation_report_json(
      "FlateDecode", flate_hello_oxide, sizeof(flate_hello_oxide),
      "in_process", &json, &error);
  if (status != OXIDE_STATUS_OK) {
    fprintf(stderr, "codec isolation report failed: %s\n",
            error ? error : "(no detail)");
    oxide_error_free(error);
    return 1;
  }

  printf("%s\n", json);
  int ok = strstr(json, "\"status\":\"success\"") != NULL ||
           strstr(json, "\"status\": \"success\"") != NULL;
  oxide_string_free(json);
  return ok ? 0 : 1;
}
