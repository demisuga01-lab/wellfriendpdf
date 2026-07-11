package org.oxidepdf.examples;

import java.nio.file.Files;
import java.nio.file.Path;
import org.oxidepdf.Oxide;

public final class Prompt19InteractiveDocx {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: Prompt19InteractiveDocx input.pdf");
        }
        try (Oxide.Document document = Oxide.Document.open(Path.of(args[0]))) {
            System.out.println(document.formJavaScriptReportJson());
            System.out.println(document.interactiveDataReportJson());
            System.out.println(document.prompt19ReportJson());
            Files.write(
                Path.of("prompt19-page-faithful.docx"),
                document.toDocx("page-faithful", true));
        }
    }
}
