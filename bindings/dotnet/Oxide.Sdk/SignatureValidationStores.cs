using System.Runtime.InteropServices;
using System.Security.Cryptography.X509Certificates;

namespace Oxide.Sdk;

/// <summary>
/// Owned explicit trust-anchor store for signature validation. Adding a
/// certificate here is distinct from adding an intermediate: only this store
/// can grant chain trust when attached to <see cref="SignatureValidationOptions"/>.
/// </summary>
public sealed class SignatureTrustStore : IDisposable
{
    private readonly NativeMethods.SignatureTrustStoreHandle _handle;
    private bool _disposed;

    public SignatureTrustStore()
    {
        _handle = NativeMethods.oxide_signature_trust_store_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.SignatureTrustStoreHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void AddTrustAnchorDer(byte[] der)
    {
        ArgumentNullException.ThrowIfNull(der);
        ThrowIfDisposed();
        if (der.Length == 0) throw new ArgumentException("DER input must not be empty.", nameof(der));
        var status = NativeMethods.oxide_signature_trust_store_add_anchor_der(
            _handle, der, (UIntPtr)der.Length, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void AddTrustAnchor(X509Certificate2 certificate)
    {
        ArgumentNullException.ThrowIfNull(certificate);
        AddTrustAnchorDer(certificate.Export(X509ContentType.Cert));
    }

    public void AddDistrustedCertificateSha256(string fingerprint)
    {
        ArgumentNullException.ThrowIfNull(fingerprint);
        ThrowIfDisposed();
        var native = NativeMethods.StringToNativeOrNull(fingerprint);
        try
        {
            var status = NativeMethods.oxide_signature_trust_store_add_distrusted_certificate_sha256(
                _handle, native, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(native);
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
            throw new ObjectDisposedException(nameof(SignatureTrustStore));
    }
}

/// <summary>
/// Owned collection of untrusted path-building certificates. Its entries are
/// never implicitly promoted to trust anchors.
/// </summary>
public sealed class SignatureIntermediateStore : IDisposable
{
    private readonly NativeMethods.SignatureIntermediateStoreHandle _handle;
    private bool _disposed;

    public SignatureIntermediateStore()
    {
        _handle = NativeMethods.oxide_signature_intermediate_store_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.SignatureIntermediateStoreHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void AddDer(byte[] der)
    {
        ArgumentNullException.ThrowIfNull(der);
        ThrowIfDisposed();
        if (der.Length == 0) throw new ArgumentException("DER input must not be empty.", nameof(der));
        var status = NativeMethods.oxide_signature_intermediate_store_add_der(
            _handle, der, (UIntPtr)der.Length, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void Add(X509Certificate2 certificate)
    {
        ArgumentNullException.ThrowIfNull(certificate);
        AddDer(certificate.Export(X509ContentType.Cert));
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
            throw new ObjectDisposedException(nameof(SignatureIntermediateStore));
    }
}

/// <summary>
/// Owned caller-supplied or replayed OCSP/CRL evidence. Inserting bytes does
/// not mark them good; the native validation pipeline rechecks them for every
/// selected path and validation time.
/// </summary>
public sealed class SignatureEvidenceStore : IDisposable
{
    private readonly NativeMethods.SignatureEvidenceStoreHandle _handle;
    private bool _disposed;

    public SignatureEvidenceStore()
    {
        _handle = NativeMethods.oxide_signature_evidence_store_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.SignatureEvidenceStoreHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void AddOcspDer(byte[] der) => AddDer(der, NativeMethods.oxide_signature_evidence_store_add_ocsp_der);

    public void AddCrlDer(byte[] der) => AddDer(der, NativeMethods.oxide_signature_evidence_store_add_crl_der);

    public void ImportBundleJson(string bundleJson)
    {
        ArgumentNullException.ThrowIfNull(bundleJson);
        ThrowIfDisposed();
        var native = NativeMethods.StringToNativeOrNull(bundleJson);
        try
        {
            var status = NativeMethods.oxide_signature_evidence_store_set_bundle_json(
                _handle, native, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(native);
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _handle.Dispose();
        _disposed = true;
        GC.SuppressFinalize(this);
    }

    private delegate int AddDerDelegate(
        NativeMethods.SignatureEvidenceStoreHandle store,
        byte[] data,
        UIntPtr len,
        out IntPtr error);

    private void AddDer(byte[] der, AddDerDelegate add)
    {
        ArgumentNullException.ThrowIfNull(der);
        ThrowIfDisposed();
        if (der.Length == 0) throw new ArgumentException("DER input must not be empty.", nameof(der));
        var status = add(_handle, der, (UIntPtr)der.Length, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    private void ThrowIfDisposed()
    {
        if (_disposed || _handle.IsClosed || _handle.IsInvalid)
            throw new ObjectDisposedException(nameof(SignatureEvidenceStore));
    }
}

/// <summary>
/// Owned bounded AIA/OCSP/CRL transport policy. It starts offline. A policy
/// must explicitly enable HTTP/HTTPS retrieval; no insecure TLS bypass exists.
/// </summary>
public sealed class SignatureRetrievalPolicy : IDisposable
{
    private readonly NativeMethods.SignatureRetrievalPolicyHandle _handle;
    private bool _disposed;

    public SignatureRetrievalPolicy()
    {
        _handle = NativeMethods.oxide_signature_retrieval_policy_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.SignatureRetrievalPolicyHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void SetJson(string policyJson)
    {
        ArgumentNullException.ThrowIfNull(policyJson);
        ThrowIfDisposed();
        var native = NativeMethods.StringToNativeOrNull(policyJson);
        try
        {
            var status = NativeMethods.oxide_signature_retrieval_policy_set_json(
                _handle, native, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(native);
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
            throw new ObjectDisposedException(nameof(SignatureRetrievalPolicy));
    }
}

/// <summary>
/// Cooperative cancellation source for a signature-validation call. It can be
/// cancelled from another managed thread; socket waits remain bounded by the
/// configured retrieval deadlines.
/// </summary>
public sealed class SignatureValidationCancellation : IDisposable
{
    private readonly NativeMethods.SignatureValidationCancellationHandle _handle;
    private bool _disposed;

    public SignatureValidationCancellation()
    {
        _handle = NativeMethods.oxide_signature_validation_cancellation_new(out var error);
        if (_handle.IsInvalid) NativeMethods.ThrowIfError(2, error);
    }

    internal NativeMethods.SignatureValidationCancellationHandle Handle
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
        var status = NativeMethods.oxide_signature_validation_cancellation_cancel(_handle, out var error);
        NativeMethods.ThrowIfError(status, error);
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
            throw new ObjectDisposedException(nameof(SignatureValidationCancellation));
    }
}
