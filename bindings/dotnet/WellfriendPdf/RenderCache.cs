using System.Runtime.InteropServices;

namespace WellfriendPdf;

public sealed class RenderCache : IDisposable
{
    private readonly NativeMethods.RenderCacheHandle _handle;
    private bool _disposed;

    public RenderCache()
    {
        _handle = NativeMethods.wellfriendpdf_render_cache_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.RenderCacheHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void Clear()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_render_cache_clear(_handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public string ApplyRenderInvalidationPlanJson(string planJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(planJson);
        var planPtr = NativeMethods.StringToNativeOrNull(planJson);
        try
        {
            var status =
                NativeMethods.wellfriendpdf_render_cache_apply_render_invalidation_plan_json(
                    _handle, planPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(planPtr);
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _handle.Dispose();
        _disposed = true;
        GC.SuppressFinalize(this);
    }

    private void ThrowIfDisposed()
    {
        if (_disposed || _handle.IsClosed || _handle.IsInvalid)
        {
            throw new ObjectDisposedException(nameof(RenderCache));
        }
    }
}
