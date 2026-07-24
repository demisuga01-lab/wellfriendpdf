using System.Runtime.InteropServices;
using System.Text;

namespace WellfriendPdf;

public sealed class WellfriendDocument : IDisposable
{
    private readonly NativeMethods.DocumentHandle _handle;
    private bool _disposed;

    private WellfriendDocument(NativeMethods.DocumentHandle handle)
    {
        _handle = handle;
    }

    public int PageCount
    {
        get
        {
            ThrowIfDisposed();
            var status = NativeMethods.wellfriendpdf_document_page_count(_handle, out var count, out var error);
            NativeMethods.ThrowIfError(status, error);
            return checked((int)count.ToUInt64());
        }
    }

    public IReadOnlyList<Page> Pages => Enumerable.Range(1, PageCount).Select(n => new Page(this, n)).ToArray();

    public static WellfriendDocument Open(string path, string? password = null)
    {
        ArgumentNullException.ThrowIfNull(path);
        return Open(File.ReadAllBytes(path), password);
    }

    public static WellfriendDocument Open(byte[] bytes, string? password = null)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        NativeMethods.DocumentHandle handle;
        IntPtr error;
        if (password is null)
        {
            handle = NativeMethods.wellfriendpdf_document_open_from_bytes(bytes, (UIntPtr)bytes.Length, out error);
        }
        else
        {
            var encoded = Encoding.UTF8.GetBytes(password);
            // Keep a non-null password pointer even when the caller supplied an
            // explicit empty string; the native length remains the actual UTF-8
            // byte count.
            var nativePassword = encoded.Length == 0 ? new byte[1] : encoded;
            handle = NativeMethods.wellfriendpdf_document_open_from_bytes_with_password(
                bytes,
                (UIntPtr)bytes.Length,
                nativePassword,
                (UIntPtr)encoded.Length,
                out error);
        }
        if (handle.IsInvalid)
        {
            NativeMethods.ThrowIfError(2, error);
        }
        return new WellfriendDocument(handle);
    }

    public static WellfriendDocument OpenPubSec(byte[] bytes, byte[] certificate, byte[] privateKey)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        ArgumentNullException.ThrowIfNull(certificate);
        ArgumentNullException.ThrowIfNull(privateKey);
        var handle = NativeMethods.wellfriendpdf_document_open_pubsec_from_bytes(
            bytes,
            (UIntPtr)bytes.Length,
            certificate,
            (UIntPtr)certificate.Length,
            privateKey,
            (UIntPtr)privateKey.Length,
            out var error);
        if (handle.IsInvalid)
        {
            NativeMethods.ThrowIfError(2, error);
        }
        return new WellfriendDocument(handle);
    }

    public static WellfriendDocument OpenPubSecPfx(byte[] bytes, byte[] pfx, byte[] password)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        ArgumentNullException.ThrowIfNull(pfx);
        password ??= Array.Empty<byte>();
        var handle = NativeMethods.wellfriendpdf_document_open_pubsec_pfx_from_bytes(
            bytes,
            (UIntPtr)bytes.Length,
            pfx,
            (UIntPtr)pfx.Length,
            password,
            (UIntPtr)password.Length,
            out var error);
        if (handle.IsInvalid)
        {
            NativeMethods.ThrowIfError(2, error);
        }
        return new WellfriendDocument(handle);
    }

    public static string FeatureReportJson()
    {
        var status = NativeMethods.wellfriendpdf_feature_report_json(out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public static string Prompt21HistoryReportJson()
    {
        var status = NativeMethods.wellfriendpdf_prompt21_history_report_json(out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public static string CryptoTamperTestJson()
    {
        var status = NativeMethods.wellfriendpdf_crypto_tamper_test_json(out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public static string CodecIsolationReportJson(
        string filter,
        byte[] encodedBytes,
        string policy = "in_process")
    {
        ArgumentNullException.ThrowIfNull(filter);
        ArgumentNullException.ThrowIfNull(encodedBytes);
        var filterPtr = NativeMethods.StringToNativeOrNull(filter);
        var policyPtr = NativeMethods.StringToNativeOrNull(policy);
        try
        {
            var status = NativeMethods.wellfriendpdf_codec_isolation_report_json(
                filterPtr,
                encodedBytes,
                (UIntPtr)encodedBytes.Length,
                policyPtr,
                out var json,
                out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (filterPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(filterPtr);
            }
            if (policyPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(policyPtr);
            }
        }
    }

    public static string EngineVersion()
    {
        return NativeMethods.TakeString(NativeMethods.wellfriendpdf_version());
    }

    public static uint AbiVersion => NativeMethods.wellfriendpdf_abi_version();

    public Page GetPage(int pageNumber)
    {
        if (pageNumber < 1 || pageNumber > PageCount)
        {
            throw new ArgumentOutOfRangeException(nameof(pageNumber), "Page numbers are 1-based.");
        }
        return new Page(this, pageNumber);
    }

    public string ExtractText(int pageNumber)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_extract_text(_handle, (UIntPtr)pageNumber, out var text, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(text);
    }

    public string ParseJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_parse_json(_handle, out var json, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(json);
    }

    public string ExtractFieldsJson(string? documentType = null)
    {
        ThrowIfDisposed();
        var docTypePtr = documentType is null ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(documentType);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_extract_fields_json(_handle, docTypePtr, out var json, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeString(json);
        }
        finally
        {
            if (docTypePtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(docTypePtr);
            }
        }
    }

    public string SecurityReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_security_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ParserReportJson(string mode = "repair")
    {
        ThrowIfDisposed();
        return ReportWithString(mode, NativeMethods.wellfriendpdf_document_parser_report_json);
    }

    public string ColorReportJson(string profile = "generic")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.wellfriendpdf_document_color_report_json);
    }

    public string ValidateJson(string profile = "all")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.wellfriendpdf_document_validate_json);
    }

    /// <summary>Returns the clause-mapped PDF/A standards envelope.</summary>
    public string ValidatePdfaStandardsJson(string? target = null)
    {
        ThrowIfDisposed();
        return ReportWithString(target, NativeMethods.wellfriendpdf_document_pdfa_standards_json);
    }

    /// <summary>Returns the clause-mapped PDF/UA standards envelope.</summary>
    public string ValidatePdfuaStandardsJson(string? target = null)
    {
        ThrowIfDisposed();
        return ReportWithString(target, NativeMethods.wellfriendpdf_document_pdfua_standards_json);
    }

    /// <summary>Returns the clause-mapped PDF/X standards envelope.</summary>
    public string ValidatePdfxStandardsJson(string? target = null)
    {
        ThrowIfDisposed();
        return ReportWithString(target, NativeMethods.wellfriendpdf_document_pdfx_standards_json);
    }

    /// <summary>
    /// Returns the combined PDF/A, PDF/UA, and PDF/X standards envelope,
    /// including cross-profile conflicts; a pass from one profile never masks
    /// failures from another profile.
    /// </summary>
    public string ValidateAllStandardsJson(string? target = null)
    {
        ThrowIfDisposed();
        return ReportWithString(target, NativeMethods.wellfriendpdf_document_standards_all_json);
    }

    /// <summary>Returns the native post-signature cryptographic validation report.</summary>
    public string SignatureReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_signatures_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    /// <summary>
    /// Creates an append-only incremental signing placeholder plan. PEM signer
    /// material is copied into native code for the call and is never logged.
    /// </summary>
    public string IncrementalSigningPlanJson(
        string keyPem,
        string certPem,
        int placeholderSize = 16 * 1024,
        int certify = 0)
    {
        ThrowIfDisposed();
        ValidateSigningInputs(keyPem, certPem, placeholderSize, certify);
        var keyPtr = NativeMethods.StringToNativeOrNull(keyPem);
        var certPtr = NativeMethods.StringToNativeOrNull(certPem);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_sign_plan_json(
                _handle, keyPtr, certPtr, (UIntPtr)placeholderSize, certify, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(keyPtr);
            Marshal.FreeCoTaskMem(certPtr);
        }
    }

    /// <summary>
    /// Performs real append-only local PEM signing through the C ABI. The
    /// native engine reopens and verifies the returned PDF before this method
    /// returns its owned bytes and IncrementalSignResult JSON report.
    /// </summary>
    public WellfriendBinaryResult SignIncremental(
        string keyPem,
        string certPem,
        int placeholderSize = 16 * 1024,
        int certify = 0,
        string? fieldName = null,
        string? reason = null)
    {
        ThrowIfDisposed();
        ValidateSigningInputs(keyPem, certPem, placeholderSize, certify);
        var keyPtr = NativeMethods.StringToNativeOrNull(keyPem);
        var certPtr = NativeMethods.StringToNativeOrNull(certPem);
        var fieldPtr = NativeMethods.StringToNativeOrNull(fieldName);
        var reasonPtr = NativeMethods.StringToNativeOrNull(reason);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_sign_pdf(
                _handle,
                keyPtr,
                certPtr,
                (UIntPtr)placeholderSize,
                certify,
                fieldPtr,
                reasonPtr,
                out var buffer,
                out var json,
                out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            Marshal.FreeCoTaskMem(keyPtr);
            Marshal.FreeCoTaskMem(certPtr);
            if (fieldPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(fieldPtr);
            if (reasonPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(reasonPtr);
        }
    }

    /// <summary>
    /// Returns the existing combined edit-policy report focused on DocMDP
    /// permissions for the requested operation.
    /// </summary>
    public string DocMdpPermissionReportJson(string operation = "form_value_update") =>
        EditPolicyReportJson(operation);

    /// <summary>
    /// Returns the existing combined edit-policy report focused on FieldMDP
    /// permissions for the requested operation.
    /// </summary>
    public string FieldMdpPermissionReportJson(string operation = "form_value_update") =>
        EditPolicyReportJson(operation);

    private static void ValidateSigningInputs(
        string keyPem,
        string certPem,
        int placeholderSize,
        int certify)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(keyPem);
        ArgumentException.ThrowIfNullOrWhiteSpace(certPem);
        if (placeholderSize <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(placeholderSize), "Placeholder size must be positive.");
        }
        if (certify is < 0 or > 3)
        {
            throw new ArgumentOutOfRangeException(nameof(certify), "Certification permission must be 0 or 1 through 3.");
        }
    }

    public string FormsReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_forms_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_xfa_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaExtractJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_xfa_extract_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaScriptReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_xfa_script_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaSecurityReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_xfa_security_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaRuntimeReportJson(string scriptPolicy = "disabled", bool executeEvents = false)
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(scriptPolicy);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_xfa_runtime_report_json(
                _handle, policyPtr, executeEvents ? 1 : 0, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (policyPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(policyPtr);
        }
    }

    public string AnnotationsReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_annotations_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string RichMediaReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_rich_media_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt17ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt17_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt18ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt18_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt18bReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt18b_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string FormJavaScriptReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_form_js_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string FormActionGraphJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_form_action_graph_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string InteractiveDataReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_interactive_data_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string WordPaginationAuditJson(string layout = "page-faithful")
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(layout);
        return ReportWithString(layout, NativeMethods.wellfriendpdf_document_word_pagination_audit_json);
    }

    public string Prompt19ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt19_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt20ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt20_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt20bReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt20b_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt21ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt21_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt21RasterVectorReportJson(nuint page = 1, string? optionsJson = null)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt21_raster_vector_report_json(
                _handle, (UIntPtr)page, optionsPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(optionsPtr);
            }
        }
    }

    public string Prompt21FontReconstructionReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt21_font_reconstruction_report_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt21ObjectStreamReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt21_object_stream_report_json(
            _handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public WellfriendBinaryResult Prompt21PackObjectStreams()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt21_pack_object_streams_pdf(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public string Prompt22ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt22_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt23ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_prompt23_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string SignatureReportWithOptionsJson(string? optionsJson = null)
    {
        ThrowIfDisposed();
        return ReportWithString(optionsJson, NativeMethods.wellfriendpdf_document_signatures_with_options_json);
    }

    public string SignatureValidationWithEvidenceJson(string? optionsJson = null)
    {
        ThrowIfDisposed();
        return ReportWithString(optionsJson, NativeMethods.wellfriendpdf_document_signature_validation_with_evidence_json);
    }

    public static string TimestampTokenValidationJson(
        byte[] token, byte[] signatureValue, string? optionsJson = null)
    {
        ArgumentNullException.ThrowIfNull(token);
        ArgumentNullException.ThrowIfNull(signatureValue);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_timestamp_token_validation_json(
                token, (UIntPtr)token.Length, signatureValue, (UIntPtr)signatureValue.Length,
                optionsPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    /// <summary>
    /// Validates signatures with an owned Prompt 24 configuration handle.
    /// The handle exposes explicit trust anchors, intermediates, evidence, and
    /// bounded retrieval policy without requiring callers to assemble a raw
    /// options JSON payload.
    /// </summary>
    public string SignatureValidationReport(SignatureValidationOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_signatures_with_options_handle(
            _handle, options.Handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    /// <summary>
    /// Validates signatures with an owned Prompt 24 configuration handle and
    /// returns the report plus a portable accepted-evidence bundle.
    /// </summary>
    public string SignatureValidationWithEvidence(SignatureValidationOptions options)
    {
        ArgumentNullException.ThrowIfNull(options);
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_signature_validation_with_evidence_handle(
            _handle, options.Handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string WriterDeterminismAuditJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_writer_determinism_audit_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string WriterExternalDiffJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_writer_external_diff_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string WriterCloseoutReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_writer_closeout_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string PubsecReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_pubsec_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string AesGcmReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_aes_gcm_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string PdfMacReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_pdf_mac_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string PdfMacVerifyJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_pdf_mac_verify_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public WellfriendBinaryResult PdfMacCreate()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_pdf_mac_create_pdf(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public WellfriendBinaryResult Prompt22Optimize(string? optionsJson = null)
    {
        ThrowIfDisposed();
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt22_optimize_pdf(
                _handle, optionsPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(optionsPtr);
            }
        }
    }

    public string Prompt20bTextRangeAnalyzeJson(nuint page = 1)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var status = NativeMethods.wellfriendpdf_document_prompt20b_text_range_analyze_json(
            _handle, (UIntPtr)page, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public WellfriendBinaryResult EditTextRange(string requestJson)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(requestJson);
        var requestPtr = NativeMethods.StringToNativeOrNull(requestJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt20b_text_range_edit_json(
                _handle, requestPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (requestPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(requestPtr);
        }
    }

    public string Prompt20VectorListJson(nuint page = 1)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var status = NativeMethods.wellfriendpdf_document_prompt20_vector_list_json(
            _handle, (UIntPtr)page, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public WellfriendBinaryResult Prompt20TextEdit(
        nuint page, string oldText, string newText, string mode = "rtl-reflow", string? optionsJson = null)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        ArgumentException.ThrowIfNullOrWhiteSpace(oldText);
        ArgumentNullException.ThrowIfNull(newText);
        ArgumentException.ThrowIfNullOrWhiteSpace(mode);
        var oldPtr = NativeMethods.StringToNativeOrNull(oldText);
        var newPtr = NativeMethods.StringToNativeOrNull(newText);
        var modePtr = NativeMethods.StringToNativeOrNull(mode);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt20_text_edit_json(
                _handle, (UIntPtr)page, oldPtr, newPtr, modePtr, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (oldPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(oldPtr);
            if (newPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(newPtr);
            if (modePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(modePtr);
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult Prompt20VectorEdit(
        nuint page, string stableId, string operationJson, string? optionsJson = null)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        ArgumentException.ThrowIfNullOrWhiteSpace(stableId);
        ArgumentException.ThrowIfNullOrWhiteSpace(operationJson);
        var idPtr = NativeMethods.StringToNativeOrNull(stableId);
        var operationPtr = NativeMethods.StringToNativeOrNull(operationJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt20_vector_edit_json(
                _handle, (UIntPtr)page, idPtr, operationPtr, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (idPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(idPtr);
            if (operationPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(operationPtr);
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult Prompt20InkFit(
        nuint page, nuint annotationIndex = 0, string? optionsJson = null,
        bool signaturePolicyOverride = false)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_prompt20_ink_fit_json(
                _handle, (UIntPtr)page, (UIntPtr)annotationIndex, optionsPtr,
                signaturePolicyOverride ? 1 : 0,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public string AssociatedFilesReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_associated_files_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string EditPolicyReportJson(string operation)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(operation);
        return ReportWithString(operation, NativeMethods.wellfriendpdf_document_edit_policy_report_json);
    }

    public string AnnotationAppearanceReportJson(string? optionsJson = null)
    {
        ThrowIfDisposed();
        return ReportWithString(optionsJson, NativeMethods.wellfriendpdf_document_annotation_appearance_report_json);
    }

    public string NonaxisRedactionPlanJson(string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        return ReportWithString(optionsJson, NativeMethods.wellfriendpdf_document_nonaxis_redaction_plan_json);
    }

    public string PagesReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_pages_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string InteractiveReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_interactive_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ChunksJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_chunks_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string AdvancedChunksJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_advanced_chunks_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string SemanticBundleJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_semantic_bundle_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string SemanticSearchJson(string query)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(query);
        return ReportWithString(query, NativeMethods.wellfriendpdf_document_semantic_search_json);
    }

    public WellfriendBinaryResult XfaRender(
        string scriptPolicy = "disabled",
        bool executeEvents = false,
        uint dpi = 72)
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(scriptPolicy);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_xfa_render_json(
                _handle, policyPtr, executeEvents ? 1 : 0, dpi,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (policyPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(policyPtr);
        }
    }

    public WellfriendBinaryResult XfaFlatten(string mode = "flatten_supported_static")
    {
        ThrowIfDisposed();
        return XfaModeOutput(mode, NativeMethods.wellfriendpdf_document_xfa_flatten_json);
    }

    public WellfriendBinaryResult XfaSanitize(string mode = "remove_scripts_events_connections")
    {
        ThrowIfDisposed();
        return XfaModeOutput(mode, NativeMethods.wellfriendpdf_document_xfa_sanitize_json);
    }

    public WellfriendBinaryResult AnnotationXfdfExport()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_annotation_xfdf_export_json(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public WellfriendBinaryResult AnnotationXfdfImport(byte[] xfdf, string? optionsJson = null)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(xfdf);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_annotation_xfdf_import_json(
                _handle, xfdf, (UIntPtr)xfdf.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult AnnotationAppearanceGenerate(string? optionsJson = null)
    {
        ThrowIfDisposed();
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_annotation_appearance_generate_json(
                _handle, optionsPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult RichMediaSanitize(
        string mode = "remove_active_content",
        string? customJson = null)
    {
        ThrowIfDisposed();
        var modePtr = NativeMethods.StringToNativeOrNull(mode);
        var customPtr = NativeMethods.StringToNativeOrNull(customJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_rich_media_sanitize_json(
                _handle, modePtr, customPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (modePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(modePtr);
            if (customPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(customPtr);
        }
    }

    public WellfriendBinaryResult RichMediaFlattenPoster()
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_rich_media_flatten_poster_json(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public WellfriendBinaryResult RedactImageNonaxis(string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_nonaxis_redaction_apply_json(
                _handle, optionsPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult RedactImageMask(string optionsJson) => Prompt18StringOutput(
        optionsJson, NativeMethods.wellfriendpdf_document_redact_image_mask_json);

    public WellfriendBinaryResult RedactInlineImage(string optionsJson) => Prompt18StringOutput(
        optionsJson, NativeMethods.wellfriendpdf_document_redact_inline_image_json);

    public WellfriendBinaryResult AssociatedFileAdd(byte[] payload, string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(payload);
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_associated_files_add_json(
                _handle, payload, (UIntPtr)payload.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult AssociatedFileUpdateOwner(byte[] payload, string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(payload);
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_associated_files_update_owner_json(
                _handle, payload, (UIntPtr)payload.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult AssociatedFileRemoveOwner(string optionsJson) =>
        Prompt18StringOutput(optionsJson, NativeMethods.wellfriendpdf_document_associated_files_remove_owner_json);

    public WellfriendBinaryResult IncrementalFormEdit(
        string fieldName, string value, bool signaturePolicyOverride = false)
    {
        ThrowIfDisposed();
        var fieldPtr = NativeMethods.StringToNativeOrNull(fieldName);
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_incremental_form_edit_json(
                _handle, fieldPtr, valuePtr, signaturePolicyOverride,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (fieldPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(fieldPtr);
            if (valuePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(valuePtr);
        }
    }

    public string SignaturePreservingFormPlan(
        string fieldName, string value, string? optionsJson = null)
    {
        ThrowIfDisposed();
        var fieldPtr = NativeMethods.StringToNativeOrNull(fieldName);
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_signature_preserving_form_plan_json(
                _handle, fieldPtr, valuePtr, optionsPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (fieldPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(fieldPtr);
            if (valuePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(valuePtr);
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult SignaturePreservingFormEdit(
        string fieldName, string value, string? optionsJson = null,
        bool explicitInvalidationOverride = false)
    {
        ThrowIfDisposed();
        var fieldPtr = NativeMethods.StringToNativeOrNull(fieldName);
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_signature_preserving_form_edit_json(
                _handle, fieldPtr, valuePtr, optionsPtr, explicitInvalidationOverride,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (fieldPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(fieldPtr);
            if (valuePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(valuePtr);
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public WellfriendBinaryResult IncrementalAnnotationEdit(
        string optionsJson, bool signaturePolicyOverride = false) =>
        Prompt18bPolicyOutput(optionsJson, signaturePolicyOverride,
            NativeMethods.wellfriendpdf_document_incremental_annotation_edit_json);

    public WellfriendBinaryResult IncrementalPagePropertyEdit(
        string optionsJson, bool signaturePolicyOverride = false) =>
        Prompt18bPolicyOutput(optionsJson, signaturePolicyOverride,
            NativeMethods.wellfriendpdf_document_incremental_page_property_edit_json);

    private delegate int Prompt18bPolicyOutputCall(
        NativeMethods.DocumentHandle document, IntPtr value, bool signaturePolicyOverride,
        out NativeMethods.WellfriendBuffer buffer, out IntPtr json, out IntPtr error);

    private WellfriendBinaryResult Prompt18bPolicyOutput(
        string value, bool signaturePolicyOverride, Prompt18bPolicyOutputCall call)
    {
        ThrowIfDisposed();
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        try
        {
            var status = call(_handle, valuePtr, signaturePolicyOverride,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (valuePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(valuePtr);
        }
    }

    public WellfriendBinaryResult AssociatedFileExtract(string stableId) =>
        Prompt18StringOutput(stableId, NativeMethods.wellfriendpdf_document_associated_files_extract_json);

    public WellfriendBinaryResult AssociatedFilesRemove(string stableIdsJson) =>
        Prompt18StringOutput(stableIdsJson, NativeMethods.wellfriendpdf_document_associated_files_remove_json);

    public WellfriendBinaryResult AssociatedFilesSanitize(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.wellfriendpdf_document_associated_files_sanitize_json);

    public WellfriendBinaryResult FormJavaScriptSanitize(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.wellfriendpdf_document_form_js_sanitize_json);

    public WellfriendBinaryResult FormJavaScriptFlattenValues(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.wellfriendpdf_document_form_js_flatten_values_json);

    private delegate int Prompt18OutputCall(
        NativeMethods.DocumentHandle document, IntPtr value, out NativeMethods.WellfriendBuffer buffer,
        out IntPtr json, out IntPtr error);

    private WellfriendBinaryResult Prompt18StringOutput(string? value, Prompt18OutputCall call)
    {
        ThrowIfDisposed();
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        try
        {
            var status = call(_handle, valuePtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (valuePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(valuePtr);
        }
    }

    public WellfriendBinaryResult Sanitize(string policy = "balanced")
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(policy);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_sanitize_json(_handle, policyPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (policyPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(policyPtr);
            }
        }
    }

    public WellfriendBinaryResult Canonicalize(long? dateEpoch = null)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_canonicalize_json(
            _handle,
            dateEpoch.GetValueOrDefault(),
            dateEpoch.HasValue ? 1 : 0,
            out var buffer,
            out var json,
            out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public WellfriendBinaryResult RedactTerms(IEnumerable<string> terms, bool strict = false)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(terms);
        var normalized = terms.Where(t => !string.IsNullOrWhiteSpace(t)).ToArray();
        if (normalized.Length == 0)
        {
            throw new ArgumentException("At least one non-empty redaction term is required.", nameof(terms));
        }

        var nativeStrings = new IntPtr[normalized.Length];
        var termsPtr = Marshal.AllocHGlobal(IntPtr.Size * normalized.Length);
        try
        {
            for (var i = 0; i < normalized.Length; i++)
            {
                nativeStrings[i] = Marshal.StringToCoTaskMemUTF8(normalized[i]);
                Marshal.WriteIntPtr(termsPtr, i * IntPtr.Size, nativeStrings[i]);
            }

            var status = NativeMethods.wellfriendpdf_document_redact_terms_json(
                _handle,
                termsPtr,
                (UIntPtr)normalized.Length,
                strict ? 1 : 0,
                out var buffer,
                out var json,
                out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            foreach (var ptr in nativeStrings)
            {
                if (ptr != IntPtr.Zero)
                {
                    Marshal.FreeCoTaskMem(ptr);
                }
            }
            Marshal.FreeHGlobal(termsPtr);
        }
    }

    public byte[] ToXlsx(string layout = "pages")
    {
        ThrowIfDisposed();
        var layoutPtr = Marshal.StringToCoTaskMemUTF8(layout);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_to_xlsx(_handle, layoutPtr, out var buffer, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(buffer);
        }
        finally
        {
            Marshal.FreeCoTaskMem(layoutPtr);
        }
    }

    public byte[] ToPptx(bool includeImages = true)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_to_pptx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public byte[] ToDocx(bool includeImages = true)
    {
        ThrowIfDisposed();
        var status = NativeMethods.wellfriendpdf_document_to_docx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public byte[] ToDocx(string layout, bool includeImages = true)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(layout);
        var layoutPtr = NativeMethods.StringToNativeOrNull(layout);
        try
        {
            var status = NativeMethods.wellfriendpdf_document_to_docx_with_layout(
                _handle, includeImages ? 1 : 0, layoutPtr, out var buffer, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(buffer);
        }
        finally
        {
            if (layoutPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(layoutPtr);
        }
    }

    public byte[] PubSecEncryptPdf(byte[] recipientCertificate)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(recipientCertificate);
        var status = NativeMethods.wellfriendpdf_document_pubsec_encrypt_pdf(
            _handle,
            recipientCertificate,
            (UIntPtr)recipientCertificate.Length,
            out var buffer,
            out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }
        _handle.Dispose();
        _disposed = true;
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
    }

    private delegate int StringReportCall(
        NativeMethods.DocumentHandle document,
        IntPtr arg,
        out IntPtr json,
        out IntPtr error);

    private delegate int StringOutputCall(
        NativeMethods.DocumentHandle document,
        IntPtr arg,
        out NativeMethods.WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr error);

    private WellfriendBinaryResult XfaModeOutput(string mode, StringOutputCall call)
    {
        var modePtr = NativeMethods.StringToNativeOrNull(mode);
        try
        {
            var status = call(_handle, modePtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (modePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(modePtr);
        }
    }

    private string ReportWithString(string? arg, StringReportCall call)
    {
        var argPtr = NativeMethods.StringToNativeOrNull(arg);
        try
        {
            var status = call(_handle, argPtr, out var json, out var error);
            return NativeMethods.TakeJson(status, json, error);
        }
        finally
        {
            if (argPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(argPtr);
            }
        }
    }
}

public sealed class Page
{
    private readonly WellfriendDocument _document;

    internal Page(WellfriendDocument document, int pageNumber)
    {
        _document = document;
        Number = pageNumber;
    }

    public int Number { get; }

    public string Text => _document.ExtractText(Number);
}
