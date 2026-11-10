using System.Runtime.InteropServices;

namespace WellfriendPdf;

internal static partial class NativeMethods
{
    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_search_text_json(
        DocumentHandle document,
        IntPtr query,
        IntPtr optionsJson,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_page_thumbnail_rgba(
        DocumentHandle document,
        UIntPtr page,
        out int found,
        out uint width,
        out uint height,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_decode_page_thumbnail(
        DocumentHandle document,
        UIntPtr page,
        out int found,
        out uint width,
        out uint height,
        out uint channels,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_page_thumbnail_png(
        DocumentHandle document,
        UIntPtr page,
        out int found,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_page_images_json(
        DocumentHandle document,
        UIntPtr page,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_extract_image_original(
        DocumentHandle document,
        UIntPtr page,
        UIntPtr imageIndex,
        out WellfriendBuffer buffer,
        out IntPtr infoJson,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_extract_image_original_sidecar(
        DocumentHandle document,
        UIntPtr page,
        UIntPtr imageIndex,
        UIntPtr sidecarIndex,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_render_page_image_object_png(
        DocumentHandle document,
        UIntPtr page,
        UIntPtr objectIndex,
        IntPtr objectName,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_render_page_image_object_rgba(
        DocumentHandle document,
        UIntPtr page,
        UIntPtr objectIndex,
        IntPtr objectName,
        out uint width,
        out uint height,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);
}
