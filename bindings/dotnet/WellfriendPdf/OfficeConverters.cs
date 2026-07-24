namespace WellfriendPdf;

public static class OfficeConverters
{
    public static byte[] DocxToPdf(string path) => DocxToPdf(File.ReadAllBytes(path));

    public static byte[] XlsxToPdf(string path) => XlsxToPdf(File.ReadAllBytes(path));

    public static byte[] PptxToPdf(string path) => PptxToPdf(File.ReadAllBytes(path));

    public static string Prompt22InspectJson(string path, string format) =>
        Prompt22InspectJson(File.ReadAllBytes(path), format);

    public static WellfriendBinaryResult Prompt22ToPdf(string path, string format) =>
        Prompt22ToPdf(File.ReadAllBytes(path), format);

    public static byte[] DocxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.wellfriendpdf_docx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public static byte[] XlsxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.wellfriendpdf_xlsx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public static byte[] PptxToPdf(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var status = NativeMethods.wellfriendpdf_pptx_to_pdf(bytes, (UIntPtr)bytes.Length, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public static string Prompt22InspectJson(byte[] bytes, string format)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        ArgumentException.ThrowIfNullOrWhiteSpace(format);
        var formatPtr = NativeMethods.StringToNativeOrNull(format);
        try
        {
            var status = NativeMethods.wellfriendpdf_prompt22_office_inspect_json(
                bytes, (UIntPtr)bytes.Length, formatPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (formatPtr != IntPtr.Zero)
            {
                System.Runtime.InteropServices.Marshal.FreeCoTaskMem(formatPtr);
            }
        }
    }

    public static WellfriendBinaryResult Prompt22ToPdf(byte[] bytes, string format)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        ArgumentException.ThrowIfNullOrWhiteSpace(format);
        var formatPtr = NativeMethods.StringToNativeOrNull(format);
        try
        {
            var status = NativeMethods.wellfriendpdf_prompt22_office_to_pdf(
                bytes, (UIntPtr)bytes.Length, formatPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (formatPtr != IntPtr.Zero)
            {
                System.Runtime.InteropServices.Marshal.FreeCoTaskMem(formatPtr);
            }
        }
    }
}
