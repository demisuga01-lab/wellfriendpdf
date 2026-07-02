using System.Reflection;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Oxide.Sdk;

internal static partial class NativeMethods
{
    private const string LibraryName = "oxide_capi";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(typeof(NativeMethods).Assembly, ResolveLibrary);
    }

    private static IntPtr ResolveLibrary(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (!string.Equals(libraryName, LibraryName, StringComparison.Ordinal))
        {
            return IntPtr.Zero;
        }

        var explicitPath = Environment.GetEnvironmentVariable("OXIDE_NATIVE_LIBRARY");
        if (!string.IsNullOrWhiteSpace(explicitPath) && File.Exists(explicitPath))
        {
            return NativeLibrary.Load(explicitPath);
        }

        return IntPtr.Zero;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct OxideBuffer
    {
        public IntPtr Data;
        public UIntPtr Len;
    }

    internal sealed class DocumentHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private DocumentHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            oxide_document_free(handle);
            return true;
        }
    }

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern DocumentHandle oxide_document_open_from_bytes(
        byte[] data,
        UIntPtr len,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void oxide_document_free(IntPtr document);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void oxide_string_free(IntPtr value);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void oxide_error_free(IntPtr value);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void oxide_buffer_free(OxideBuffer buffer);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_page_count(
        DocumentHandle document,
        out UIntPtr count,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_extract_text(
        DocumentHandle document,
        UIntPtr page,
        out IntPtr text,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_parse_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_extract_fields_json(
        DocumentHandle document,
        IntPtr docType,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_to_xlsx(
        DocumentHandle document,
        IntPtr layout,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_to_pptx(
        DocumentHandle document,
        int includeImages,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_to_docx(
        DocumentHandle document,
        int includeImages,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_docx_to_pdf(
        byte[] data,
        UIntPtr len,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_xlsx_to_pdf(
        byte[] data,
        UIntPtr len,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_pptx_to_pdf(
        byte[] data,
        UIntPtr len,
        out OxideBuffer buffer,
        out IntPtr errorOut);

    internal static void ThrowIfError(int status, IntPtr errorOut)
    {
        if (status == 0)
        {
            if (errorOut != IntPtr.Zero)
            {
                oxide_error_free(errorOut);
            }
            return;
        }

        var message = errorOut == IntPtr.Zero
            ? $"Oxide native call failed with status {status}."
            : Marshal.PtrToStringUTF8(errorOut) ?? $"Oxide native call failed with status {status}.";
        if (errorOut != IntPtr.Zero)
        {
            oxide_error_free(errorOut);
        }
        throw new OxideException(message, status);
    }

    internal static string TakeString(IntPtr value)
    {
        try
        {
            return Marshal.PtrToStringUTF8(value) ?? string.Empty;
        }
        finally
        {
            if (value != IntPtr.Zero)
            {
                oxide_string_free(value);
            }
        }
    }

    internal static byte[] TakeBuffer(OxideBuffer buffer)
    {
        if (buffer.Data == IntPtr.Zero || buffer.Len == UIntPtr.Zero)
        {
            return Array.Empty<byte>();
        }

        try
        {
            var len = checked((int)buffer.Len.ToUInt64());
            var managed = new byte[len];
            Marshal.Copy(buffer.Data, managed, 0, len);
            return managed;
        }
        finally
        {
            oxide_buffer_free(buffer);
        }
    }
}
