# .NET SDK Prompt 02

The .NET SDK lives under `bindings/dotnet/Oxide.Sdk` and wraps the stable C ABI
with P/Invoke. It returns the same versioned report JSON as the Rust facade and
C ABI.

## Package Shape

- Library project: `bindings/dotnet/Oxide.Sdk/Oxide.Sdk.csproj`
- Tests: `bindings/dotnet/Oxide.Sdk.Tests`
- Example: `bindings/dotnet/examples/Prompt02Reports.cs`
- NuGet metadata: package id, license, tags, readme, repository, documentation
  file generation.

## Public API

Lifecycle:

- `OxideDocument.Open(string path)`
- `OxideDocument.Open(byte[] bytes)`
- `Dispose()` / `using`
- `PageCount`, `Pages`, `GetPage(n)`, `ExtractText(n)`

Reports:

- `OxideDocument.FeatureReportJson()`
- `OxideDocument.EngineVersion()`, `OxideDocument.AbiVersion`
- `SecurityReportJson`
- `ParserReportJson`
- `ColorReportJson`
- `ValidateJson`
- `FormsReportJson`
- `AnnotationsReportJson`
- `PagesReportJson`
- `InteractiveReportJson`
- `ChunksJson`

Outputs:

- `Sanitize`
- `Canonicalize`
- `RedactTerms`
- `ToDocx`, `ToXlsx`, `ToPptx`
- `OfficeConverters.DocxToPdf`, `XlsxToPdf`, `PptxToPdf`

## Native Loading and Ownership

The resolver checks `OXIDE_NATIVE_LIBRARY`, the application/assembly/current
directories, local `target/debug` and `target/release`, and
`runtimes/<rid>/native`. Native document handles are owned by `SafeHandle`.
Native output buffers are copied into managed `byte[]` and freed immediately.
Errors preserve the C ABI status code and message in `OxideException`.

## Limits

Password open is not yet public because the current C ABI open function accepts
bytes only. Progress and cancellation tokens are not exposed because the engine
facade calls used here do not observe binding-level cancellation.
