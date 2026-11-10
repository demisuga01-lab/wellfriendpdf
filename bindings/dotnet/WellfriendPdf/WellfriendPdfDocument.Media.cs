using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace WellfriendPdf;

/// <summary>Decoded 8-bit samples (<see cref="Channels"/> per pixel, row-major).</summary>
public sealed record WellfriendRawImage(uint Width, uint Height, uint Channels, byte[] Pixels);

/// <summary>
/// A page image as <c>pdfimages -all</c> saves it: original JPEG (<c>jpg</c>), JPEG 2000
/// (<c>jp2</c>), JBIG2 (<c>jb2e</c>) or CCITT (<c>ccitt</c>) bytes, otherwise a PNG.
/// <see cref="Sidecars"/> maps a sidecar extension (<c>jb2g</c> JBIG2 globals,
/// <c>params</c> CCITT fax2tiff options) to its bytes.
/// </summary>
public sealed record WellfriendOriginalImage(
    byte[] Bytes,
    string Extension,
    string Filter,
    bool Passthrough,
    IReadOnlyDictionary<string, byte[]> Sidecars,
    string InfoJson);

/// <summary>
/// Text-search options. Unset (null) values keep the engine defaults: case-sensitive,
/// NFKC Unicode normalisation on, accents and partial words matched, all pages.
/// </summary>
public sealed record TextSearchOptions
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    [JsonPropertyName("pages")]
    public IReadOnlyList<int>? Pages { get; init; }

    [JsonPropertyName("case_sensitive")]
    public bool? CaseSensitive { get; init; }

    [JsonPropertyName("normalize_unicode")]
    public bool? NormalizeUnicode { get; init; }

    [JsonPropertyName("ignore_accents")]
    public bool? IgnoreAccents { get; init; }

    [JsonPropertyName("whole_word")]
    public bool? WholeWord { get; init; }

    [JsonPropertyName("include_hidden")]
    public bool? IncludeHidden { get; init; }

    [JsonPropertyName("max_matches")]
    public int? MaxMatches { get; init; }

    [JsonPropertyName("normalize_ligatures")]
    public bool? NormalizeLigatures { get; init; }

    [JsonPropertyName("ignore_hyphenation")]
    public bool? IgnoreHyphenation { get; init; }

    [JsonPropertyName("collapse_whitespace")]
    public bool? CollapseWhitespace { get; init; }

    public string ToJson() => JsonSerializer.Serialize(this, JsonOptions);
}

public sealed partial class WellfriendDocument
{
    /// <summary>
    /// Text search returning a <c>binding_media.text_search.v1</c> JSON report
    /// (<c>match_count</c>, <c>matches</c> with page, text and highlight quads).
    /// </summary>
    public string SearchTextJson(string query, string? optionsJson = null)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrEmpty(query);
        var queryPtr = NativeMethods.StringToNativeOrNull(query);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_search_text_json(
                _handle, queryPtr, optionsPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (queryPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(queryPtr);
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public string SearchTextJson(string query, TextSearchOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        return SearchTextJson(query, options.ToJson());
    }

    /// <summary>The page's embedded thumbnail as straight-alpha RGBA8, or null when it has none.</summary>
    public WellfriendRawImage? PageThumbnailRgba(int pageNumber)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var status = NativeMethods.wellfriendpdf_document_page_thumbnail_rgba(
            _handle, (UIntPtr)pageNumber, out var found, out var width, out var height,
            out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        var pixels = NativeMethods.TakeBuffer(buffer);
        return found != 0 ? new WellfriendRawImage(width, height, 4, pixels) : null;
    }

    /// <summary>The page's embedded thumbnail with the decoder's channel layout, or null.</summary>
    public WellfriendRawImage? DecodePageThumbnail(int pageNumber)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var status = NativeMethods.wellfriendpdf_document_decode_page_thumbnail(
            _handle, (UIntPtr)pageNumber, out var found, out var width, out var height,
            out var channels, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        var pixels = NativeMethods.TakeBuffer(buffer);
        return found != 0 ? new WellfriendRawImage(width, height, channels, pixels) : null;
    }

    /// <summary>The page's embedded thumbnail as RGBA PNG bytes, or null when it has none.</summary>
    public byte[]? PageThumbnailPng(int pageNumber)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var status = NativeMethods.wellfriendpdf_document_page_thumbnail_png(
            _handle, (UIntPtr)pageNumber, out var found, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        var png = NativeMethods.TakeBuffer(buffer);
        return found != 0 ? png : null;
    }

    /// <summary>JSON array describing the page's images; each <c>index</c> selects an image for <see cref="ExtractImageOriginal"/>.</summary>
    public string PageImagesJson(int pageNumber)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var status = NativeMethods.wellfriendpdf_document_page_images_json(
            _handle, (UIntPtr)pageNumber, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public WellfriendOriginalImage ExtractImageOriginal(int pageNumber, int imageIndex)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        ArgumentOutOfRangeException.ThrowIfNegative(imageIndex);
        var status = NativeMethods.wellfriendpdf_document_extract_image_original(
            _handle, (UIntPtr)pageNumber, (UIntPtr)imageIndex, out var buffer, out var infoJson,
            out var error);
        NativeMethods.ThrowIfError(status, error);
        var bytes = NativeMethods.TakeBuffer(buffer);
        var info = NativeMethods.TakeString(infoJson);
        using var document = JsonDocument.Parse(info);
        var root = document.RootElement;
        var sidecars = new Dictionary<string, byte[]>(StringComparer.Ordinal);
        var sidecarIndex = 0;
        foreach (var sidecar in root.GetProperty("sidecars").EnumerateArray())
        {
            var sidecarStatus = NativeMethods.wellfriendpdf_document_extract_image_original_sidecar(
                _handle, (UIntPtr)pageNumber, (UIntPtr)imageIndex, (UIntPtr)sidecarIndex,
                out var sidecarBuffer, out var sidecarError);
            NativeMethods.ThrowIfError(sidecarStatus, sidecarError);
            sidecars[sidecar.GetProperty("extension").GetString() ?? string.Empty] =
                NativeMethods.TakeBuffer(sidecarBuffer);
            sidecarIndex++;
        }
        return new WellfriendOriginalImage(
            bytes,
            root.GetProperty("extension").GetString() ?? string.Empty,
            root.GetProperty("filter").GetString() ?? string.Empty,
            root.GetProperty("passthrough").GetBoolean(),
            sidecars,
            info);
    }

    /// <summary>Render the n-th top-level image object of a page on its own as RGBA PNG.</summary>
    public byte[] RenderPageImageObjectPng(int pageNumber, int objectIndex)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(objectIndex);
        return RenderImageObjectPng(pageNumber, objectIndex, null);
    }

    /// <summary>Render the first top-level occurrence of a named image XObject as RGBA PNG.</summary>
    public byte[] RenderPageImageObjectPng(int pageNumber, string objectName)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(objectName);
        return RenderImageObjectPng(pageNumber, 0, objectName);
    }

    public WellfriendRawImage RenderPageImageObjectRgba(int pageNumber, int objectIndex)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(objectIndex);
        return RenderImageObjectRgba(pageNumber, objectIndex, null);
    }

    public WellfriendRawImage RenderPageImageObjectRgba(int pageNumber, string objectName)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(objectName);
        return RenderImageObjectRgba(pageNumber, 0, objectName);
    }

    private byte[] RenderImageObjectPng(int pageNumber, int objectIndex, string? objectName)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var namePtr = NativeMethods.StringToNativeOrNull(objectName);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_render_page_image_object_png(
                _handle, (UIntPtr)pageNumber, (UIntPtr)objectIndex, namePtr, out var buffer,
                out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(buffer);
        }
        finally
        {
            if (namePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(namePtr);
        }
    }

    private WellfriendRawImage RenderImageObjectRgba(int pageNumber, int objectIndex, string? objectName)
    {
        ThrowIfDisposed();
        RequirePage(pageNumber);
        var namePtr = NativeMethods.StringToNativeOrNull(objectName);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_render_page_image_object_rgba(
                _handle, (UIntPtr)pageNumber, (UIntPtr)objectIndex, namePtr, out var width,
                out var height, out var buffer, out var error);
            NativeMethods.ThrowIfError(status, error);
            return new WellfriendRawImage(width, height, 4, NativeMethods.TakeBuffer(buffer));
        }
        finally
        {
            if (namePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(namePtr);
        }
    }

    private static void RequirePage(int pageNumber)
    {
        if (pageNumber < 1) throw new ArgumentOutOfRangeException(nameof(pageNumber));
    }
}
