using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace WellfriendPdf;

internal static partial class NativeMethods
{
    internal sealed class StorySessionHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private StorySessionHandle() : base(true) { }
        protected override bool ReleaseHandle()
        {
            wellfriendpdf_story_session_free(handle);
            return true;
        }
    }

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern StorySessionHandle wellfriendpdf_story_session_open(
        byte[] data, UIntPtr len, RenderCancellationHandle cancellation, out IntPtr error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern StorySessionHandle wellfriendpdf_story_session_open_with_password(
        byte[] data, UIntPtr len, byte[] password, UIntPtr passwordLen,
        RenderCancellationHandle cancellation, out IntPtr error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_story_session_command_json(
        StorySessionHandle session, byte[] json, UIntPtr len, RenderCancellationHandle cancellation,
        out WellfriendBuffer output, out IntPtr error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_story_session_bytes(
        StorySessionHandle session, out WellfriendBuffer output, out IntPtr error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_story_session_render_page_png(
        StorySessionHandle session, UIntPtr page, uint dpi, RenderCancellationHandle cancellation,
        out WellfriendBuffer output, out IntPtr error);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    private static extern void wellfriendpdf_story_session_free(IntPtr session);
}
