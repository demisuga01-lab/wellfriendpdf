namespace Oxide.Sdk;

public static class OfficeConverters
{
    public static byte[] DocxToPdf(string path) => DocxToPdf(File.ReadAllBytes(path));

    public static byte[] XlsxToPdf(string path) => XlsxToPdf(File.ReadAllBytes(path));

    public static byte[] PptxToPdf(string path) => PptxToPdf(File.ReadAllBytes(path));

    public static byte[] DocxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.oxide_docx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public static byte[] XlsxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.oxide_xlsx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public static byte[] PptxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.oxide_pptx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }
}
