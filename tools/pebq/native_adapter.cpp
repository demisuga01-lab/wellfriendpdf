// PEBQ native adapter for qpdf, Poppler, MuPDF, and PDFium.
//
// Build exactly one engine into each executable by defining one of:
//   PEBQ_QPDF, PEBQ_POPPLER, PEBQ_MUPDF, PEBQ_PDFIUM
//
// Protocol:
//   adapter --request <page-count|render> <pdf> [dpi] [output.ppm]
//   adapter --server
//
// Server input is tab separated: profile, path, dpi, output. Each request
// produces one compact JSON line. File reading, document opening/page count,
// rasterization, RGB normalization, and output writing have separate timers.

#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

#if defined(PEBQ_QPDF)
#include <qpdf/QPDF.hh>
#elif defined(PEBQ_POPPLER)
#include <poppler-document.h>
#include <poppler-image.h>
#include <poppler-page-renderer.h>
#include <poppler-page.h>
#elif defined(PEBQ_MUPDF)
#include <mupdf/fitz.h>
#elif defined(PEBQ_PDFIUM)
extern "C" {
typedef void *FPDF_DOCUMENT;
typedef void *FPDF_PAGE;
typedef void *FPDF_BITMAP;
void FPDF_InitLibrary(void);
void FPDF_DestroyLibrary(void);
FPDF_DOCUMENT FPDF_LoadMemDocument64(const void *, size_t, const char *);
void FPDF_CloseDocument(FPDF_DOCUMENT);
int FPDF_GetPageCount(FPDF_DOCUMENT);
FPDF_PAGE FPDF_LoadPage(FPDF_DOCUMENT, int);
void FPDF_ClosePage(FPDF_PAGE);
float FPDF_GetPageWidthF(FPDF_PAGE);
float FPDF_GetPageHeightF(FPDF_PAGE);
FPDF_BITMAP FPDFBitmap_CreateEx(int, int, int, void *, int);
void FPDFBitmap_Destroy(FPDF_BITMAP);
void FPDFBitmap_FillRect(FPDF_BITMAP, int, int, int, int, unsigned long);
void FPDF_RenderPageBitmap(FPDF_BITMAP, FPDF_PAGE, int, int, int, int, int, int);
unsigned long FPDF_GetLastError(void);
}
#endif

namespace {

using Clock = std::chrono::steady_clock;

#if defined(PEBQ_QPDF)
constexpr const char *kEngine = "qpdf";
#elif defined(PEBQ_POPPLER)
constexpr const char *kEngine = "poppler";
#elif defined(PEBQ_MUPDF)
constexpr const char *kEngine = "mupdf";
#elif defined(PEBQ_PDFIUM)
constexpr const char *kEngine = "pdfium";
#else
#error "Define exactly one PEBQ engine macro"
#endif

double elapsed_ms(Clock::time_point start) {
    return std::chrono::duration<double, std::milli>(Clock::now() - start).count();
}

std::string json_escape(const std::string &value) {
    std::ostringstream out;
    for (unsigned char c : value) {
        switch (c) {
        case '\\': out << "\\\\"; break;
        case '"': out << "\\\""; break;
        case '\n': out << "\\n"; break;
        case '\r': out << "\\r"; break;
        case '\t': out << "\\t"; break;
        default:
            if (c < 0x20) {
                const char hex[] = "0123456789abcdef";
                out << "\\u00" << hex[(c >> 4) & 0xf] << hex[c & 0xf];
            } else {
                out << static_cast<char>(c);
            }
        }
    }
    return out.str();
}

std::vector<unsigned char> read_file(const std::string &path) {
    std::ifstream in(path, std::ios::binary | std::ios::ate);
    if (!in) throw std::runtime_error("unable to open input");
    const auto length = in.tellg();
    if (length < 0) throw std::runtime_error("unable to determine input length");
    std::vector<unsigned char> bytes(static_cast<size_t>(length));
    in.seekg(0);
    if (!bytes.empty() && !in.read(reinterpret_cast<char *>(bytes.data()), length)) {
        throw std::runtime_error("unable to read complete input");
    }
    return bytes;
}

uint64_t fnv1a(const unsigned char *bytes, size_t length) {
    uint64_t hash = 1469598103934665603ULL;
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

uint64_t peak_rss_kib() {
    std::ifstream status("/proc/self/status");
    std::string key;
    while (status >> key) {
        if (key == "VmHWM:") {
            uint64_t value = 0;
            std::string unit;
            status >> value >> unit;
            return value;
        }
        std::string rest;
        std::getline(status, rest);
    }
    return 0;
}

void write_ppm(const std::string &path, int width, int height,
               const std::vector<unsigned char> &rgb) {
    std::ofstream out(path, std::ios::binary);
    if (!out) throw std::runtime_error("unable to create output raster");
    out << "P6\n" << width << " " << height << "\n255\n";
    out.write(reinterpret_cast<const char *>(rgb.data()),
              static_cast<std::streamsize>(rgb.size()));
    if (!out) throw std::runtime_error("unable to write output raster");
}

struct EngineDocument {
    int pages = 0;
#if defined(PEBQ_QPDF)
    std::unique_ptr<QPDF> document;
#elif defined(PEBQ_POPPLER)
    std::unique_ptr<poppler::document> document;
#elif defined(PEBQ_MUPDF)
    fz_context *context = nullptr;
    fz_buffer *buffer = nullptr;
    fz_stream *stream = nullptr;
    fz_document *document = nullptr;
#elif defined(PEBQ_PDFIUM)
    FPDF_DOCUMENT document = nullptr;
#endif

    ~EngineDocument() {
#if defined(PEBQ_MUPDF)
        if (context) {
            if (document) fz_drop_document(context, document);
            if (stream) fz_drop_stream(context, stream);
            if (buffer) fz_drop_buffer(context, buffer);
            fz_drop_context(context);
        }
#elif defined(PEBQ_PDFIUM)
        if (document) FPDF_CloseDocument(document);
#endif
    }
};

std::unique_ptr<EngineDocument> open_document(
    const std::string &label, const std::vector<unsigned char> &bytes) {
    auto result = std::make_unique<EngineDocument>();
#if defined(PEBQ_QPDF)
    result->document = std::make_unique<QPDF>();
    result->document->processMemoryFile(
        label.c_str(),
        reinterpret_cast<const char *>(bytes.data()),
        bytes.size());
    result->pages = static_cast<int>(result->document->getAllPages().size());
#elif defined(PEBQ_POPPLER)
    result->document.reset(poppler::document::load_from_raw_data(
        reinterpret_cast<const char *>(bytes.data()), bytes.size()));
    if (!result->document) throw std::runtime_error("Poppler rejected input");
    result->pages = result->document->pages();
#elif defined(PEBQ_MUPDF)
    result->context = fz_new_context(nullptr, nullptr, FZ_STORE_UNLIMITED);
    if (!result->context) throw std::runtime_error("MuPDF context allocation failed");
    fz_register_document_handlers(result->context);
    fz_try(result->context) {
        result->buffer = fz_new_buffer_from_copied_data(
            result->context, bytes.data(), bytes.size());
        result->stream = fz_open_buffer(result->context, result->buffer);
        result->document = fz_open_document_with_stream(
            result->context, "application/pdf", result->stream);
        result->pages = fz_count_pages(result->context, result->document);
    }
    fz_catch(result->context) {
        throw std::runtime_error(fz_caught_message(result->context));
    }
#elif defined(PEBQ_PDFIUM)
    result->document = FPDF_LoadMemDocument64(bytes.data(), bytes.size(), nullptr);
    if (!result->document) {
        throw std::runtime_error("PDFium rejected input with error " +
                                 std::to_string(FPDF_GetLastError()));
    }
    result->pages = FPDF_GetPageCount(result->document);
#endif
    if (result->pages <= 0) throw std::runtime_error("document has no pages");
    return result;
}

struct Raster {
    int width = 0;
    int height = 0;
    std::vector<unsigned char> rgb;
};

Raster render_first_page(EngineDocument &source, int dpi) {
#if defined(PEBQ_QPDF)
    (void)source;
    (void)dpi;
    throw std::runtime_error("qpdf has no raster renderer");
#elif defined(PEBQ_POPPLER)
    std::unique_ptr<poppler::page> page(source.document->create_page(0));
    if (!page) throw std::runtime_error("Poppler could not load page 1");
    poppler::page_renderer renderer;
    renderer.set_image_format(poppler::image::format_rgb24);
    renderer.set_paper_color(0xffffffffU);
    poppler::image image = renderer.render_page(page.get(), dpi, dpi);
    if (!image.is_valid()) throw std::runtime_error("Poppler render failed");
    Raster out;
    out.width = image.width();
    out.height = image.height();
    out.rgb.resize(static_cast<size_t>(out.width) * out.height * 3);
    for (int y = 0; y < out.height; ++y) {
        const auto *row = reinterpret_cast<const unsigned char *>(image.const_data()) +
                          static_cast<size_t>(y) * image.bytes_per_row();
        std::memcpy(out.rgb.data() + static_cast<size_t>(y) * out.width * 3,
                    row, static_cast<size_t>(out.width) * 3);
    }
    return out;
#elif defined(PEBQ_MUPDF)
    Raster out;
    fz_pixmap *pixmap = nullptr;
    fz_try(source.context) {
        const float scale = static_cast<float>(dpi) / 72.0f;
        pixmap = fz_new_pixmap_from_page_number(
            source.context, source.document, 0, fz_scale(scale, scale),
            fz_device_rgb(source.context), 0);
        out.width = fz_pixmap_width(source.context, pixmap);
        out.height = fz_pixmap_height(source.context, pixmap);
        const int stride = fz_pixmap_stride(source.context, pixmap);
        const int components = fz_pixmap_components(source.context, pixmap);
        const unsigned char *samples = fz_pixmap_samples(source.context, pixmap);
        if (components < 3) fz_throw(source.context, FZ_ERROR_FORMAT, "unexpected pixmap format");
        out.rgb.resize(static_cast<size_t>(out.width) * out.height * 3);
        for (int y = 0; y < out.height; ++y) {
            const unsigned char *row = samples + static_cast<size_t>(y) * stride;
            for (int x = 0; x < out.width; ++x) {
                const unsigned char *pixel = row + static_cast<size_t>(x) * components;
                unsigned char *target = out.rgb.data() +
                    (static_cast<size_t>(y) * out.width + x) * 3;
                target[0] = pixel[0]; target[1] = pixel[1]; target[2] = pixel[2];
            }
        }
    }
    fz_always(source.context) {
        if (pixmap) fz_drop_pixmap(source.context, pixmap);
    }
    fz_catch(source.context) {
        throw std::runtime_error(fz_caught_message(source.context));
    }
    return out;
#elif defined(PEBQ_PDFIUM)
    FPDF_PAGE page = FPDF_LoadPage(source.document, 0);
    if (!page) throw std::runtime_error("PDFium could not load page 1");
    const int width = std::max(1, static_cast<int>(std::ceil(FPDF_GetPageWidthF(page) * dpi / 72.0)));
    const int height = std::max(1, static_cast<int>(std::ceil(FPDF_GetPageHeightF(page) * dpi / 72.0)));
    std::vector<unsigned char> bgra(static_cast<size_t>(width) * height * 4);
    FPDF_BITMAP bitmap = FPDFBitmap_CreateEx(width, height, 4, bgra.data(), width * 4);
    if (!bitmap) {
        FPDF_ClosePage(page);
        throw std::runtime_error("PDFium bitmap allocation failed");
    }
    FPDFBitmap_FillRect(bitmap, 0, 0, width, height, 0xffffffffU);
    FPDF_RenderPageBitmap(bitmap, page, 0, 0, width, height, 0, 0x01 | 0x08);
    Raster out;
    out.width = width;
    out.height = height;
    out.rgb.resize(static_cast<size_t>(width) * height * 3);
    for (size_t i = 0, j = 0; i < bgra.size(); i += 4, j += 3) {
        out.rgb[j] = bgra[i + 2];
        out.rgb[j + 1] = bgra[i + 1];
        out.rgb[j + 2] = bgra[i];
    }
    FPDFBitmap_Destroy(bitmap);
    FPDF_ClosePage(page);
    return out;
#endif
}

std::string execute(const std::string &profile, const std::string &path,
                    int dpi, const std::string &output) {
    const auto request_start = Clock::now();
    try {
        const auto read_start = Clock::now();
        const auto bytes = read_file(path);
        const double read_ms = elapsed_ms(read_start);

        const auto parse_start = Clock::now();
        auto document = open_document(path, bytes);
        const double parse_ms = elapsed_ms(parse_start);

        double render_ms = 0.0;
        double write_ms = 0.0;
        int width = 0;
        int height = 0;
        uint64_t raster_hash = 0;
        if (profile == "render") {
            const auto render_start = Clock::now();
            auto raster = render_first_page(*document, dpi);
            render_ms = elapsed_ms(render_start);
            width = raster.width;
            height = raster.height;
            raster_hash = fnv1a(raster.rgb.data(), raster.rgb.size());
            if (!output.empty() && output != "-") {
                const auto write_start = Clock::now();
                write_ppm(output, width, height, raster.rgb);
                write_ms = elapsed_ms(write_start);
            }
        } else if (profile != "page-count") {
            throw std::runtime_error("unknown profile");
        }

        std::ostringstream json;
        json.precision(9);
        json << "{\"engine\":\"" << kEngine << "\",\"status\":\"ok\""
             << ",\"profile\":\"" << profile << "\""
             << ",\"path\":\"" << json_escape(path) << "\""
             << ",\"input_bytes\":" << bytes.size()
             << ",\"page_count\":" << document->pages
             << ",\"read_ms\":" << read_ms
             << ",\"parse_ms\":" << parse_ms
             << ",\"render_ms\":" << render_ms
             << ",\"write_ms\":" << write_ms
             << ",\"width\":" << width << ",\"height\":" << height
             << ",\"raster_fnv1a64\":\"" << std::hex << raster_hash << std::dec << "\""
             << ",\"peak_rss_kib\":" << peak_rss_kib()
             << ",\"request_ms\":" << elapsed_ms(request_start) << "}";
        return json.str();
    } catch (const std::exception &error) {
        std::ostringstream json;
        json << "{\"engine\":\"" << kEngine << "\",\"status\":\"error\""
             << ",\"profile\":\"" << json_escape(profile) << "\""
             << ",\"path\":\"" << json_escape(path) << "\""
             << ",\"error\":\"" << json_escape(error.what()) << "\""
             << ",\"request_ms\":" << elapsed_ms(request_start) << "}";
        return json.str();
    }
}

std::vector<std::string> split_tabs(const std::string &line) {
    std::vector<std::string> fields;
    size_t start = 0;
    while (true) {
        const size_t at = line.find('\t', start);
        fields.push_back(line.substr(start, at == std::string::npos ? at : at - start));
        if (at == std::string::npos) break;
        start = at + 1;
    }
    return fields;
}

} // namespace

int main(int argc, char **argv) {
#if defined(PEBQ_PDFIUM)
    FPDF_InitLibrary();
#endif
    int code = 0;
    if (argc >= 4 && std::string(argv[1]) == "--request") {
        const std::string profile = argv[2];
        const std::string path = argv[3];
        const int dpi = argc >= 5 ? std::stoi(argv[4]) : 144;
        const std::string output = argc >= 6 ? argv[5] : "-";
        const std::string result = execute(profile, path, dpi, output);
        std::cout << result << '\n';
        code = result.find("\"status\":\"ok\"") == std::string::npos ? 1 : 0;
    } else if (argc == 2 && std::string(argv[1]) == "--server") {
        std::string line;
        while (std::getline(std::cin, line)) {
            const auto fields = split_tabs(line);
            if (fields.size() < 2) {
                std::cout << "{\"engine\":\"" << kEngine
                          << "\",\"status\":\"error\",\"error\":\"invalid request\"}\n";
            } else {
                const int dpi = fields.size() >= 3 && !fields[2].empty() ? std::stoi(fields[2]) : 144;
                const std::string output = fields.size() >= 4 ? fields[3] : "-";
                std::cout << execute(fields[0], fields[1], dpi, output) << '\n';
            }
            std::cout.flush();
        }
    } else {
        std::cerr << "usage: " << argv[0]
                  << " --request <page-count|render> <pdf> [dpi] [output.ppm]\n"
                  << "       " << argv[0] << " --server\n";
        code = 2;
    }
#if defined(PEBQ_PDFIUM)
    FPDF_DestroyLibrary();
#endif
    return code;
}
