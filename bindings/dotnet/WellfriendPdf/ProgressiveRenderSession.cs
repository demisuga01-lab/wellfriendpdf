using System;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading;

namespace WellfriendPdf;

public sealed record ProgressiveAdjacentPagePrefetchExecution(
    string ReportJson,
    ProgressiveRenderSession? Session);

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

    public string StepJson(ulong maxTiles, CancellationToken cancellationToken)
    {
        ThrowIfDisposed();
        if (cancellationToken.IsCancellationRequested)
        {
            RequestCancel();
            cancellationToken.ThrowIfCancellationRequested();
        }

        using var registration = cancellationToken.Register(
            static state =>
            {
                try
                {
                    ((ProgressiveRenderSession)state!).RequestCancel();
                }
                catch (ObjectDisposedException)
                {
                }
            },
            this);
        var report = StepJson(maxTiles);
        cancellationToken.ThrowIfCancellationRequested();
        return report;
    }

    public string PauseJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_pause_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public void RequestCancel()
    {
        ThrowIfDisposed();
        var status =
            NativeMethods.wellfriendpdf_progressive_render_request_cancel(_handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public string ReviseViewportHintJson(uint x, uint y, uint width, uint height)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_revise_viewport_hint_json(
            _handle, 1, x, y, width, height, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ClearViewportHintJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_revise_viewport_hint_json(
            _handle, 0, 0, 0, 0, 0, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ReviseDirtyRegionJson(uint x, uint y, uint width, uint height)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_revise_dirty_region_json(
            _handle, 1, x, y, width, height, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ClearDirtyRegionJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_revise_dirty_region_json(
            _handle, 0, 0, 0, 0, 0, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ReviseRenderContextJson(
        string? renderContractFingerprint = null,
        string? visibilityFingerprint = null)
    {
        ThrowIfDisposed();
        var contractPtr = NativeMethods.StringToNativeOrNull(renderContractFingerprint);
        var visibilityPtr = NativeMethods.StringToNativeOrNull(visibilityFingerprint);
        try
        {
            var status = NativeMethods.wellfriendpdf_progressive_render_revise_render_context_json(
                _handle, contractPtr, visibilityPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(contractPtr);
            Marshal.FreeCoTaskMem(visibilityPtr);
        }
    }

    public string ReviseRenderContractJson(string contractJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(contractJson);
        var contractPtr = NativeMethods.StringToNativeOrNull(contractJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_progressive_render_revise_render_contract_json(
                _handle, contractPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(contractPtr);
        }
    }

    public string ReviseRenderContract(RenderContract contract)
    {
        ArgumentNullException.ThrowIfNull(contract);
        return ReviseRenderContractJson(contract.ToJson());
    }

    public string ApplyRenderInvalidationPlanJson(string planJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(planJson);
        var planPtr = NativeMethods.StringToNativeOrNull(planJson);
        try
        {
            var status =
                NativeMethods.wellfriendpdf_progressive_render_apply_render_invalidation_plan_json(
                    _handle, planPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(planPtr);
        }
    }

    public string EvaluateTilePublicationJson(string publicationJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(publicationJson);
        var publicationPtr = NativeMethods.StringToNativeOrNull(publicationJson);
        try
        {
            var status =
                NativeMethods.wellfriendpdf_progressive_render_evaluate_tile_publication_json(
                    _handle, publicationPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(publicationPtr);
        }
    }

    public string ViewerQueueJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_viewer_queue_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ExecuteViewerQueueJson(ulong maxItems = 1)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_execute_viewer_queue_json(
            _handle, (UIntPtr)maxItems, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ExecuteViewerQueueJson(RenderCancellation cancellation)
    {
        return ExecuteViewerQueueJson(1, cancellation);
    }

    public string ExecuteViewerQueueJson(ulong maxItems, RenderCancellation cancellation)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(cancellation);
        var status = NativeMethods.wellfriendpdf_progressive_render_execute_viewer_queue_json_and_cancellation(
            _handle, (UIntPtr)maxItems, cancellation.Handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ExecuteViewerQueueJson(CancellationToken cancellationToken)
    {
        return ExecuteViewerQueueJson(1, cancellationToken);
    }

    public string ExecuteViewerQueueJson(ulong maxItems, CancellationToken cancellationToken)
    {
        using var cancellation = new RenderCancellation();
        using var registration = RegisterRenderCancellation(cancellation, cancellationToken);
        var report = ExecuteViewerQueueJson(maxItems, cancellation);
        cancellationToken.ThrowIfCancellationRequested();
        return report;
    }

    public ProgressiveAdjacentPagePrefetchExecution ExecuteAdjacentPagePrefetch(
        string prefetchIdentity,
        ulong maxTiles = 1)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(prefetchIdentity);
        var prefetchPtr = NativeMethods.StringToNativeOrNull(prefetchIdentity);
        NativeMethods.ProgressiveRenderJobHandle? childHandle = null;
        try
        {
            var status =
                NativeMethods.wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json(
                    _handle, prefetchPtr, (UIntPtr)maxTiles, out childHandle, out var json, out var error);
            var report = NativeMethods.TakeJson(status, json, error);
            ProgressiveRenderSession? session = null;
            if (childHandle is not null && !childHandle.IsInvalid)
            {
                session = new ProgressiveRenderSession(childHandle);
                childHandle = null;
            }
            return new ProgressiveAdjacentPagePrefetchExecution(report, session);
        }
        finally
        {
            childHandle?.Dispose();
            Marshal.FreeCoTaskMem(prefetchPtr);
        }
    }

    public ProgressiveAdjacentPagePrefetchExecution ExecuteAdjacentPagePrefetch(
        string prefetchIdentity,
        RenderCancellation cancellation)
    {
        return ExecuteAdjacentPagePrefetch(prefetchIdentity, 1, cancellation);
    }

    public ProgressiveAdjacentPagePrefetchExecution ExecuteAdjacentPagePrefetch(
        string prefetchIdentity,
        ulong maxTiles,
        RenderCancellation cancellation)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(prefetchIdentity);
        ArgumentNullException.ThrowIfNull(cancellation);
        var prefetchPtr = NativeMethods.StringToNativeOrNull(prefetchIdentity);
        NativeMethods.ProgressiveRenderJobHandle? childHandle = null;
        try
        {
            var status =
                NativeMethods.wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json_and_cancellation(
                    _handle, prefetchPtr, (UIntPtr)maxTiles, cancellation.Handle, out childHandle,
                    out var json, out var error);
            var report = NativeMethods.TakeJson(status, json, error);
            ProgressiveRenderSession? session = null;
            if (childHandle is not null && !childHandle.IsInvalid)
            {
                session = new ProgressiveRenderSession(childHandle);
                childHandle = null;
            }
            return new ProgressiveAdjacentPagePrefetchExecution(report, session);
        }
        finally
        {
            childHandle?.Dispose();
            Marshal.FreeCoTaskMem(prefetchPtr);
        }
    }

    public ProgressiveAdjacentPagePrefetchExecution ExecuteAdjacentPagePrefetch(
        string prefetchIdentity,
        CancellationToken cancellationToken)
    {
        return ExecuteAdjacentPagePrefetch(prefetchIdentity, 1, cancellationToken);
    }

    public ProgressiveAdjacentPagePrefetchExecution ExecuteAdjacentPagePrefetch(
        string prefetchIdentity,
        ulong maxTiles,
        CancellationToken cancellationToken)
    {
        using var cancellation = new RenderCancellation();
        using var registration = RegisterRenderCancellation(cancellation, cancellationToken);
        var execution = ExecuteAdjacentPagePrefetch(prefetchIdentity, maxTiles, cancellation);
        cancellationToken.ThrowIfCancellationRequested();
        return execution;
    }

    public string ViewerCallbackDispatchJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_progressive_render_viewer_callback_dispatch_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string DispatchViewerCallbacks(Action<string> callback)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(callback);
        var report = ViewerCallbackDispatchJson();
        using var parsed = JsonDocument.Parse(report);
        if (parsed.RootElement.TryGetProperty("events", out var events)
            && events.ValueKind == JsonValueKind.Array)
        {
            foreach (var callbackEvent in events.EnumerateArray())
            {
                callback(callbackEvent.GetRawText());
            }
        }
        return report;
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

    public byte[] FinishPng(CancellationToken cancellationToken)
    {
        ThrowIfDisposed();
        if (cancellationToken.IsCancellationRequested)
        {
            RequestCancel();
            cancellationToken.ThrowIfCancellationRequested();
        }

        using var registration = cancellationToken.Register(
            static state =>
            {
                try
                {
                    ((ProgressiveRenderSession)state!).RequestCancel();
                }
                catch (ObjectDisposedException)
                {
                }
            },
            this);
        var png = FinishPng();
        cancellationToken.ThrowIfCancellationRequested();
        return png;
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

    private static CancellationTokenRegistration RegisterRenderCancellation(
        RenderCancellation cancellation,
        CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            cancellation.Cancel();
            cancellationToken.ThrowIfCancellationRequested();
        }

        return cancellationToken.Register(
            static state =>
            {
                try
                {
                    ((RenderCancellation)state!).Cancel();
                }
                catch (ObjectDisposedException)
                {
                }
            },
            cancellation);
    }
}
