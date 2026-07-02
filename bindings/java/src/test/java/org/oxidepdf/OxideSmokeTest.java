package org.oxidepdf;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

public final class OxideSmokeTest {
    public static void main(String[] args) throws Exception {
        Path fixture = fixturePath();
        try (Oxide.Document doc = Oxide.Document.open(fixture)) {
            assertTrue(doc.pageCount() >= 1, "page count");
            assertTrue(!doc.page(1).text().isBlank(), "text extraction");
            assertTrue(doc.parseJson().contains("\"schema_version\""), "parse json");

            byte[] docx = doc.toDocx(true);
            byte[] xlsx = doc.toXlsx("pages");
            byte[] pptx = doc.toPptx(true);
            assertPrefix(docx, "PK", "docx");
            assertPrefix(xlsx, "PK", "xlsx");
            assertPrefix(pptx, "PK", "pptx");
            assertPrefix(Oxide.Office.docxToPdf(docx), "%PDF-", "docx pdf");
            assertPrefix(Oxide.Office.xlsxToPdf(xlsx), "%PDF-", "xlsx pdf");
            assertPrefix(Oxide.Office.pptxToPdf(pptx), "%PDF-", "pptx pdf");
        }

        boolean threw = false;
        try {
            Oxide.Document.open(new byte[] {1, 2, 3, 4});
        } catch (Oxide.OxideException expected) {
            threw = expected.status() != 0;
        }
        assertTrue(threw, "malformed input exception");
    }

    private static void assertPrefix(byte[] bytes, String expected, String label) {
        String actual = new String(bytes, 0, Math.min(bytes.length, expected.length()), StandardCharsets.US_ASCII);
        assertTrue(expected.equals(actual), label + " prefix");
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
