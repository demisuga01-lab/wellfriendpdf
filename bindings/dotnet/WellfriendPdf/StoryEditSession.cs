using System.Text;
using System.Text.Json;
using System.Security.Cryptography;

namespace WellfriendPdf;

/// <summary>
/// Retained PDF-native editing with exact preview approval and byte-level undo.
/// All access and disposal is serialized. No file is written by this class.
/// </summary>
public sealed class StoryEditSession : IDisposable
{
    private const int MaxCommandBytes = 32 * 1024 * 1024;
    private static readonly UTF8Encoding Utf8 = new(false, true);
    private readonly object _gate = new();
    private readonly NativeMethods.StorySessionHandle _handle;
    private bool _disposed;

    private StoryEditSession(NativeMethods.StorySessionHandle handle) => _handle = handle;

    public static StoryEditSession Open(byte[] bytes, CancellationToken cancellationToken = default)
        => OpenCore(bytes, null, cancellationToken);

    /// <summary>
    /// Opens a Standard-handler encrypted PDF with its permissions/owner
    /// password and creates an unencrypted,
    /// revision-bound editing working copy. The UTF-8 password is wiped after
    /// the synchronous native open and is never retained by the session.
    /// </summary>
    public static StoryEditSession Open(
        byte[] bytes,
        string password,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(password);
        var passwordBytes = Utf8.GetBytes(password);
        try
        {
            return OpenCore(bytes, passwordBytes, cancellationToken);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(passwordBytes);
        }
    }

    /// <summary>
    /// Opens a Standard-handler encrypted PDF with the exact permissions/owner
    /// password bytes.
    /// The caller retains ownership of <paramref name="password"/>; native code
    /// borrows it only for this synchronous call and the session stores no copy.
    /// </summary>
    public static StoryEditSession OpenWithPasswordBytes(
        byte[] bytes,
        byte[] password,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(password);
        return OpenCore(bytes, password, cancellationToken);
    }

    private static StoryEditSession OpenCore(
        byte[] bytes,
        byte[]? password,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        if (bytes.Length == 0 || bytes.Length > 256 * 1024 * 1024)
            throw new ArgumentOutOfRangeException(nameof(bytes), "Input must be 1..=256 MiB.");
        return WithCancellation(cancellationToken, cancellation =>
        {
            IntPtr error;
            var handle = password is null
                ? NativeMethods.wellfriendpdf_story_session_open(
                    bytes, (UIntPtr)bytes.Length, cancellation.Handle, out error)
                : NativeMethods.wellfriendpdf_story_session_open_with_password(
                    bytes, (UIntPtr)bytes.Length, password, (UIntPtr)password.Length,
                    cancellation.Handle, out error);
            if (handle.IsInvalid)
            {
                handle.Dispose();
                NativeMethods.ThrowIfError(2, error);
            }
            else NativeMethods.ThrowIfError(0, error);
            return new StoryEditSession(handle);
        });
    }

    /// <summary>Executes the shared v1 JSON command envelope. Not a file/network API.</summary>
    public string CommandJson(string commandJson, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(commandJson);
        if (Utf8.GetByteCount(commandJson) > MaxCommandBytes)
            throw new ArgumentOutOfRangeException(nameof(commandJson), "Command exceeds 32 MiB.");
        var bytes = Utf8.GetBytes(commandJson);
        lock (_gate)
        {
            ThrowIfDisposed();
            return WithCancellation(cancellationToken, cancellation =>
            {
                var status = NativeMethods.wellfriendpdf_story_session_command_json(
                    _handle, bytes, (UIntPtr)bytes.Length, cancellation.Handle, out var output, out var error);
                NativeMethods.ThrowIfError(status, error);
                return Utf8.GetString(NativeMethods.TakeBuffer(output));
            });
        }
    }

    public string StatusJson() => CommandJson("{\"op\":\"status\"}");
    public string SavedStoriesJson() => CommandJson("{\"op\":\"saved_stories\"}");
    public string PagesJson() => CommandJson("{\"op\":\"pages\"}");
    public string PreviewJson(string requestJson, CancellationToken cancellationToken = default) =>
        CommandJson(JsonSerializer.Serialize(new { op = "preview", request = Parse(requestJson) }), cancellationToken);
    public string CheckpointJson(string requestJson, string receiptJson, CancellationToken cancellationToken = default) =>
        CommandJson(JsonSerializer.Serialize(new { op = "checkpoint", request = Parse(requestJson), receipt = Parse(receiptJson) }), cancellationToken);
    public bool Undo(CancellationToken cancellationToken = default) =>
        JsonSerializer.Deserialize<bool>(CommandJson("{\"op\":\"undo\"}", cancellationToken));
    public bool Redo(CancellationToken cancellationToken = default) =>
        JsonSerializer.Deserialize<bool>(CommandJson("{\"op\":\"redo\"}", cancellationToken));

    public byte[] Bytes()
    {
        lock (_gate)
        {
            ThrowIfDisposed();
            var status = NativeMethods.wellfriendpdf_story_session_bytes(_handle, out var output, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(output);
        }
    }

    /// <summary>Explicit rendering only; 1-based page, 1..300 DPI, at most 16M pixels.</summary>
    public byte[] RenderPagePng(int page, uint dpi = 96, CancellationToken cancellationToken = default)
    {
        if (page < 1) throw new ArgumentOutOfRangeException(nameof(page));
        lock (_gate)
        {
            ThrowIfDisposed();
            return WithCancellation(cancellationToken, cancellation =>
            {
                var status = NativeMethods.wellfriendpdf_story_session_render_page_png(
                    _handle, (UIntPtr)page, dpi, cancellation.Handle, out var output, out var error);
                NativeMethods.ThrowIfError(status, error);
                return NativeMethods.TakeBuffer(output);
            });
        }
    }

    public void Dispose()
    {
        lock (_gate)
        {
            if (_disposed) return;
            _handle.Dispose();
            _disposed = true;
        }
        GC.SuppressFinalize(this);
    }

    private static JsonElement Parse(string json)
    {
        ArgumentNullException.ThrowIfNull(json);
        if (Utf8.GetByteCount(json) > MaxCommandBytes) throw new ArgumentOutOfRangeException(nameof(json));
        using var document = JsonDocument.Parse(json);
        if (document.RootElement.ValueKind != JsonValueKind.Object)
            throw new ArgumentException("Expected a JSON object.", nameof(json));
        return document.RootElement.Clone();
    }

    private static T WithCancellation<T>(CancellationToken token, Func<RenderCancellation, T> action)
    {
        token.ThrowIfCancellationRequested();
        using var source = new RenderCancellation();
        using var registration = token.Register(static state => ((RenderCancellation)state!).Cancel(), source);
        // Dispose the registration (waiting for a callback) before the source.
        // Never throw cancellation after a successful native checkpoint.
        return action(source);
    }

    private void ThrowIfDisposed()
    {
        if (_disposed || _handle.IsClosed || _handle.IsInvalid)
            throw new ObjectDisposedException(nameof(StoryEditSession));
    }
}
