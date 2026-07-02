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
