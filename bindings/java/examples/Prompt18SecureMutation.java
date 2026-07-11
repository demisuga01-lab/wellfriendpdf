package org.oxidepdf.examples;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.oxidepdf.Oxide;

public final class Prompt18SecureMutation {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: Prompt18SecureMutation input.pdf");
        }
        try (Oxide.Document document = Oxide.Document.open(Path.of(args[0]))) {
            System.out.println(document.prompt18ReportJson());
            System.out.println(document.associatedFilesReportJson());
            System.out.println(document.editPolicyReportJson("attachment_remove"));
            String options = "{\"filename\":\"evidence.txt\",\"mime\":\"text/plain\",\"relationship\":\"data\",\"deterministic\":true}";
            Oxide.BinaryResult added = document.associatedFileAdd(
                "bounded evidence".getBytes(StandardCharsets.UTF_8), options);
            Files.write(Path.of("prompt18-associated.pdf"), added.bytes());
        }
    }
}
