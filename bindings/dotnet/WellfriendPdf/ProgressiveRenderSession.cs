using System.Runtime.InteropServices;

namespace WellfriendPdf;

public sealed class ProgressiveRenderSession : IDisposable
{
    private readonly NativeMethods.ProgressiveRenderJobHandle _handle;
    private bool _disposed;

    internal ProgressiveRenderSession(NativeMethods.ProgressiveRenderJobHandle handle)
    {
        _handle = handle;
    }

    public string StepJson(ulong maxTiles = 1)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_step_json(
            _handle, (UIntPtr)maxTiles, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string PauseJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_pause_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public void ResumeJson(string tokenJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(tokenJson);
        var tokenPtr = NativeMethods.StringToNativeOrNull(tokenJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_progressive_render_resume_json(
                _handle, tokenPtr, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(tokenPtr);
        }
    }

    public void Cancel()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_cancel(_handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public byte[] FinishPng()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_finish_png(
            _handle, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public void Dispose()
    {
        if (_disposed) return;
        _handle.Dispose();
        _disposed = true;
    }

    private void ThrowIfDisposed()
    {
        if (_disposed || _handle.IsInvalid)
        {
            throw new ObjectDisposedException(nameof(ProgressiveRenderSession));
        }
    }
}
