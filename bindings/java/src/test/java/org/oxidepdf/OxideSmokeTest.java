package org.oxidepdf;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.Map;

public final class OxideSmokeTest {
    public static void main(String[] args) throws Exception {
        Path fixture = fixturePath();
        try (Oxide.Document doc = Oxide.Document.open(fixture)) {
            assertTrue(doc.pageCount() >= 1, "page count");
            assertTrue(!doc.page(1).text().isBlank(), "text extraction");
            assertTrue(doc.parseJson().contains("\"schema_version\""), "parse json");
            Map<String, String> reports = new LinkedHashMap<>();
            reports.put("feature", Oxide.featureReportJson());
            reports.put("security", doc.securityReportJson());
            reports.put("parser", doc.parserReportJson("repair"));
            reports.put("color", doc.colorReportJson("generic"));
            reports.put("validate_security", doc.validateJson("security"));
            reports.put("forms", doc.formsReportJson());
            reports.put("annotations", doc.annotationsReportJson());
            reports.put("pages", doc.pagesReportJson());
            reports.put("interactive", doc.interactiveReportJson());
            reports.put("chunks", doc.chunksJson());
            assertTrue(reports.get("feature").contains("feature_report"), "feature report");
            assertTrue(!Oxide.engineVersion().isBlank(), "engine version");
            assertTrue(Oxide.abiVersion() >= 1, "abi version");
            for (Map.Entry<String, String> entry : reports.entrySet()) {
                assertReport(entry.getValue(), entry.getKey() + " report");
            }

            byte[] docx = doc.toDocx(true);
            byte[] xlsx = doc.toXlsx("pages");
            byte[] pptx = doc.toPptx(true);
            Oxide.BinaryResult sanitized = doc.sanitize("balanced");
            Oxide.BinaryResult canonicalized = doc.canonicalize(0L);
            reports.put("sanitize", sanitized.reportJson());
            reports.put("canonicalize", canonicalized.reportJson());
            assertPrefix(docx, "PK", "docx");
            assertPrefix(xlsx, "PK", "xlsx");
            assertPrefix(pptx, "PK", "pptx");
            assertPrefix(sanitized.bytes(), "%PDF-", "sanitized pdf");
            assertPrefix(canonicalized.bytes(), "%PDF-", "canonicalized pdf");
            assertReport(sanitized.reportJson(), "sanitize report");
            assertReport(canonicalized.reportJson(), "canonicalize report");
            assertPrefix(Oxide.Office.docxToPdf(docx), "%PDF-", "docx pdf");
            assertPrefix(Oxide.Office.xlsxToPdf(xlsx), "%PDF-", "xlsx pdf");
            assertPrefix(Oxide.Office.pptxToPdf(pptx), "%PDF-", "pptx pdf");
            writePrompt02Artifact(fixture, reports, sanitized, canonicalized);
        }

        try (Oxide.Document emptyPassword = Oxide.Document.open(fixture, "")) {
            assertTrue(emptyPassword.pageCount() >= 1, "explicit empty password open");
        }
        try (Oxide.Document ignoredPassword = Oxide.Document.open(
                Files.readAllBytes(fixture),
                "ignored-for-unencrypted")) {
            assertTrue(ignoredPassword.pageCount() >= 1, "password open from bytes");
        }
        String feature = Oxide.featureReportJson();
        assertTrue(feature.contains("\"progress\""), "progress feature posture");
        assertTrue(feature.contains("progress_not_supported"), "progress unsupported status");
        assertTrue(feature.contains("\"cancellation\""), "cancellation feature posture");
        assertTrue(
            feature.contains("cancellation_not_supported_for_prompt02_bindings"),
            "cancellation unsupported status");
        assertTrue(feature.contains("\"codec_isolation\""), "codec isolation feature posture");
        assertTrue(feature.contains("\"prompt07_transparency_compositing\""), "prompt07 feature posture");
        assertTrue(
            feature.contains("native_foundation_with_prompt07b_closure"),
            "prompt07 native foundation status");
        assertTrue(feature.contains("\"prompt07b_transparency_closure\""), "prompt07b closure posture");
        assertTrue(feature.contains("\"oxide_outlier_failures\":0"), "prompt07b outlier count");
        assertTrue(feature.contains("\"memory_cap_mb\":4096"), "prompt07 memory cap");
        assertTrue(feature.contains("\"Luminosity\""), "prompt07 blend mode report");
        String isolation = Oxide.codecIsolationReportJson(
            "FlateDecode",
            "not-decoded-in-report-only".getBytes(StandardCharsets.UTF_8),
            "report_only");
        assertTrue(isolation.contains("codec_isolation_report"), "codec isolation report");
        assertTrue(isolation.contains("report_only"), "codec isolation report_only status");

        for (int i = 0; i < 25; i++) {
            try (Oxide.Document doc = Oxide.Document.open(fixture)) {
                assertTrue(doc.pageCount() >= 1, "stress page count");
                assertReport(doc.securityReportJson(), "stress security report");
            }
        }

        boolean threw = false;
        try {
            Oxide.Document.open(new byte[] {1, 2, 3, 4});
        } catch (Oxide.OxideException expected) {
            threw = expected.status() != 0;
        }
        assertTrue(threw, "malformed input exception");

        String secret = "do-not-echo-java-password";
        try {
            Oxide.Document.open(new byte[] {1, 2, 3, 4}, secret);
            throw new AssertionError("malformed password open should fail");
        } catch (Oxide.OxideException expected) {
            assertTrue(!expected.getMessage().contains(secret), "password not echoed");
        }
    }

    private static void assertReport(String json, String label) {
        assertTrue(json.contains("\"schema_version\""), label);
    }

    private static void assertPrefix(byte[] bytes, String expected, String label) {
        String actual = new String(bytes, 0, Math.min(bytes.length, expected.length()), StandardCharsets.US_ASCII);
        assertTrue(expected.equals(actual), label + " prefix");
    }

    private static void writePrompt02Artifact(
            Path fixture,
            Map<String, String> reports,
            Oxide.BinaryResult sanitized,
            Oxide.BinaryResult canonicalized) throws Exception {
        String dir = System.getenv("OXIDE_PROMPT02_ARTIFACT_DIR");
        if (dir == null || dir.isBlank()) {
            return;
        }

        Path artifactDir = Path.of(dir);
        Files.createDirectories(artifactDir);
        StringBuilder json = new StringBuilder();
        json.append("{\n");
        json.append("  \"surface\": \"java\",\n");
        json.append("  \"fixture\": \"").append(escape(fixture.toString())).append("\",\n");
        json.append("  \"engine_version\": \"").append(escape(Oxide.engineVersion())).append("\",\n");
        json.append("  \"abi_version\": ").append(Oxide.abiVersion()).append(",\n");
        json.append("  \"reports\": {\n");
        int index = 0;
        for (Map.Entry<String, String> entry : reports.entrySet()) {
            if (index++ > 0) {
                json.append(",\n");
            }
            byte[] bytes = entry.getValue().getBytes(StandardCharsets.UTF_8);
            json.append("    \"").append(escape(entry.getKey())).append("\": {")
                .append("\"sha256\": \"").append(sha256(bytes)).append("\", ")
                .append("\"bytes\": ").append(bytes.length).append("}");
        }
        json.append("\n  },\n");
        json.append("  \"outputs\": {\n");
        json.append("    \"sanitized\": {\"bytes\": ").append(sanitized.bytes().length)
            .append(", \"sha256\": \"").append(sha256(sanitized.bytes())).append("\"},\n");
        json.append("    \"canonicalized\": {\"bytes\": ").append(canonicalized.bytes().length)
            .append(", \"sha256\": \"").append(sha256(canonicalized.bytes())).append("\"}\n");
        json.append("  }\n");
        json.append("}\n");
        Files.writeString(artifactDir.resolve("java-smoke.json"), json.toString(), StandardCharsets.UTF_8);
    }

    private static String sha256(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    private static String escape(String value) {
        return value.replace("\\", "\\\\").replace("\"", "\\\"");
    }

    private static void assertTrue(boolean value, String label) {
        if (!value) {
            throw new AssertionError(label);
        }
    }

    private static Path fixturePath() throws Exception {
        String env = System.getenv("OXIDE_FIXTURE_PDF");
        if (env != null && !env.isBlank() && Files.exists(Path.of(env))) {
            return Path.of(env);
        }
        Path dir = Path.of("").toAbsolutePath();
        while (dir != null) {
            Path candidate = dir.resolve("crates/engine/tests/fixtures/tracemonkey.pdf");
            if (Files.exists(candidate)) {
                return candidate;
            }
            dir = dir.getParent();
        }
        throw new IllegalStateException("Could not locate tracemonkey.pdf fixture");
    }
}
