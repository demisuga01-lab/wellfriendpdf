package io.wellfriendpdf.examples;

import java.nio.file.Files;
import java.nio.file.Path;
import io.wellfriendpdf.Wellfriend;

public final class FormActionPolicyInteractiveDocx {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: FormActionPolicyInteractiveDocx input.pdf");
        }
        try (WellfriendPdf.Document document = WellfriendPdf.Document.open(Path.of(args[0]))) {
            System.out.println(document.formJavaScriptReportJson());
            System.out.println(document.interactiveDataReportJson());
            System.out.println(document.form_action_policyReportJson());
            Files.write(
                Path.of("form_action_policy-page-faithful.docx"),
                document.toDocx("page-faithful", true));
        }
    }
}
