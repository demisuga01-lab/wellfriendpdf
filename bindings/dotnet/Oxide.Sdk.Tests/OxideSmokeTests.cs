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
        Assert.Contains("engine_tile_progressive_resume_supported", feature);
        Assert.Contains("\"cancellation\"", feature);
        Assert.Contains("engine_render_cancellation_supported_binding_tokens_later", feature);
        Assert.Contains("\"codec_isolation\"", feature);
        Assert.Contains("\"prompt07_transparency_compositing\"", feature);
        Assert.Contains("native_foundation_with_prompt07b_closure", feature);
        Assert.Contains("\"prompt07b_transparency_closure\"", feature);
        Assert.Contains("\"oxide_outlier_failures\":0", feature);
        Assert.Contains("\"memory_cap_mb\":4096", feature);
        Assert.Contains("\"Luminosity\"", feature);
        Assert.Contains("\"prompt08_text_clipping_shading_patterns\"", feature);
        Assert.Contains("native_common_paths_with_bounded_unsupported_reports", feature);
        Assert.Contains("\"rendering_modes\":[4,5,6,7]", feature);
        Assert.Contains("\"prompt08b_type3_cid_tensor_closure\"", feature);
        Assert.Contains("complete_native_common_paths_with_reference_cluster_limits", feature);
        Assert.Contains("native_tensor_product_interior", feature);
        Assert.Contains("\"prompt09_annotation_ocg_progressive_cache\"", feature);
        Assert.Contains("implemented_with_bounded_unsupported_reports", feature);
        Assert.Contains("\"prompt09b_annotation_progressive_cache_validation\"", feature);
        Assert.Contains("implemented_and_proven", feature);
        Assert.Contains("\"schema_change\":\"additive_section_only\"", feature);
        Assert.Contains("\"prompt10_cjk_rtl_color_glyph_reference_harness\"", feature);
        Assert.Contains("unsupported_color_tables_are_detected_and_reported", feature);
        Assert.Contains("\"additive_feature_report_prompt10\"", feature);
        Assert.Contains("\"prompt10b_color_glyph_cjk_rtl_fidelity_closure\"", feature);
        Assert.Contains("\"implemented_with_precise_security_and_exotic_limits\"", feature);
        Assert.Contains("\"additive_feature_report_prompt10b\"", feature);
        Assert.Contains("\"prompt10c_color_glyph_hinting_cff_closure\"", feature);
        Assert.Contains("\"implemented_with_operator_level_limits\"", feature);
        Assert.Contains("\"additive_feature_report_prompt10c\"", feature);
        Assert.Contains("\"prompt10d_full_colrv1_svg_color_glyph_closure\"", feature);
        Assert.Contains("\"safe_static_subset_rendered_active_constructs_blocked\"", feature);
        Assert.Contains("\"additive_feature_report_prompt10d\"", feature);
        Assert.Contains("\"prompt10e_colrv1_gradient_clip_composite_closure\"", feature);
        Assert.Contains("\"implemented_with_exact_mode_limits\"", feature);
        Assert.Contains("\"additive_feature_report_prompt10e\"", feature);
        Assert.Contains("\"prompt10f_colrv1_porterduff_radial_closure\"", feature);
        Assert.Contains("\"DestinationAtop\"", feature);
        Assert.Contains("\"implemented_with_reference_equivalence\"", feature);
        Assert.Contains("\"additive_feature_report_prompt10f\"", feature);
        Assert.Contains("\"prompt11_renderer_fuzz_cmm_closeout\"", feature);
        Assert.Contains("\"hard_blocked_precise_no_default_native_dependency\"", feature);
        Assert.Contains("\"additive_feature_report_prompt11\"", feature);
        Assert.Contains("\"prompt11b_native_littlecms_cmm_backend_closure\"", feature);
        Assert.Contains("\"native-cmm-lcms2\"", feature);
        Assert.Contains("\"additive_feature_report_prompt11b\"", feature);
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
