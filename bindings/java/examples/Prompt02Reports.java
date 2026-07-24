package io.wellfriendpdf.examples;

import java.nio.file.Files;
import java.nio.file.Path;
import io.wellfriendpdf.Wellfriend;

public final class Prompt02Reports {
    private Prompt02Reports() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            System.err.println("usage: Prompt02Reports <pdf>");
            System.exit(2);
        }

        Path input = Path.of(args[0]);
        try (WellfriendPdf.Document doc = WellfriendPdf.Document.open(input)) {
            System.out.println(doc.securityReportJson());
            WellfriendPdf.BinaryResult sanitized = doc.sanitize("balanced");
            Files.write(input.resolveSibling(input.getFileName() + ".sanitized.pdf"), sanitized.bytes());
            System.out.println(sanitized.reportJson());
        }
    }
}
