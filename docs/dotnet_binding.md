# .NET Binding

The .NET binding lives under `bindings/dotnet/WellfriendPdf`. It wraps the stable C
ABI with P/Invoke and exposes an idiomatic C# layer:

- `WellfriendDocument.Open(path | bytes)`
- `doc.PageCount`
- `doc.GetPage(1).Text`
- `doc.ParseJson()`
- `doc.ExtractFieldsJson()`
- `doc.ToDocx()`, `doc.ToXlsx()`, `doc.ToPptx()`
- `OfficeConverters.DocxToPdf(...)`, `XlsxToPdf(...)`, `PptxToPdf(...)`

Native handles are owned by `SafeHandle`, so `using` / `Dispose()` releases Rust
resources deterministically.

```csharp
using WellfriendPdf;

using var doc = WellfriendDocument.Open("report.pdf");
Console.WriteLine(doc.GetPage(1).Text);
File.WriteAllBytes("report.docx", doc.ToDocx());
File.WriteAllBytes("report.pdf", OfficeConverters.DocxToPdf("report.docx"));
```

Development test command used on this host:

```powershell
$env:WELLFRIENDPDF_NATIVE_LIBRARY="E:\wellpdfsdk\target\debug\wellfriendpdf_capi.dll"
$env:WELLFRIENDPDF_FIXTURE_PDF="E:\wellpdfsdk\crates\engine\tests\fixtures\tracemonkey.pdf"
dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --nologo
```

Verified here: Windows x64, .NET SDK 10.0.103 targeting `net8.0`. Other
platforms are structurally supported through the C ABI but not verified in this
run. Mobile packaging is future work, not part of this binding.
