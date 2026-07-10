# Oxide.Sdk

Idiomatic .NET binding for Oxide's Rust PDF engine. The package wraps the
stable C ABI with P/Invoke and returns the same versioned JSON report envelopes
as Rust, Python, C ABI, WASM, and Java.

```csharp
using Oxide.Sdk;

using var doc = OxideDocument.Open("report.pdf", password: null);
Console.WriteLine(doc.PageCount);
Console.WriteLine(doc.GetPage(1).Text);
Console.WriteLine(doc.SecurityReportJson());
Console.WriteLine(doc.SemanticBundleJson());
Console.WriteLine(doc.AdvancedChunksJson());
Console.WriteLine(doc.SemanticSearchJson("invoice"));
Console.WriteLine(OxideDocument.CodecIsolationReportJson(
    "FlateDecode",
    Convert.FromHexString("789ccb48cdc9c957c8afc84c49050019dd044e"),
    "in_process"));

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
forms, annotations, page operations, interactive content, legacy chunks,
Prompt 15 semantic bundles, advanced chunks, and provenance-aware search.

Outputs: sanitize, canonicalize, redact terms, DOCX, XLSX, PPTX, and Office to
PDF conversion helpers.

Password open is available through `OxideDocument.Open(path, password)` and
`OxideDocument.Open(bytes, password)`. Passwords are UTF-8 operation-scoped
inputs and are not retained on the managed document object.

Known limits: progress and cancellation are reported through
`FeatureReportJson()` as unsupported for the Prompt 02 binding surface; no
no-op callbacks or ignored `CancellationToken` overloads are exposed. Mobile
packaging is out of scope for this package.
