package org.oxidepdf;

import java.io.IOException;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemoryLayout;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.net.URISyntaxException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

public final class Oxide {
    private Oxide() {
    }

    public static String featureReportJson() {
        return Native.featureReportJson();
    }

    public static String codecIsolationReportJson(String filter, byte[] encodedBytes, String policy) {
        Objects.requireNonNull(filter, "filter");
        Objects.requireNonNull(encodedBytes, "encodedBytes");
        return Native.codecIsolationReportJson(filter, encodedBytes, policy);
    }

    public static String engineVersion() {
        return Native.engineVersion();
    }

    public static int abiVersion() {
        return Native.abiVersion();
    }

    public static final class OxideException extends RuntimeException {
        private final int status;

        OxideException(String message, int status) {
            super(message);
            this.status = status;
        }

        public int status() {
            return status;
        }
    }

    public record BinaryResult(byte[] bytes, String reportJson) {
        public void writeBytes(Path path) throws IOException {
            Files.write(path, bytes);
        }
    }

    public static final class Document implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        private Document(MemorySegment handle) {
            this.handle = handle;
        }

        public static Document open(Path path) throws IOException {
            return open(path, null);
        }

        public static Document open(Path path, String password) throws IOException {
            Objects.requireNonNull(path, "path");
            return open(Files.readAllBytes(path), password);
        }

        public static Document open(byte[] bytes) {
            return open(bytes, null);
        }

        public static Document open(byte[] bytes, String password) {
            Objects.requireNonNull(bytes, "bytes");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment passwordPtr = MemorySegment.NULL;
                long passwordLen = 0;
                if (password != null) {
                    byte[] passwordBytes = password.getBytes(StandardCharsets.UTF_8);
                    passwordLen = passwordBytes.length;
                    passwordPtr = arena.allocate(Math.max(passwordBytes.length, 1));
                    if (passwordBytes.length > 0) {
                        passwordPtr.copyFrom(MemorySegment.ofArray(passwordBytes));
                    }
                }
                MemorySegment handle = (MemorySegment) Native.OPEN_WITH_PASSWORD.invokeExact(
                    data,
                    (long) bytes.length,
                    passwordPtr,
                    passwordLen,
                    err
                );
                if (Native.isNull(handle)) {
                    Native.throwError(2, err);
                }
                return new Document(handle);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide native open failed", ex);
            }
        }

        public int pageCount() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment count = arena.allocate(ValueLayout.JAVA_LONG);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PAGE_COUNT.invokeExact(handle, count, err);
                Native.throwError(status, err);
                return Math.toIntExact(count.get(ValueLayout.JAVA_LONG, 0));
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide page_count failed", ex);
            }
        }

        public Page page(int pageNumber) {
            if (pageNumber < 1 || pageNumber > pageCount()) {
                throw new IndexOutOfBoundsException("Page numbers are 1-based");
            }
            return new Page(this, pageNumber);
        }

        public List<Page> pages() {
            int count = pageCount();
            var pages = new ArrayList<Page>(count);
            for (int idx = 1; idx <= count; idx++) {
                pages.add(new Page(this, idx));
            }
            return List.copyOf(pages);
        }

        public String extractText(int pageNumber) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.EXTRACT_TEXT.invokeExact(handle, (long) pageNumber, textOut, err);
                Native.throwError(status, err);
                return Native.takeString(textOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide extract_text failed", ex);
            }
        }

        public String parseJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PARSE_JSON.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide parse_json failed", ex);
            }
        }

        public String securityReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SECURITY_REPORT, "security_report");
        }

        public String parserReportJson(String mode) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.PARSER_REPORT, mode, "parser_report");
        }

        public String colorReportJson(String profile) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.COLOR_REPORT, profile, "color_report");
        }

        public String validateJson(String profile) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.VALIDATE, profile, "validate");
        }

        public String formsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.FORMS_REPORT, "forms_report");
        }

        public String annotationsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ANNOTATIONS_REPORT, "annotations_report");
        }

        public String pagesReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PAGES_REPORT, "pages_report");
        }

        public String interactiveReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.INTERACTIVE_REPORT, "interactive_report");
        }

        public String chunksJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.CHUNKS, "chunks");
        }

        public String advancedChunksJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ADVANCED_CHUNKS, "advanced_chunks");
        }

        public String semanticBundleJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SEMANTIC_BUNDLE, "semantic_bundle");
        }

        public String semanticSearchJson(String query) {
            ensureOpen();
            Objects.requireNonNull(query, "query");
            if (query.isBlank()) {
                throw new IllegalArgumentException("query must not be blank");
            }
            return Native.documentStringReport(handle, Native.SEMANTIC_SEARCH, query, "semantic_search");
        }

        public BinaryResult sanitize(String policy) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.SANITIZE, policy, "sanitize");
        }

        public BinaryResult canonicalize(Long dateEpoch) {
            ensureOpen();
            return Native.canonicalize(handle, dateEpoch);
        }

        public BinaryResult redactTerms(List<String> terms, boolean strict) {
            ensureOpen();
            return Native.redactTerms(handle, terms, strict);
        }

        public byte[] toDocx(boolean includeImages) {
            ensureOpen();
            return Native.documentToBytes(handle, Native.TO_DOCX, includeImages ? 1 : 0);
        }

        public byte[] toXlsx(String layout) {
            ensureOpen();
            Objects.requireNonNull(layout, "layout");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment layoutPtr = arena.allocateFrom(layout);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.TO_XLSX.invokeExact(handle, layoutPtr, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide to_xlsx failed", ex);
            }
        }

        public byte[] toPptx(boolean includeImages) {
            ensureOpen();
            return Native.documentToBytes(handle, Native.TO_PPTX, includeImages ? 1 : 0);
        }

        @Override
        public void close() {
            if (!closed) {
                try {
                    Native.FREE_DOC.invokeExact(handle);
                } catch (Throwable ex) {
                    throw new IllegalStateException("Oxide document free failed", ex);
                } finally {
                    closed = true;
                    handle = MemorySegment.NULL;
                }
            }
        }

        private void ensureOpen() {
            if (closed) {
                throw new IllegalStateException("Document is closed");
            }
        }
    }

    public record Page(Document document, int number) {
        public String text() {
            return document.extractText(number);
        }
    }

    public static final class Office {
        private Office() {
        }

        public static byte[] docxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.DOCX_TO_PDF);
        }

        public static byte[] xlsxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.XLSX_TO_PDF);
        }

        public static byte[] pptxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.PPTX_TO_PDF);
        }
    }

    private static final class Native {
        private static final Linker LINKER = Linker.nativeLinker();
        private static final Arena LOOKUP_ARENA = Arena.ofAuto();
        private static final SymbolLookup LOOKUP = loadLibrary();
        private static final MemoryLayout BUFFER_LAYOUT = MemoryLayout.structLayout(
            ValueLayout.ADDRESS.withName("data"),
            ValueLayout.JAVA_LONG.withName("len")
        );
        private static final long BUFFER_LEN_OFFSET = ValueLayout.ADDRESS.byteSize();
        private static final long ADDRESS_SIZE = ValueLayout.ADDRESS.byteSize();

        private static final MethodHandle OPEN = downcall(
            "oxide_document_open_from_bytes",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle OPEN_WITH_PASSWORD = downcall(
            "oxide_document_open_from_bytes_with_password",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FREE_DOC = downcall(
            "oxide_document_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle STRING_FREE = downcall(
            "oxide_string_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle ERROR_FREE = downcall(
            "oxide_error_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle BUFFER_FREE = downcall(
            "oxide_buffer_free",
            FunctionDescriptor.ofVoid(BUFFER_LAYOUT)
        );
        private static final MethodHandle PAGE_COUNT = downcall(
            "oxide_document_page_count",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EXTRACT_TEXT = downcall(
            "oxide_document_extract_text",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PARSE_JSON = downcall(
            "oxide_document_parse_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_XLSX = downcall(
            "oxide_document_to_xlsx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_PPTX = downcall(
            "oxide_document_to_pptx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_DOCX = downcall(
            "oxide_document_to_docx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DOCX_TO_PDF = downcall(
            "oxide_docx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle XLSX_TO_PDF = downcall(
            "oxide_xlsx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PPTX_TO_PDF = downcall(
            "oxide_pptx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SECURITY_REPORT = documentReport("oxide_document_security_report_json");
        private static final MethodHandle PARSER_REPORT = documentStringReport("oxide_document_parser_report_json");
        private static final MethodHandle COLOR_REPORT = documentStringReport("oxide_document_color_report_json");
        private static final MethodHandle VALIDATE = documentStringReport("oxide_document_validate_json");
        private static final MethodHandle FORMS_REPORT = documentReport("oxide_document_forms_report_json");
        private static final MethodHandle ANNOTATIONS_REPORT = documentReport("oxide_document_annotations_report_json");
        private static final MethodHandle PAGES_REPORT = documentReport("oxide_document_pages_report_json");
        private static final MethodHandle INTERACTIVE_REPORT = documentReport("oxide_document_interactive_report_json");
        private static final MethodHandle CHUNKS = documentReport("oxide_document_chunks_json");
        private static final MethodHandle ADVANCED_CHUNKS = documentReport("oxide_document_advanced_chunks_json");
        private static final MethodHandle SEMANTIC_BUNDLE = documentReport("oxide_document_semantic_bundle_json");
        private static final MethodHandle SEMANTIC_SEARCH = documentStringReport("oxide_document_semantic_search_json");
        private static final MethodHandle SANITIZE = downcall(
            "oxide_document_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle CANONICALIZE = downcall(
            "oxide_document_canonicalize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle REDACT = downcall(
            "oxide_document_redact_terms_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FEATURE_REPORT = downcall(
            "oxide_feature_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle CODEC_ISOLATION_REPORT = downcall(
            "oxide_codec_isolation_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle VERSION = downcall(
            "oxide_version",
            FunctionDescriptor.of(ValueLayout.ADDRESS)
        );
        private static final MethodHandle ABI_VERSION = downcall(
            "oxide_abi_version",
            FunctionDescriptor.of(ValueLayout.JAVA_INT)
        );

        private static SymbolLookup loadLibrary() {
            Path path = findNativeLibrary();
            if (path == null) {
                throw new IllegalStateException(
                    "Could not locate oxide_capi native library. Set OXIDE_NATIVE_LIBRARY or place it under target/debug, target/release, or runtimes/<rid>/native.");
            }
            return SymbolLookup.libraryLookup(path, LOOKUP_ARENA);
        }

        private static Path findNativeLibrary() {
            String explicit = System.getenv("OXIDE_NATIVE_LIBRARY");
            if (explicit != null && !explicit.isBlank() && Files.exists(Path.of(explicit))) {
                return Path.of(explicit);
            }

            String mapped = mappedLibraryName();
            String rid = runtimeIdentifier();
            Path cwd = Path.of("").toAbsolutePath();
            List<Path> roots = new ArrayList<>();
            roots.add(cwd);
            roots.add(cwd.resolve("target/debug"));
            roots.add(cwd.resolve("target/release"));
            Path packageBase = packageBase();
            if (packageBase != null) {
                roots.add(packageBase);
            }
            for (Path root : roots) {
                Path direct = root.resolve(mapped);
                if (Files.exists(direct)) {
                    return direct;
                }
                Path ridPath = root.resolve("runtimes").resolve(rid).resolve("native").resolve(mapped);
                if (Files.exists(ridPath)) {
                    return ridPath;
                }
            }
            return null;
        }

        private static Path packageBase() {
            try {
                Path location = Path.of(Oxide.class.getProtectionDomain().getCodeSource().getLocation().toURI());
                return Files.isRegularFile(location) ? location.getParent() : location;
            } catch (NullPointerException | SecurityException | URISyntaxException ex) {
                return null;
            }
        }

        private static String mappedLibraryName() {
            String os = System.getProperty("os.name", "").toLowerCase();
            if (os.contains("win")) {
                return "oxide_capi.dll";
            }
            if (os.contains("mac") || os.contains("darwin")) {
                return "liboxide_capi.dylib";
            }
            return "liboxide_capi.so";
        }

        private static String runtimeIdentifier() {
            String os = System.getProperty("os.name", "").toLowerCase();
            String osPart = os.contains("win") ? "win" : os.contains("mac") || os.contains("darwin") ? "osx" : "linux";
            String arch = System.getProperty("os.arch", "").toLowerCase();
            String archPart = arch.contains("aarch64") || arch.contains("arm64") ? "arm64" : arch.contains("86") && !arch.contains("64") ? "x86" : "x64";
            return osPart + "-" + archPart;
        }

        private static MethodHandle documentReport(String name) {
            return downcall(
                name,
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
            );
        }

        private static MethodHandle documentStringReport(String name) {
            return downcall(
                name,
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
            );
        }

        private static MethodHandle downcall(String name, FunctionDescriptor descriptor) {
            MemorySegment symbol = LOOKUP.find(name)
                .orElseThrow(() -> new UnsatisfiedLinkError("Missing native symbol " + name));
            return LINKER.downcallHandle(symbol, descriptor);
        }

        private static String featureReportJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) FEATURE_REPORT.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide feature_report failed", ex);
            }
        }

        private static String codecIsolationReportJson(String filter, byte[] encodedBytes, String policy) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment filterPtr = arena.allocateFrom(filter);
                MemorySegment data = encodedBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(encodedBytes.length);
                if (encodedBytes.length > 0) {
                    data.copyFrom(MemorySegment.ofArray(encodedBytes));
                }
                MemorySegment policyPtr = policy == null || policy.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(policy);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) CODEC_ISOLATION_REPORT.invokeExact(
                    filterPtr,
                    data,
                    (long) encodedBytes.length,
                    policyPtr,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return takeString(jsonOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide codec isolation report failed", ex);
            }
        }

        private static String engineVersion() {
            try (Arena ignored = Arena.ofConfined()) {
                MemorySegment ptr = (MemorySegment) VERSION.invokeExact();
                if (isNull(ptr)) {
                    return "";
                }
                try {
                    return ptr.reinterpret(Long.MAX_VALUE).getString(0);
                } finally {
                    STRING_FREE.invokeExact(ptr);
                }
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide version query failed", ex);
            }
        }

        private static int abiVersion() {
            try {
                return (int) ABI_VERSION.invokeExact();
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide ABI version query failed", ex);
            }
        }

        private static String documentReport(MemorySegment handle, MethodHandle method, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide " + operation + " failed", ex);
            }
        }

        private static String documentStringReport(MemorySegment handle, MethodHandle method, String value, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment arg = value == null || value.isBlank() ? MemorySegment.NULL : arena.allocateFrom(value);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, arg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide " + operation + " failed", ex);
            }
        }

        private static BinaryResult documentStringOutput(MemorySegment handle, MethodHandle method, String value, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment arg = value == null || value.isBlank() ? MemorySegment.NULL : arena.allocateFrom(value);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, arg, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide " + operation + " failed", ex);
            }
        }

        private static BinaryResult canonicalize(MemorySegment handle, Long dateEpoch) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                long epoch = dateEpoch == null ? 0L : dateEpoch.longValue();
                int status = (int) CANONICALIZE.invokeExact(handle, epoch, dateEpoch == null ? 0 : 1, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide canonicalize failed", ex);
            }
        }

        private static BinaryResult redactTerms(MemorySegment handle, List<String> terms, boolean strict) {
            Objects.requireNonNull(terms, "terms");
            List<String> clean = terms.stream().filter(t -> t != null && !t.isBlank()).toList();
            if (clean.isEmpty()) {
                throw new IllegalArgumentException("At least one non-empty redaction term is required");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment termArray = arena.allocate(ADDRESS_SIZE * clean.size());
                for (int i = 0; i < clean.size(); i++) {
                    termArray.set(ValueLayout.ADDRESS, ADDRESS_SIZE * i, arena.allocateFrom(clean.get(i)));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) REDACT.invokeExact(handle, termArray, (long) clean.size(), strict ? 1 : 0, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide redact_terms failed", ex);
            }
        }

        private static void throwError(int status, MemorySegment errorPtr) throws Throwable {
            if (status == 0) {
                MemorySegment maybeError = errorPtr.get(ValueLayout.ADDRESS, 0);
                if (!isNull(maybeError)) {
                    ERROR_FREE.invokeExact(maybeError);
                }
                return;
            }
            MemorySegment err = errorPtr.get(ValueLayout.ADDRESS, 0);
            String message = isNull(err)
                ? "Oxide native call failed with status " + status
                : err.reinterpret(Long.MAX_VALUE).getString(0);
            if (!isNull(err)) {
                ERROR_FREE.invokeExact(err);
            }
            throw new OxideException(message, status);
        }

        private static String takeString(MemorySegment outPtr) throws Throwable {
            MemorySegment ptr = outPtr.get(ValueLayout.ADDRESS, 0);
            if (isNull(ptr)) {
                return "";
            }
            try {
                return ptr.reinterpret(Long.MAX_VALUE).getString(0);
            } finally {
                STRING_FREE.invokeExact(ptr);
            }
        }

        private static byte[] documentToBytes(MemorySegment handle, MethodHandle method, int flag) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, flag, buffer, err);
                throwError(status, err);
                return takeBuffer(buffer);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide document conversion failed", ex);
            }
        }

        private static byte[] officeToPdf(byte[] bytes, MethodHandle method) {
            Objects.requireNonNull(bytes, "bytes");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(data, (long) bytes.length, buffer, err);
                throwError(status, err);
                return takeBuffer(buffer);
            } catch (OxideException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Oxide office-to-pdf conversion failed", ex);
            }
        }

        private static byte[] takeBuffer(MemorySegment buffer) throws Throwable {
            MemorySegment data = buffer.get(ValueLayout.ADDRESS, 0);
            long len = buffer.get(ValueLayout.JAVA_LONG, BUFFER_LEN_OFFSET);
            if (isNull(data) || len <= 0) {
                return new byte[0];
            }
            try {
                return data.reinterpret(len).toArray(ValueLayout.JAVA_BYTE);
            } finally {
                BUFFER_FREE.invokeExact(buffer);
            }
        }

        private static boolean isNull(MemorySegment segment) {
            return segment == null || segment.equals(MemorySegment.NULL) || segment.address() == 0;
        }
    }
}
