namespace WellfriendPdf;

/// <summary>
/// Cooperative cancellation source for contract rendering.
/// </summary>
public sealed class RenderCancellation : IDisposable
{
    private readonly NativeMethods.RenderCancellationHandle _handle;
    private bool _disposed;

    public RenderCancellation()
    {
        _handle = NativeMethods.wellfriendpdf_render_cancellation_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.RenderCancellationHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void Cancel()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_render_cancellation_cancel(_handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public bool IsCancelled
    {
        get
        {
            ThrowIfDisposed();
            var status = NativeMethods.wellfriendpdf_render_cancellation_is_cancelled(
                _handle, out var cancelled, out var error);
            NativeMethods.ThrowIfError(status, error);
            return cancelled != 0;
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
            throw new ObjectDisposedException(nameof(RenderCancellation));
        }
    }
}
