# Oxide.Sdk

Idiomatic .NET binding for Oxide's Rust PDF engine.

```csharp
using Oxide.Sdk;

using var doc = OxideDocument.Open("report.pdf");
Console.WriteLine(doc.PageCount);
Console.WriteLine(doc.GetPage(1).Text);

File.WriteAllBytes("report.docx", doc.ToDocx());
File.WriteAllBytes("report.xlsx", doc.ToXlsx(layout: "pages"));
File.WriteAllBytes("report.pptx", doc.ToPptx());

File.WriteAllBytes("from-word.pdf", OfficeConverters.DocxToPdf("report.docx"));
```

The binding wraps the stable C ABI with P/Invoke. Set `OXIDE_NATIVE_LIBRARY` to
the platform-specific `oxide_capi` dynamic library during local development.
Native handles are owned by `SafeHandle`; dispose documents with `using`.

Mobile packaging is deliberately out of scope for this package.
