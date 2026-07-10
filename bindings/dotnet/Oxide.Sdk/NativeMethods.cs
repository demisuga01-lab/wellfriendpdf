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

        foreach (var candidate in NativeLibraryCandidates(assembly))
        {
            if (File.Exists(candidate))
            {
                return NativeLibrary.Load(candidate);
            }
        }

        return IntPtr.Zero;
    }

    private static IEnumerable<string> NativeLibraryCandidates(Assembly assembly)
    {
        var mapped = MapNativeLibraryName();
        var rid = RuntimeIdentifier();
        var bases = new[]
        {
            AppContext.BaseDirectory,
            Path.GetDirectoryName(assembly.Location) ?? AppContext.BaseDirectory,
            Environment.CurrentDirectory,
            Path.Combine(Environment.CurrentDirectory, "target", "debug"),
            Path.Combine(Environment.CurrentDirectory, "target", "release"),
        };

        foreach (var root in bases.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            yield return Path.Combine(root, mapped);
            yield return Path.Combine(root, "runtimes", rid, "native", mapped);
        }
    }

    private static string RuntimeIdentifier()
    {
        var os = RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
            ? "win"
            : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
                ? "osx"
                : "linux";
        var arch = RuntimeInformation.ProcessArchitecture switch
        {
            Architecture.X64 => "x64",
            Architecture.Arm64 => "arm64",
            Architecture.X86 => "x86",
            Architecture.Arm => "arm",
            _ => RuntimeInformation.ProcessArchitecture.ToString().ToLowerInvariant(),
        };
        return $"{os}-{arch}";
    }

    private static string MapNativeLibraryName()
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return $"{LibraryName}.dll";
        }
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            return $"lib{LibraryName}.dylib";
        }
        return $"lib{LibraryName}.so";
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
    internal static extern DocumentHandle oxide_document_open_from_bytes_with_password(
        byte[] data,
        UIntPtr len,
        byte[] password,
        UIntPtr passwordLen,
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

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_security_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_parser_report_json(
        DocumentHandle document,
        IntPtr mode,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_color_report_json(
        DocumentHandle document,
        IntPtr profile,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_validate_json(
        DocumentHandle document,
        IntPtr profile,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_forms_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_extract_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_script_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_security_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_runtime_report_json(
        DocumentHandle document,
        IntPtr scriptPolicy,
        int executeEvents,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_annotations_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_pages_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_interactive_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_chunks_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_advanced_chunks_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_semantic_bundle_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_semantic_search_json(
        DocumentHandle document,
        IntPtr query,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_render_json(
        DocumentHandle document,
        IntPtr scriptPolicy,
        int executeEvents,
        uint dpi,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_flatten_json(
        DocumentHandle document,
        IntPtr mode,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_xfa_sanitize_json(
        DocumentHandle document,
        IntPtr mode,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_sanitize_json(
        DocumentHandle document,
        IntPtr policy,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_canonicalize_json(
        DocumentHandle document,
        long dateEpoch,
        int hasDateEpoch,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_document_redact_terms_json(
        DocumentHandle document,
        IntPtr terms,
        UIntPtr termsLen,
        int strict,
        out OxideBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_feature_report_json(
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int oxide_codec_isolation_report_json(
        IntPtr filter,
        byte[] data,
        UIntPtr len,
        IntPtr policy,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr oxide_version();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint oxide_abi_version();

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

    internal static string TakeJson(int status, IntPtr json, IntPtr error)
    {
        ThrowIfError(status, error);
        return TakeString(json);
    }

    internal static OxideBinaryResult TakeOutput(int status, OxideBuffer buffer, IntPtr json, IntPtr error)
    {
        ThrowIfError(status, error);
        return new OxideBinaryResult(TakeBuffer(buffer), TakeString(json));
    }

    internal static IntPtr StringToNativeOrNull(string? value)
    {
        return string.IsNullOrEmpty(value) ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(value);
    }
}
