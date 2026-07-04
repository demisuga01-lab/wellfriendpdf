# Oxide.Sdk

Idiomatic .NET binding for Oxide's Rust PDF engine. The package wraps the
stable C ABI with P/Invoke and returns the same versioned JSON report envelopes
as Rust, Python, C ABI, WASM, and Java.

```csharp
using Oxide.Sdk;

using var doc = OxideDocument.Open("report.pdf");
Console.WriteLine(doc.PageCount);
Console.WriteLine(doc.GetPage(1).Text);
Console.WriteLine(doc.SecurityReportJson());

var sanitized = doc.Sanitize("balanced");
File.WriteAllBytes("sanitized.pdf", sanitized.Bytes);
Console.WriteLine(sanitized.ReportJson);

File.WriteAllBytes("report.docx", doc.ToDocx());
File.WriteAllBytes("report.xlsx", doc.ToXlsx(layout: "pages"));
File.WriteAllBytes("report.pptx", doc.ToPptx());
File.WriteAllBytes("from-word.pdf", OfficeConverters.DocxToPdf("report.docx"));
```

## Native Loading

During development, set `OXIDE_NATIVE_LIBRARY` to the platform-specific
`oxide_capi` dynamic library. When packaged, the resolver also checks:

- `AppContext.BaseDirectory`
- the assembly directory
- the current directory
- `target/debug` and `target/release`
- `runtimes/<rid>/native`

Use `using` or `Dispose()` for documents. Native handles are owned by
`SafeHandle`; output buffers are copied into managed `byte[]` values and freed
before methods return.

## Prompt 02 Surface

Reports: feature, engine/ABI version, security, parser, color, validation,
forms, annotations, page operations, interactive content, and chunks.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Known limits: password open is not yet exposed through the C ABI-backed .NET
surface; progress and cancellation are not exposed because current engine calls
do not observe binding-level tokens. Mobile packaging is out of scope for this
package.
