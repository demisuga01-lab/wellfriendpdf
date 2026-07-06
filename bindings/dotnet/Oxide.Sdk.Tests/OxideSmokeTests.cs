using System.Text;
using System.Text.Json;
using System.Security.Cryptography;
using Oxide.Sdk;
using Xunit;

namespace Oxide.Sdk.Tests;

public sealed class OxideSmokeTests
{
    [Fact]
    public void OpenExtractAndConvert()
    {
        using var doc = OxideDocument.Open(FixturePath());
        Assert.True(doc.PageCount >= 1);
        Assert.False(string.IsNullOrWhiteSpace(doc.GetPage(1).Text));
        Assert.Contains("\"schema_version\"", doc.ParseJson());
        var reports = new Dictionary<string, string>
        {
            ["feature"] = OxideDocument.FeatureReportJson(),
            ["security"] = doc.SecurityReportJson(),
            ["parser"] = doc.ParserReportJson(),
            ["color"] = doc.ColorReportJson(),
            ["validate_security"] = doc.ValidateJson("security"),
            ["forms"] = doc.FormsReportJson(),
            ["annotations"] = doc.AnnotationsReportJson(),
            ["pages"] = doc.PagesReportJson(),
            ["interactive"] = doc.InteractiveReportJson(),
            ["chunks"] = doc.ChunksJson(),
        };
        Assert.Contains("feature_report", reports["feature"]);
        Assert.False(string.IsNullOrWhiteSpace(OxideDocument.EngineVersion()));
        Assert.True(OxideDocument.AbiVersion >= 1);
        foreach (var report in reports.Values)
        {
            AssertReport(report);
        }

        var docx = doc.ToDocx();
        var xlsx = doc.ToXlsx();
        var pptx = doc.ToPptx();
        var sanitized = doc.Sanitize();
        var canonicalized = doc.Canonicalize(0);
        reports["sanitize"] = sanitized.ReportJson;
        reports["canonicalize"] = canonicalized.ReportJson;

        Assert.StartsWith("PK", Encoding.ASCII.GetString(docx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(xlsx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(pptx, 0, 2));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(sanitized.Bytes, 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(canonicalized.Bytes, 0, 5));
        AssertReport(sanitized.ReportJson);
        AssertReport(canonicalized.ReportJson);
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.DocxToPdf(docx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.XlsxToPdf(xlsx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.PptxToPdf(pptx), 0, 5));
        WritePrompt02Artifact(FixturePath(), reports, sanitized, canonicalized);
    }

    [Fact]
    public void MalformedInputRaisesManagedException()
    {
        var ex = Assert.Throws<OxideException>(() => OxideDocument.Open(new byte[] { 1, 2, 3, 4 }));
        Assert.NotEqual(0, ex.Status);
    }

    [Fact]
    public void PasswordOpenRoutesThroughNativeAbiWithoutLeakingSecrets()
    {
        using (var empty = OxideDocument.Open(FixturePath(), password: ""))
        {
            Assert.True(empty.PageCount >= 1);
        }

        var bytes = File.ReadAllBytes(FixturePath());
        using (var ignored = OxideDocument.Open(bytes, password: "ignored-for-unencrypted"))
        {
            Assert.True(ignored.PageCount >= 1);
        }

        const string secret = "do-not-echo-dotnet-password";
        var ex = Assert.Throws<OxideException>(() => OxideDocument.Open(new byte[] { 1, 2, 3, 4 }, secret));
        Assert.DoesNotContain(secret, ex.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void FeatureReportRecordsPrompt02BProgressAndCancellationPosture()
    {
        var feature = OxideDocument.FeatureReportJson();
        Assert.Contains("\"progress\"", feature);
        Assert.Contains("progress_not_supported", feature);
        Assert.Contains("\"cancellation\"", feature);
        Assert.Contains("cancellation_not_supported_for_prompt02_bindings", feature);
        Assert.Contains("\"codec_isolation\"", feature);
        Assert.Contains("\"prompt07_transparency_compositing\"", feature);
        Assert.Contains("native_foundation_with_bounded_offscreen_admission_and_multi_reference_corpus", feature);
        Assert.Contains("\"memory_cap_mb\":4096", feature);
        Assert.Contains("\"Luminosity\"", feature);
        var isolation = OxideDocument.CodecIsolationReportJson(
            "FlateDecode",
            Encoding.UTF8.GetBytes("not-decoded-in-report-only"),
            "report_only");
        Assert.Contains("codec_isolation_report", isolation);
        Assert.Contains("report_only", isolation);
    }

    [Fact]
    public void RepeatedOpenReportAndDisposeStress()
    {
        for (var i = 0; i < 25; i++)
        {
            using var doc = OxideDocument.Open(FixturePath());
            Assert.True(doc.PageCount >= 1);
            Assert.Contains("\"schema_version\"", doc.SecurityReportJson());
        }
    }

    [Fact]
    public void DisposeIsIdempotent()
    {
        var doc = OxideDocument.Open(FixturePath());
        doc.Dispose();
        doc.Dispose();
        Assert.Throws<ObjectDisposedException>(() => doc.PageCount);
    }

    private static string FixturePath()
    {
        var env = Environment.GetEnvironmentVariable("OXIDE_FIXTURE_PDF");
        if (!string.IsNullOrWhiteSpace(env) && File.Exists(env))
        {
            return env;
        }

        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "crates", "engine", "tests", "fixtures", "tracemonkey.pdf");
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }

        throw new FileNotFoundException("Could not locate tracemonkey.pdf fixture.");
    }

    private static void AssertReport(string json)
    {
        Assert.Contains("\"schema_version\"", json);
    }

    private static void WritePrompt02Artifact(
        string fixture,
        IReadOnlyDictionary<string, string> reports,
        OxideBinaryResult sanitized,
        OxideBinaryResult canonicalized)
    {
        var artifactDir = Environment.GetEnvironmentVariable("OXIDE_PROMPT02_ARTIFACT_DIR");
        if (string.IsNullOrWhiteSpace(artifactDir))
        {
            return;
        }

        Directory.CreateDirectory(artifactDir);
        var payload = new
        {
            surface = "dotnet",
            fixture,
            engine_version = OxideDocument.EngineVersion(),
            abi_version = OxideDocument.AbiVersion,
            reports = reports.ToDictionary(
                kvp => kvp.Key,
                kvp => new { sha256 = Sha256(Encoding.UTF8.GetBytes(kvp.Value)), bytes = Encoding.UTF8.GetByteCount(kvp.Value) }),
            outputs = new
            {
                sanitized = new { bytes = sanitized.Bytes.Length, sha256 = Sha256(sanitized.Bytes) },
                canonicalized = new { bytes = canonicalized.Bytes.Length, sha256 = Sha256(canonicalized.Bytes) },
            },
        };
        File.WriteAllText(
            Path.Combine(artifactDir, "dotnet-smoke.json"),
            JsonSerializer.Serialize(payload, new JsonSerializerOptions { WriteIndented = true }));
    }

    private static string Sha256(byte[] bytes)
    {
        return Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
    }
}
