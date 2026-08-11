// Direct PDFium C raster harness.
//
// This program is intentionally a correctness/smoke harness, not a benchmark.
// It emits one JSONL record per rendered page and never records timing data.

#include <errno.h>
#include <inttypes.h>
#include <limits.h>
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fpdfview.h"

#define DEFAULT_DPI 72
#define FNV_OFFSET UINT64_C(14695981039346656037)
#define FNV_PRIME UINT64_C(1099511628211)

typedef struct {
    float left;
    float bottom;
    float right;
    float top;
} PageRect;

typedef enum {
    PAGE_BOX_MEDIA,
    PAGE_BOX_CROP
} PageBoxMode;

typedef enum {
    PIXEL_FORMAT_BGRA,
    PIXEL_FORMAT_BGRX
} PixelFormatMode;

typedef struct {
    const char* input_path;
    const char* output_path;
    const char* jsonl_path;
    const char* manifest_path;
    int page_number;
    int dpi;
    int output_width;
    int output_height;
    int workers;
    bool all_pages;
    bool annotations;
    bool forms;
    uint32_t background_argb;
    PageBoxMode page_box;
    PixelFormatMode pixel_format;
    bool has_matrix;
    FS_MATRIX matrix;
    bool has_clip;
    FS_RECTF clip;
} Options;

static void usage(const char* program) {
    fprintf(stderr,
            "Usage: %s --input FILE [--page N|--all-pages] [--dpi N] \\\n"
            "          [--output FILE_OR_PREFIX] [--jsonl FILE] [--manifest FILE] \\\n"
            "          [--page-box media|crop] [--width PX --height PX] \\\n"
            "          [--pixel-format bgra|bgrx] [--workers N] \\\n"
            "          [--annotations 0|1] [--forms 0|1] [--background AARRGGBB] \\\n"
            "          [--matrix a b c d e f] [--clip x y width height]\n",
            program);
}

static bool parse_int(const char* text, int* out) {
    char* end = NULL;
    errno = 0;
    long value = strtol(text, &end, 10);
    if (errno != 0 || end == text || *end != '\0' || value < INT_MIN || value > INT_MAX) {
        return false;
    }
    *out = (int)value;
    return true;
}

static bool parse_float(const char* text, float* out) {
    char* end = NULL;
    errno = 0;
    float value = strtof(text, &end);
    if (errno != 0 || end == text || *end != '\0' || !isfinite(value)) {
        return false;
    }
    *out = value;
    return true;
}

static bool parse_argb(const char* text, uint32_t* out) {
    char* end = NULL;
    errno = 0;
    unsigned long value = strtoul(text, &end, 16);
    if (errno != 0 || end == text || *end != '\0' || value > UINT32_MAX) {
        return false;
    }
    *out = (uint32_t)value;
    return true;
}

static bool parse_bool(const char* text, bool* out) {
    if (strcmp(text, "1") == 0 || strcmp(text, "true") == 0) {
        *out = true;
        return true;
    }
    if (strcmp(text, "0") == 0 || strcmp(text, "false") == 0) {
        *out = false;
        return true;
    }
    return false;
}

static bool parse_page_box(const char* text, PageBoxMode* out) {
    if (strcmp(text, "media") == 0) {
        *out = PAGE_BOX_MEDIA;
        return true;
    }
    if (strcmp(text, "crop") == 0) {
        *out = PAGE_BOX_CROP;
        return true;
    }
    return false;
}

static const char* page_box_name(PageBoxMode mode) {
    return mode == PAGE_BOX_CROP ? "crop" : "media";
}

static bool parse_pixel_format(const char* text, PixelFormatMode* out) {
    if (strcmp(text, "bgra") == 0) {
        *out = PIXEL_FORMAT_BGRA;
        return true;
    }
    if (strcmp(text, "bgrx") == 0) {
        *out = PIXEL_FORMAT_BGRX;
        return true;
    }
    return false;
}

static const char* pixel_format_name(PixelFormatMode mode) {
    return mode == PIXEL_FORMAT_BGRX ? "bgrx" : "bgra";
}

static uint64_t fnv1a64(const unsigned char* data, size_t len) {
    uint64_t hash = FNV_OFFSET;
    for (size_t index = 0; index < len; ++index) {
        hash ^= data[index];
        hash *= FNV_PRIME;
    }
    return hash;
}

static void json_escape(FILE* output, const char* value) {
    for (const unsigned char* cursor = (const unsigned char*)value; *cursor; ++cursor) {
        switch (*cursor) {
            case '\\': fputs("\\\\", output); break;
            case '"': fputs("\\\"", output); break;
            case '\n': fputs("\\n", output); break;
            case '\r': fputs("\\r", output); break;
            case '\t': fputs("\\t", output); break;
            default:
                if (*cursor < 0x20) {
                    fprintf(output, "\\u%04x", *cursor);
                } else {
                    fputc(*cursor, output);
                }
        }
    }
}

static void emit_error(FILE* jsonl, int page, const char* code, const char* detail) {
    if (jsonl == NULL) {
        return;
    }
    fprintf(jsonl, "{\"engine\":\"pdfium-c\",\"page\":%d,\"status\":\"error\",\"code\":\"", page);
    json_escape(jsonl, code);
    fputs("\",\"detail\":\"", jsonl);
    json_escape(jsonl, detail);
    fputs("\"}\n", jsonl);
    fflush(jsonl);
}

static void emit_success(
    FILE* jsonl,
    const Options* options,
    PageRect page_box,
    int page,
    int width,
    int height,
    int stride,
    uint64_t hash,
    const char* output_path
) {
    if (jsonl == NULL) {
        return;
    }
    fprintf(jsonl,
            "{\"engine\":\"pdfium-c\",\"page\":%d,\"status\":\"ok\","
            "\"width\":%d,\"height\":%d,\"stride\":%d,\"dpi\":%d,"
            "\"page_box\":\"%s\",\"page_box_rect\":[%.6g,%.6g,%.6g,%.6g],"
            "\"pixel_format\":\"%s\",\"hash_fnv1a64\":\"%016" PRIx64 "\","
            "\"annotations\":%s,\"forms_requested\":%s,\"workers\":%d,\"output\":\"",
            page,
            width,
            height,
            stride,
            options->dpi,
            page_box_name(options->page_box),
            page_box.left,
            page_box.bottom,
            page_box.right,
            page_box.top,
            pixel_format_name(options->pixel_format),
            hash,
            options->annotations ? "true" : "false",
            options->forms ? "true" : "false",
            options->workers);
    json_escape(jsonl, output_path == NULL ? "" : output_path);
    fputs("\"}\n", jsonl);
    fflush(jsonl);
}

static bool page_box_rect(FPDF_PAGE page, PageBoxMode mode, PageRect* out) {
    if (mode == PAGE_BOX_CROP) {
        FS_RECTF bbox;
        if (!FPDF_GetPageBoundingBox(page, &bbox)) {
            return false;
        }
        float bottom = bbox.bottom < bbox.top ? bbox.bottom : bbox.top;
        float top = bbox.bottom < bbox.top ? bbox.top : bbox.bottom;
        *out = (PageRect){bbox.left, bottom, bbox.right, top};
    } else {
        double width = FPDF_GetPageWidth(page);
        double height = FPDF_GetPageHeight(page);
        if (!isfinite(width) || !isfinite(height)) {
            return false;
        }
        *out = (PageRect){0.0f, 0.0f, (float)width, (float)height};
    }
    return isfinite(out->left) && isfinite(out->bottom) && isfinite(out->right) && isfinite(out->top) &&
           out->right > out->left && out->top > out->bottom;
}

static bool emit_manifest(const Options* options, FPDF_DOCUMENT document, int total_pages) {
    if (options->manifest_path == NULL) {
        return true;
    }
    FILE* manifest = fopen(options->manifest_path, "wb");
    if (manifest == NULL) {
        return false;
    }
    int file_version = 0;
    int has_file_version = FPDF_GetFileVersion(document, &file_version);
    fprintf(manifest,
            "{"
            "\"schema_version\":1,"
            "\"engine\":\"pdfium-c\","
            "\"harness\":\"wellfriend-pdfium-harness\","
            "\"harness_version\":\"1\","
            "\"pdfium_runtime_version\":\"not_exposed_by_public_c_api\","
            "\"pdf_file_version\":");
    if (has_file_version) {
        fprintf(manifest, "%d", file_version);
    } else {
        fputs("null", manifest);
    }
    fprintf(manifest,
            ",\"total_pages\":%d,"
            "\"page_selection\":\"%s\","
            "\"requested_page\":%d,"
            "\"dpi\":%d,"
            "\"output_width\":%d,"
            "\"output_height\":%d,"
            "\"page_box\":\"%s\","
            "\"pixel_format\":\"%s\","
            "\"annotations\":%s,"
            "\"forms_requested\":%s,"
            "\"workers\":%d,"
            "\"outputs\":{\"raw_bitmap\":\"%s\",\"jsonl\":\"%s\"}"
            "}\n",
            total_pages,
            options->all_pages ? "all" : "single",
            options->page_number,
            options->dpi,
            options->output_width,
            options->output_height,
            page_box_name(options->page_box),
            pixel_format_name(options->pixel_format),
            options->annotations ? "true" : "false",
            options->forms ? "true" : "false",
            options->workers,
            options->output_path == NULL ? "" : options->output_path,
            options->jsonl_path == NULL ? "stdout" : options->jsonl_path);
    return fclose(manifest) == 0;
}

static bool write_bytes(const char* output_path, const unsigned char* data, size_t len) {
    if (output_path == NULL) {
        return true;
    }
    FILE* output = fopen(output_path, "wb");
    if (output == NULL) {
        return false;
    }
    size_t written = fwrite(data, 1, len, output);
    int close_status = fclose(output);
    return written == len && close_status == 0;
}

static char* duplicate_string(const char* value) {
    size_t length = strlen(value);
    char* copy = (char*)malloc(length + 1);
    if (copy != NULL) {
        memcpy(copy, value, length + 1);
    }
    return copy;
}

static char* page_output_path(const Options* options, int page_number) {
    if (options->output_path == NULL) {
        return NULL;
    }
    if (!options->all_pages) {
        return duplicate_string(options->output_path);
    }
    size_t needed = strlen(options->output_path) + 32;
    char* output = (char*)malloc(needed);
    if (output != NULL) {
        snprintf(output, needed, "%s-page-%04d.bgra", options->output_path, page_number);
    }
    return output;
}

static int render_page(FPDF_DOCUMENT document, const Options* options, int page_number, FILE* jsonl) {
    FPDF_PAGE page = FPDF_LoadPage(document, page_number - 1);
    if (page == NULL) {
        emit_error(jsonl, page_number, "load_page", "FPDF_LoadPage returned null");
        return 1;
    }

    PageRect box;
    if (!page_box_rect(page, options->page_box, &box)) {
        emit_error(jsonl, page_number, "page_box", "could not resolve the requested PDFium page box");
        FPDF_ClosePage(page);
        return 1;
    }
    double width_points = (double)box.right - (double)box.left;
    double height_points = (double)box.top - (double)box.bottom;
    double scale_x = (double)options->dpi / 72.0;
    double scale_y = scale_x;
    double width_double = options->output_width > 0 ? (double)options->output_width : ceil(width_points * scale_x);
    double height_double = options->output_height > 0 ? (double)options->output_height : ceil(height_points * scale_y);
    if (!isfinite(width_double) || !isfinite(height_double) || width_double < 1 || height_double < 1 ||
        width_double > INT_MAX || height_double > INT_MAX) {
        emit_error(jsonl, page_number, "dimensions", "invalid PDFium page dimensions");
        FPDF_ClosePage(page);
        return 1;
    }
    int width = (int)width_double;
    int height = (int)height_double;
    scale_x = (double)width / width_points;
    scale_y = (double)height / height_points;
    FPDF_BITMAP bitmap = FPDFBitmap_Create(width, height, options->pixel_format == PIXEL_FORMAT_BGRA ? 1 : 0);
    if (bitmap == NULL) {
        emit_error(jsonl, page_number, "bitmap_create", "FPDFBitmap_Create returned null");
        FPDF_ClosePage(page);
        return 1;
    }

    FPDFBitmap_FillRect(bitmap, 0, 0, width, height, options->background_argb);
    int flags = options->annotations ? FPDF_ANNOT : 0;
    FS_MATRIX matrix = options->has_matrix
        ? options->matrix
        : (FS_MATRIX){(float)scale_x, 0.0f, 0.0f, (float)-scale_y, (float)(-box.left * scale_x),
                      (float)(box.top * scale_y)};
    FS_RECTF clip = options->has_clip
        ? options->clip
        : (FS_RECTF){0, 0, width, height};
    FPDF_RenderPageBitmapWithMatrix(bitmap, page, &matrix, &clip, flags);

    int stride = FPDFBitmap_GetStride(bitmap);
    unsigned char* buffer = (unsigned char*)FPDFBitmap_GetBuffer(bitmap);
    size_t byte_len = stride > 0 && height > 0 ? (size_t)stride * (size_t)height : 0;
    char* output_path = page_output_path(options, page_number);
    int result = 0;
    if (buffer == NULL || byte_len == 0) {
        emit_error(jsonl, page_number, "bitmap_buffer", "PDFium returned an empty bitmap buffer");
        result = 1;
    } else if (!write_bytes(output_path, buffer, byte_len)) {
        emit_error(jsonl, page_number, "write_output", "could not write raw BGRA output");
        result = 1;
    } else {
        emit_success(jsonl, options, box, page_number, width, height, stride, fnv1a64(buffer, byte_len), output_path);
    }

    free(output_path);
    FPDFBitmap_Destroy(bitmap);
    FPDF_ClosePage(page);
    return result;
}

int main(int argc, char** argv) {
    Options options = {
        .input_path = NULL,
        .output_path = NULL,
        .jsonl_path = NULL,
        .manifest_path = NULL,
        .page_number = 1,
        .dpi = DEFAULT_DPI,
        .output_width = 0,
        .output_height = 0,
        .workers = 1,
        .all_pages = false,
        .annotations = true,
        .forms = true,
        .background_argb = UINT32_C(0xffffffff),
        .page_box = PAGE_BOX_MEDIA,
        .pixel_format = PIXEL_FORMAT_BGRA,
        .has_matrix = false,
        .has_clip = false,
    };

    for (int index = 1; index < argc; ++index) {
        const char* arg = argv[index];
        if (strcmp(arg, "--input") == 0 && index + 1 < argc) {
            options.input_path = argv[++index];
        } else if (strcmp(arg, "--output") == 0 && index + 1 < argc) {
            options.output_path = argv[++index];
        } else if (strcmp(arg, "--jsonl") == 0 && index + 1 < argc) {
            options.jsonl_path = argv[++index];
        } else if (strcmp(arg, "--manifest") == 0 && index + 1 < argc) {
            options.manifest_path = argv[++index];
        } else if (strcmp(arg, "--page") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.page_number) || options.page_number < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--all-pages") == 0) {
            options.all_pages = true;
        } else if (strcmp(arg, "--dpi") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.dpi) || options.dpi < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--width") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.output_width) || options.output_width < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--height") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.output_height) || options.output_height < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--workers") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.workers) || options.workers < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--worker-count") == 0 && index + 1 < argc) {
            if (!parse_int(argv[++index], &options.workers) || options.workers < 1) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--page-box") == 0 && index + 1 < argc) {
            if (!parse_page_box(argv[++index], &options.page_box)) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--pixel-format") == 0 && index + 1 < argc) {
            if (!parse_pixel_format(argv[++index], &options.pixel_format)) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--annotations") == 0 && index + 1 < argc) {
            if (!parse_bool(argv[++index], &options.annotations)) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--forms") == 0 && index + 1 < argc) {
            if (!parse_bool(argv[++index], &options.forms)) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--background") == 0 && index + 1 < argc) {
            if (!parse_argb(argv[++index], &options.background_argb)) {
                usage(argv[0]);
                return 2;
            }
        } else if (strcmp(arg, "--matrix") == 0 && index + 6 < argc) {
            float values[6];
            for (int matrix_index = 0; matrix_index < 6; ++matrix_index) {
                if (!parse_float(argv[++index], &values[matrix_index])) {
                    usage(argv[0]);
                    return 2;
                }
            }
            options.matrix = (FS_MATRIX){values[0], values[1], values[2], values[3], values[4], values[5]};
            options.has_matrix = true;
        } else if (strcmp(arg, "--clip") == 0 && index + 4 < argc) {
            int values[4];
            for (int clip_index = 0; clip_index < 4; ++clip_index) {
                if (!parse_int(argv[++index], &values[clip_index])) {
                    usage(argv[0]);
                    return 2;
                }
            }
            if (values[2] <= 0 || values[3] <= 0) {
                usage(argv[0]);
                return 2;
            }
            options.clip = (FS_RECTF){values[0], values[1], values[0] + values[2], values[1] + values[3]};
            options.has_clip = true;
        } else {
            usage(argv[0]);
            return 2;
        }
    }

    if (options.input_path == NULL) {
        usage(argv[0]);
        return 2;
    }
    if ((options.output_width == 0) != (options.output_height == 0)) {
        fprintf(stderr, "--width and --height must be supplied together\n");
        return 2;
    }
    if (options.all_pages && options.output_path != NULL && strstr(options.output_path, "%") != NULL) {
        fprintf(stderr, "--all-pages output uses an automatic -page-NNNN.bgra suffix; do not use a format string\n");
        return 2;
    }

    FILE* jsonl = options.jsonl_path == NULL ? stdout : fopen(options.jsonl_path, "wb");
    if (jsonl == NULL) {
        fprintf(stderr, "cannot open JSONL output: %s\n", strerror(errno));
        return 2;
    }

    FPDF_InitLibrary();
    FPDF_DOCUMENT document = FPDF_LoadDocument(options.input_path, NULL);
    if (document == NULL) {
        char detail[80];
        snprintf(detail, sizeof(detail), "FPDF_LoadDocument failed with code %lu", (unsigned long)FPDF_GetLastError());
        emit_error(jsonl, options.page_number, "load_document", detail);
        if (jsonl != stdout) fclose(jsonl);
        FPDF_DestroyLibrary();
        return 1;
    }

    int total_pages = FPDF_GetPageCount(document);
    if (!emit_manifest(&options, document, total_pages)) {
        emit_error(jsonl, options.page_number, "manifest", "could not write version manifest");
        FPDF_CloseDocument(document);
        FPDF_DestroyLibrary();
        if (jsonl != stdout) fclose(jsonl);
        return 1;
    }
    int status = 0;
    if (options.all_pages) {
        for (int page = 1; page <= total_pages; ++page) {
            status |= render_page(document, &options, page, jsonl);
        }
    } else if (options.page_number > total_pages) {
        emit_error(jsonl, options.page_number, "page_range", "requested page is outside the document page count");
        status = 1;
    } else {
        status = render_page(document, &options, options.page_number, jsonl);
    }

    FPDF_CloseDocument(document);
    FPDF_DestroyLibrary();
    if (jsonl != stdout) fclose(jsonl);
    return status == 0 ? 0 : 1;
}
