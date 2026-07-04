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
- `OxideDocument.Open(string path, string? password = null)`
- `OxideDocument.Open(byte[] bytes, string? password = null)`
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

Prompt 02B added password-aware open overloads. Passwords are encoded as UTF-8
for the native open call, are not logged, and are not retained on the managed
document object. C ABI tests cover correct/wrong passwords on a generated
encrypted fixture; .NET tests cover the public overloads and verify malformed
input errors do not echo the supplied password.

Progress and cancellation tokens are not exposed as .NET callbacks or
`CancellationToken` overloads because the Prompt 02 report/output facade calls
used by this binding do not observe binding-level tokens. Query
`FeatureReportJson()` for the machine-readable `progress_not_supported` and
`cancellation_not_supported_for_prompt02_bindings` statuses.
