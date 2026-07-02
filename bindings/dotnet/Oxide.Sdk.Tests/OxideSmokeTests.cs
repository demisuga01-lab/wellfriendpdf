using System.Text;
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

        var docx = doc.ToDocx();
        var xlsx = doc.ToXlsx();
        var pptx = doc.ToPptx();

        Assert.StartsWith("PK", Encoding.ASCII.GetString(docx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(xlsx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(pptx, 0, 2));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.DocxToPdf(docx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.XlsxToPdf(xlsx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.PptxToPdf(pptx), 0, 5));
    }

    [Fact]
    public void MalformedInputRaisesManagedException()
    {
        var ex = Assert.Throws<OxideException>(() => OxideDocument.Open(new byte[] { 1, 2, 3, 4 }));
        Assert.NotEqual(0, ex.Status);
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
}
