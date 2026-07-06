package org.oxidepdf.packagesmoke;

import java.nio.file.Files;
import java.nio.file.Path;

import org.oxidepdf.Oxide;

public final class PackageSmoke {
    private PackageSmoke() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: PackageSmoke <fixture.pdf>");
        }
        Path fixture = Path.of(args[0]);
        if (!Files.exists(fixture)) {
            throw new IllegalArgumentException("fixture does not exist: " + fixture);
        }
        Oxide.Document.class.getMethod("open", Path.class, String.class);
        Oxide.Document.class.getMethod("open", byte[].class, String.class);
        if (Oxide.engineVersion().isBlank() || Oxide.abiVersion() < 1) {
            throw new AssertionError("version queries failed");
        }

        try (Oxide.Document doc = Oxide.Document.open(fixture, "")) {
            if (doc.pageCount() < 1) {
                throw new AssertionError("expected at least one page");
            }
            if (doc.page(1).text().isBlank()) {
                throw new AssertionError("text extraction returned blank");
            }
            String security = doc.securityReportJson();
            if (!security.contains("\"schema_version\"")) {
                throw new AssertionError("security report missing schema_version");
            }
            String parser = doc.parserReportJson("repair");
            if (!parser.contains("\"schema_version\"")) {
                throw new AssertionError("parser report missing schema_version");
            }
            Oxide.BinaryResult sanitized = doc.sanitize("balanced");
            if (sanitized.bytes().length == 0 || !sanitized.reportJson().contains("sanitize_report")) {
                throw new AssertionError("sanitize output/report missing");
            }
        }

        String feature = Oxide.featureReportJson();
        if (!feature.contains("progress_not_supported")
                || !feature.contains("cancellation_not_supported_for_prompt02_bindings")) {
            throw new AssertionError("feature report missing Prompt 02B progress/cancellation posture");
        }
        if (!feature.contains("\"prompt07b_transparency_closure\"")
                || !feature.contains("\"oxide_outlier_failures\":0")) {
            throw new AssertionError("feature report missing Prompt 07B transparency closure posture");
        }
        if (!feature.contains("\"prompt08_text_clipping_shading_patterns\"")
                || !feature.contains("\"rendering_modes\":[4,5,6,7]")) {
            throw new AssertionError("feature report missing Prompt 08 text/shading/pattern posture");
        }
    }
}
