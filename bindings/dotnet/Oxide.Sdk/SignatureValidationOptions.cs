using System.Runtime.InteropServices;
using System.Security.Cryptography.X509Certificates;

namespace Oxide.Sdk;

/// <summary>
/// Owned Prompt 24 signature-validation configuration.
/// Trust anchors, intermediates, and revocation evidence are copied into a
/// native SafeHandle. Network retrieval remains disabled unless an explicit
/// bounded retrieval-policy JSON object enables it.
/// </summary>
public sealed class SignatureValidationOptions : IDisposable
{
    private readonly NativeMethods.SignatureValidationOptionsHandle _handle;
    private bool _disposed;

    public SignatureValidationOptions()
    {
        _handle = NativeMethods.oxide_signature_validation_options_new(out var error);
        if (_handle.IsInvalid)
        {
            NativeMethods.ThrowIfError(2, error);
        }
    }

    internal NativeMethods.SignatureValidationOptionsHandle Handle
    {
        get
        {
            ThrowIfDisposed();
            return _handle;
        }
    }

    public void AddTrustAnchorDer(byte[] der) => AddDer(
        der,
        NativeMethods.oxide_signature_validation_options_add_trust_anchor_der);

    /// <summary>
    /// Copies explicit anchors and the deny overlay from an owned trust store.
    /// The store can be disposed after this call because the native options
    /// retain their own canonical certificate bytes.
    /// </summary>
    public void ApplyTrustStore(SignatureTrustStore trustStore)
    {
        ArgumentNullException.ThrowIfNull(trustStore);
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_apply_trust_store(
            _handle, trustStore.Handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    /// <summary>Adds an explicit trust anchor from an X509Certificate2.</summary>
    public void AddTrustAnchor(X509Certificate2 certificate)
    {
        ArgumentNullException.ThrowIfNull(certificate);
        AddTrustAnchorDer(certificate.Export(X509ContentType.Cert));
    }

    public void AddIntermediateDer(byte[] der) => AddDer(
        der,
        NativeMethods.oxide_signature_validation_options_add_intermediate_der);

    /// <summary>Copies untrusted path-building candidates from an owned store.</summary>
    public void ApplyIntermediateStore(SignatureIntermediateStore intermediateStore)
    {
        ArgumentNullException.ThrowIfNull(intermediateStore);
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_apply_intermediate_store(
            _handle, intermediateStore.Handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    /// <summary>Adds an untrusted path-building intermediate from X509Certificate2.</summary>
    public void AddIntermediate(X509Certificate2 certificate)
    {
        ArgumentNullException.ThrowIfNull(certificate);
        AddIntermediateDer(certificate.Export(X509ContentType.Cert));
    }

    /// <summary>
    /// Rejects any candidate path containing this certificate SHA-256
    /// fingerprint. Separators are accepted; malformed fingerprints are
    /// rejected by the native policy parser.
    /// </summary>
    public void AddDistrustedCertificateSha256(string fingerprint)
    {
        ArgumentNullException.ThrowIfNull(fingerprint);
        ThrowIfDisposed();
        var native = NativeMethods.StringToNativeOrNull(fingerprint);
        try
        {
            var status = NativeMethods.oxide_signature_validation_options_add_distrusted_certificate_sha256(
                _handle, native, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(native);
        }
    }

    public void AddOcspDer(byte[] der) => AddDer(
        der,
        NativeMethods.oxide_signature_validation_options_add_ocsp_der);

    public void AddCrlDer(byte[] der) => AddDer(
        der,
        NativeMethods.oxide_signature_validation_options_add_crl_der);

    /// <summary>
    /// Copies supplied/replayed evidence. The call does not make any evidence
    /// trusted or fresh; the native engine reevaluates it at validation time.
    /// </summary>
    public void ApplyEvidenceStore(SignatureEvidenceStore evidenceStore)
    {
        ArgumentNullException.ThrowIfNull(evidenceStore);
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_apply_evidence_store(
            _handle, evidenceStore.Handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void SetValidationTimeUnix(ulong validationTimeUnix)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_set_validation_time_unix(
            _handle, validationTimeUnix, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void UseSystemValidationTime()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_clear_validation_time(
            _handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void SetRevocationMode(SignatureRevocationMode mode)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_set_revocation_mode(
            _handle, (int)mode, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    /// <summary>
    /// Sets the native bounded retrieval policy. The JSON follows the Rust
    /// <c>RetrievalPolicy</c> schema and must explicitly set <c>enabled</c> to
    /// allow network access.
    /// </summary>
    public void SetRetrievalPolicyJson(string policyJson)
    {
        ArgumentNullException.ThrowIfNull(policyJson);
        ThrowIfDisposed();
        var policy = NativeMethods.StringToNativeOrNull(policyJson);
        try
        {
            var status = NativeMethods.oxide_signature_validation_options_set_retrieval_policy_json(
                _handle, policy, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(policy);
        }
    }

    /// <summary>
    /// Copies a bounded transport policy from an owned handle. The caller must
    /// explicitly configure <c>enabled</c> in the policy before online AIA,
    /// OCSP, or CRL retrieval can occur.
    /// </summary>
    public void ApplyRetrievalPolicy(SignatureRetrievalPolicy policy)
    {
        ArgumentNullException.ThrowIfNull(policy);
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_apply_retrieval_policy(
            _handle, policy.Handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    /// <summary>
    /// Attaches a cooperative cancellation source. The source may be disposed
    /// after attachment because the native options retain a shared token clone.
    /// </summary>
    public void SetCancellation(SignatureValidationCancellation cancellation)
    {
        ArgumentNullException.ThrowIfNull(cancellation);
        ThrowIfDisposed();
        var status = NativeMethods.oxide_signature_validation_options_set_cancellation(
            _handle, cancellation.Handle, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    /// <summary>
    /// Sets the native <c>SignatureAlgorithmPolicy</c> JSON schema. Recognized
    /// legacy algorithms remain unavailable unless this explicit policy permits
    /// them.
    /// </summary>
    public void SetAlgorithmPolicyJson(string policyJson)
    {
        ArgumentNullException.ThrowIfNull(policyJson);
        ThrowIfDisposed();
        var policy = NativeMethods.StringToNativeOrNull(policyJson);
        try
        {
            var status = NativeMethods.oxide_signature_validation_options_set_algorithm_policy_json(
                _handle, policy, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(policy);
        }
    }

    /// <summary>
    /// Imports a content-addressed evidence bundle. It remains untrusted until
    /// the normal validation pipeline rechecks every certificate, OCSP
    /// response, and CRL.
    /// </summary>
    public void SetEvidenceBundleJson(string bundleJson)
    {
        ArgumentNullException.ThrowIfNull(bundleJson);
        ThrowIfDisposed();
        var bundle = NativeMethods.StringToNativeOrNull(bundleJson);
        try
        {
            var status = NativeMethods.oxide_signature_validation_options_set_evidence_bundle_json(
                _handle, bundle, out var error);
            NativeMethods.ThrowIfError(status, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(bundle);
        }
    }

    public void SetPathLimits(nuint maxChainDepth, nuint maxPathCandidates)
    {
        ThrowIfDisposed();
        if (maxChainDepth == 0 || maxPathCandidates == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(maxChainDepth));
        }
        var status = NativeMethods.oxide_signature_validation_options_set_path_limits(
            _handle, (UIntPtr)maxChainDepth, (UIntPtr)maxPathCandidates, out var error);
        NativeMethods.ThrowIfError(status, error);
    }

    public void Dispose()
    {
        if (_disposed) return;
        _handle.Dispose();
        _disposed = true;
        GC.SuppressFinalize(this);
    }

    private delegate int AddDerDelegate(
        NativeMethods.SignatureValidationOptionsHandle options,
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
        {
            throw new ObjectDisposedException(nameof(SignatureValidationOptions));
        }
    }
}

/// <summary>Prompt 24 revocation-policy modes exposed by the native handle.</summary>
public enum SignatureRevocationMode
{
    NotChecked = 0,
    OfflineStrict = 1,
    OfflineBestEffort = 2,
    OnlineStrict = 3,
    OnlineBestEffort = 4,
}
