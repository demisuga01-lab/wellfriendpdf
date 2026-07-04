using System.Runtime.InteropServices;

namespace Oxide.Sdk;

public sealed class OxideDocument : IDisposable
{
    private readonly NativeMethods.DocumentHandle _handle;
    private bool _disposed;

    private OxideDocument(NativeMethods.DocumentHandle handle)
    {
        _handle = handle;
    }

    public int PageCount
    {
        get
        {
            ThrowIfDisposed();
            var status = NativeMethods.oxide_document_page_count(_handle, out var count, out var error);
            NativeMethods.ThrowIfError(status, error);
            return checked((int)count.ToUInt64());
        }
    }

    public IReadOnlyList<Page> Pages => Enumerable.Range(1, PageCount).Select(n => new Page(this, n)).ToArray();

    public static OxideDocument Open(string path)
    {
        ArgumentNullException.ThrowIfNull(path);
        return Open(File.ReadAllBytes(path));
    }

    public static OxideDocument Open(byte[] bytes)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        var handle = NativeMethods.oxide_document_open_from_bytes(bytes, (UIntPtr)bytes.Length, out var error);
        if (handle.IsInvalid)
        {
            NativeMethods.ThrowIfError(2, error);
        }
        return new OxideDocument(handle);
    }

    public static string FeatureReportJson()
    {
        var status = NativeMethods.oxide_feature_report_json(out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public static string EngineVersion()
    {
        return NativeMethods.TakeString(NativeMethods.oxide_version());
    }

    public static uint AbiVersion => NativeMethods.oxide_abi_version();

    public Page GetPage(int pageNumber)
    {
        if (pageNumber < 1 || pageNumber > PageCount)
        {
            throw new ArgumentOutOfRangeException(nameof(pageNumber), "Page numbers are 1-based.");
        }
        return new Page(this, pageNumber);
    }

    public string ExtractText(int pageNumber)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_extract_text(_handle, (UIntPtr)pageNumber, out var text, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(text);
    }

    public string ParseJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_parse_json(_handle, out var json, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(json);
    }

    public string ExtractFieldsJson(string? documentType = null)
    {
        ThrowIfDisposed();
        var docTypePtr = documentType is null ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(documentType);
        try
        {
            var status = NativeMethods.oxide_document_extract_fields_json(_handle, docTypePtr, out var json, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeString(json);
        }
        finally
        {
            if (docTypePtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(docTypePtr);
            }
        }
    }

    public string SecurityReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_security_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ParserReportJson(string mode = "repair")
    {
        ThrowIfDisposed();
        return ReportWithString(mode, NativeMethods.oxide_document_parser_report_json);
    }

    public string ColorReportJson(string profile = "generic")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.oxide_document_color_report_json);
    }

    public string ValidateJson(string profile = "all")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.oxide_document_validate_json);
    }

    public string FormsReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_forms_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string AnnotationsReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_annotations_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string PagesReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_pages_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string InteractiveReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_interactive_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ChunksJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_chunks_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public OxideBinaryResult Sanitize(string policy = "balanced")
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(policy);
        try
        {
            var status = NativeMethods.oxide_document_sanitize_json(_handle, policyPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (policyPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(policyPtr);
            }
        }
    }

    public OxideBinaryResult Canonicalize(long? dateEpoch = null)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_canonicalize_json(
            _handle,
            dateEpoch.GetValueOrDefault(),
            dateEpoch.HasValue ? 1 : 0,
            out var buffer,
            out var json,
            out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public OxideBinaryResult RedactTerms(IEnumerable<string> terms, bool strict = false)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(terms);
        var normalized = terms.Where(t => !string.IsNullOrWhiteSpace(t)).ToArray();
        if (normalized.Length == 0)
        {
            throw new ArgumentException("At least one non-empty redaction term is required.", nameof(terms));
        }

        var nativeStrings = new IntPtr[normalized.Length];
        var termsPtr = Marshal.AllocHGlobal(IntPtr.Size * normalized.Length);
        try
        {
            for (var i = 0; i < normalized.Length; i++)
            {
                nativeStrings[i] = Marshal.StringToCoTaskMemUTF8(normalized[i]);
                Marshal.WriteIntPtr(termsPtr, i * IntPtr.Size, nativeStrings[i]);
            }

            var status = NativeMethods.oxide_document_redact_terms_json(
                _handle,
                termsPtr,
                (UIntPtr)normalized.Length,
                strict ? 1 : 0,
                out var buffer,
                out var json,
                out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            foreach (var ptr in nativeStrings)
            {
                if (ptr != IntPtr.Zero)
                {
                    Marshal.FreeCoTaskMem(ptr);
                }
            }
            Marshal.FreeHGlobal(termsPtr);
        }
    }

    public byte[] ToXlsx(string layout = "pages")
    {
        ThrowIfDisposed();
        var layoutPtr = Marshal.StringToCoTaskMemUTF8(layout);
        try
        {
            var status = NativeMethods.oxide_document_to_xlsx(_handle, layoutPtr, out var buffer, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(buffer);
        }
        finally
        {
            Marshal.FreeCoTaskMem(layoutPtr);
        }
    }

    public byte[] ToPptx(bool includeImages = true)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_to_pptx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public byte[] ToDocx(bool includeImages = true)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_to_docx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _handle.Dispose();
        _disposed = true;
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
    }

    private delegate int StringReportCall(
        NativeMethods.DocumentHandle document,
        IntPtr arg,
        out IntPtr json,
        out IntPtr error);

    private string ReportWithString(string? arg, StringReportCall call)
    {
        var argPtr = NativeMethods.StringToNativeOrNull(arg);
        try
        {
            var status = call(_handle, argPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (argPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(argPtr);
            }
        }
    }
}

public sealed class Page
{
    private readonly OxideDocument _document;

    internal Page(OxideDocument document, int pageNumber)
    {
        _document = document;
        Number = pageNumber;
    }

    public int Number { get; }

    public string Text => _document.ExtractText(Number);
}
