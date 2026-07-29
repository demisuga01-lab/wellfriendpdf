package io.wellfriendpdf.examples;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import io.wellfriendpdf.Wellfriend;

public final class SecureMutationSecureMutation {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: SecureMutationSecureMutation input.pdf");
        }
        try (WellfriendPdf.Document document = WellfriendPdf.Document.open(Path.of(args[0]))) {
            System.out.println(document.secure_mutationReportJson());
            System.out.println(document.associatedFilesReportJson());
            System.out.println(document.editPolicyReportJson("attachment_remove"));
            String options = "{\"filename\":\"evidence.txt\",\"mime\":\"text/plain\",\"relationship\":\"data\",\"deterministic\":true}";
            WellfriendPdf.BinaryResult added = document.associatedFileAdd(
                "bounded evidence".getBytes(StandardCharsets.UTF_8), options);
            Files.write(Path.of("secure_mutation-associated.pdf"), added.bytes());
        }
    }
}
