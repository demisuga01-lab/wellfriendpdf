using System.Runtime.InteropServices;
using System.Text;

namespace Oxide.Sdk;

public sealed class OxideDocument : IDisposable
{
    private readonly NativeMethods.DocumentHandle _handle;
    private bool _disposed;

    private OxideDocument(NativeMethods.DocumentHandle handle)
    {
        _handle = handle;
    }

    public int PageCount
    {
        get
        {
            ThrowIfDisposed();
            var status = NativeMethods.oxide_document_page_count(_handle, out var count, out var error);
            NativeMethods.ThrowIfError(status, error);
            return checked((int)count.ToUInt64());
        }
    }

    public IReadOnlyList<Page> Pages => Enumerable.Range(1, PageCount).Select(n => new Page(this, n)).ToArray();

    public static OxideDocument Open(string path, string? password = null)
    {
        ArgumentNullException.ThrowIfNull(path);
        return Open(File.ReadAllBytes(path), password);
    }

    public static OxideDocument Open(byte[] bytes, string? password = null)
    {
        ArgumentNullException.ThrowIfNull(bytes);
        NativeMethods.DocumentHandle handle;
        IntPtr error;
        if (password is null)
        {
            handle = NativeMethods.oxide_document_open_from_bytes(bytes, (UIntPtr)bytes.Length, out error);
        }
        else
        {
            var encoded = Encoding.UTF8.GetBytes(password);
            // Keep a non-null password pointer even when the caller supplied an
            // explicit empty string; the native length remains the actual UTF-8
            // byte count.
            var nativePassword = encoded.Length == 0 ? new byte[1] : encoded;
            handle = NativeMethods.oxide_document_open_from_bytes_with_password(
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
        return new OxideDocument(handle);
    }

    public static string FeatureReportJson()
    {
        var status = NativeMethods.oxide_feature_report_json(out var json, out var error);
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
            var status = NativeMethods.oxide_codec_isolation_report_json(
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
        return NativeMethods.TakeString(NativeMethods.oxide_version());
    }

    public static uint AbiVersion => NativeMethods.oxide_abi_version();

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
        var status = NativeMethods.oxide_document_extract_text(_handle, (UIntPtr)pageNumber, out var text, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(text);
    }

    public string ParseJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_parse_json(_handle, out var json, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeString(json);
    }

    public string ExtractFieldsJson(string? documentType = null)
    {
        ThrowIfDisposed();
        var docTypePtr = documentType is null ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(documentType);
        try
        {
            var status = NativeMethods.oxide_document_extract_fields_json(_handle, docTypePtr, out var json, out var error);
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
        var status = NativeMethods.oxide_document_security_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ParserReportJson(string mode = "repair")
    {
        ThrowIfDisposed();
        return ReportWithString(mode, NativeMethods.oxide_document_parser_report_json);
    }

    public string ColorReportJson(string profile = "generic")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.oxide_document_color_report_json);
    }

    public string ValidateJson(string profile = "all")
    {
        ThrowIfDisposed();
        return ReportWithString(profile, NativeMethods.oxide_document_validate_json);
    }

    public string FormsReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_forms_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_xfa_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaExtractJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_xfa_extract_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaScriptReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_xfa_script_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaSecurityReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_xfa_security_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string XfaRuntimeReportJson(string scriptPolicy = "disabled", bool executeEvents = false)
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(scriptPolicy);
        try
        {
            var status = NativeMethods.oxide_document_xfa_runtime_report_json(
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
        var status = NativeMethods.oxide_document_annotations_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string RichMediaReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_rich_media_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt17ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_prompt17_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt18ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_prompt18_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt18bReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_prompt18b_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string FormJavaScriptReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_form_js_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string FormActionGraphJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_form_action_graph_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string InteractiveDataReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_interactive_data_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string WordPaginationAuditJson(string layout = "page-faithful")
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(layout);
        return ReportWithString(layout, NativeMethods.oxide_document_word_pagination_audit_json);
    }

    public string Prompt19ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_prompt19_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt20ReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_prompt20_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string Prompt20VectorListJson(nuint page = 1)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var status = NativeMethods.oxide_document_prompt20_vector_list_json(
            _handle, (UIntPtr)page, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public OxideBinaryResult Prompt20TextEdit(
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
            var status = NativeMethods.oxide_document_prompt20_text_edit_json(
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

    public OxideBinaryResult Prompt20VectorEdit(
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
            var status = NativeMethods.oxide_document_prompt20_vector_edit_json(
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

    public OxideBinaryResult Prompt20InkFit(
        nuint page, nuint annotationIndex = 0, string? optionsJson = null,
        bool signaturePolicyOverride = false)
    {
        ThrowIfDisposed();
        if (page == 0) throw new ArgumentOutOfRangeException(nameof(page));
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_prompt20_ink_fit_json(
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
        var status = NativeMethods.oxide_document_associated_files_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string EditPolicyReportJson(string operation)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(operation);
        return ReportWithString(operation, NativeMethods.oxide_document_edit_policy_report_json);
    }

    public string AnnotationAppearanceReportJson(string? optionsJson = null)
    {
        ThrowIfDisposed();
        return ReportWithString(optionsJson, NativeMethods.oxide_document_annotation_appearance_report_json);
    }

    public string NonaxisRedactionPlanJson(string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        return ReportWithString(optionsJson, NativeMethods.oxide_document_nonaxis_redaction_plan_json);
    }

    public string PagesReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_pages_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string InteractiveReportJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_interactive_report_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string ChunksJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_chunks_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string AdvancedChunksJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_advanced_chunks_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string SemanticBundleJson()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_semantic_bundle_json(_handle, out var json, out var error);
        return NativeMethods.TakeJson(status, json, error);
    }

    public string SemanticSearchJson(string query)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(query);
        return ReportWithString(query, NativeMethods.oxide_document_semantic_search_json);
    }

    public OxideBinaryResult XfaRender(
        string scriptPolicy = "disabled",
        bool executeEvents = false,
        uint dpi = 72)
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(scriptPolicy);
        try
        {
            var status = NativeMethods.oxide_document_xfa_render_json(
                _handle, policyPtr, executeEvents ? 1 : 0, dpi,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (policyPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(policyPtr);
        }
    }

    public OxideBinaryResult XfaFlatten(string mode = "flatten_supported_static")
    {
        ThrowIfDisposed();
        return XfaModeOutput(mode, NativeMethods.oxide_document_xfa_flatten_json);
    }

    public OxideBinaryResult XfaSanitize(string mode = "remove_scripts_events_connections")
    {
        ThrowIfDisposed();
        return XfaModeOutput(mode, NativeMethods.oxide_document_xfa_sanitize_json);
    }

    public OxideBinaryResult AnnotationXfdfExport()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_annotation_xfdf_export_json(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public OxideBinaryResult AnnotationXfdfImport(byte[] xfdf, string? optionsJson = null)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(xfdf);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_annotation_xfdf_import_json(
                _handle, xfdf, (UIntPtr)xfdf.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public OxideBinaryResult AnnotationAppearanceGenerate(string? optionsJson = null)
    {
        ThrowIfDisposed();
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_annotation_appearance_generate_json(
                _handle, optionsPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public OxideBinaryResult RichMediaSanitize(
        string mode = "remove_active_content",
        string? customJson = null)
    {
        ThrowIfDisposed();
        var modePtr = NativeMethods.StringToNativeOrNull(mode);
        var customPtr = NativeMethods.StringToNativeOrNull(customJson);
        try
        {
            var status = NativeMethods.oxide_document_rich_media_sanitize_json(
                _handle, modePtr, customPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (modePtr != IntPtr.Zero) Marshal.FreeCoTaskMem(modePtr);
            if (customPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(customPtr);
        }
    }

    public OxideBinaryResult RichMediaFlattenPoster()
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_rich_media_flatten_poster_json(
            _handle, out var buffer, out var json, out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public OxideBinaryResult RedactImageNonaxis(string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_nonaxis_redaction_apply_json(
                _handle, optionsPtr, out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public OxideBinaryResult RedactImageMask(string optionsJson) => Prompt18StringOutput(
        optionsJson, NativeMethods.oxide_document_redact_image_mask_json);

    public OxideBinaryResult RedactInlineImage(string optionsJson) => Prompt18StringOutput(
        optionsJson, NativeMethods.oxide_document_redact_inline_image_json);

    public OxideBinaryResult AssociatedFileAdd(byte[] payload, string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(payload);
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_associated_files_add_json(
                _handle, payload, (UIntPtr)payload.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public OxideBinaryResult AssociatedFileUpdateOwner(byte[] payload, string optionsJson)
    {
        ThrowIfDisposed();
        ArgumentNullException.ThrowIfNull(payload);
        ArgumentException.ThrowIfNullOrWhiteSpace(optionsJson);
        var optionsPtr = NativeMethods.StringToNativeOrNull(optionsJson);
        try
        {
            var status = NativeMethods.oxide_document_associated_files_update_owner_json(
                _handle, payload, (UIntPtr)payload.Length, optionsPtr,
                out var buffer, out var json, out var error);
            return NativeMethods.TakeOutput(status, buffer, json, error);
        }
        finally
        {
            if (optionsPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(optionsPtr);
        }
    }

    public OxideBinaryResult AssociatedFileRemoveOwner(string optionsJson) =>
        Prompt18StringOutput(optionsJson, NativeMethods.oxide_document_associated_files_remove_owner_json);

    public OxideBinaryResult IncrementalFormEdit(
        string fieldName, string value, bool signaturePolicyOverride = false)
    {
        ThrowIfDisposed();
        var fieldPtr = NativeMethods.StringToNativeOrNull(fieldName);
        var valuePtr = NativeMethods.StringToNativeOrNull(value);
        try
        {
            var status = NativeMethods.oxide_document_incremental_form_edit_json(
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

    public OxideBinaryResult IncrementalAnnotationEdit(
        string optionsJson, bool signaturePolicyOverride = false) =>
        Prompt18bPolicyOutput(optionsJson, signaturePolicyOverride,
            NativeMethods.oxide_document_incremental_annotation_edit_json);

    public OxideBinaryResult IncrementalPagePropertyEdit(
        string optionsJson, bool signaturePolicyOverride = false) =>
        Prompt18bPolicyOutput(optionsJson, signaturePolicyOverride,
            NativeMethods.oxide_document_incremental_page_property_edit_json);

    private delegate int Prompt18bPolicyOutputCall(
        NativeMethods.DocumentHandle document, IntPtr value, bool signaturePolicyOverride,
        out NativeMethods.OxideBuffer buffer, out IntPtr json, out IntPtr error);

    private OxideBinaryResult Prompt18bPolicyOutput(
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

    public OxideBinaryResult AssociatedFileExtract(string stableId) =>
        Prompt18StringOutput(stableId, NativeMethods.oxide_document_associated_files_extract_json);

    public OxideBinaryResult AssociatedFilesRemove(string stableIdsJson) =>
        Prompt18StringOutput(stableIdsJson, NativeMethods.oxide_document_associated_files_remove_json);

    public OxideBinaryResult AssociatedFilesSanitize(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.oxide_document_associated_files_sanitize_json);

    public OxideBinaryResult FormJavaScriptSanitize(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.oxide_document_form_js_sanitize_json);

    public OxideBinaryResult FormJavaScriptFlattenValues(string? optionsJson = null) =>
        Prompt18StringOutput(optionsJson, NativeMethods.oxide_document_form_js_flatten_values_json);

    private delegate int Prompt18OutputCall(
        NativeMethods.DocumentHandle document, IntPtr value, out NativeMethods.OxideBuffer buffer,
        out IntPtr json, out IntPtr error);

    private OxideBinaryResult Prompt18StringOutput(string? value, Prompt18OutputCall call)
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

    public OxideBinaryResult Sanitize(string policy = "balanced")
    {
        ThrowIfDisposed();
        var policyPtr = NativeMethods.StringToNativeOrNull(policy);
        try
        {
            var status = NativeMethods.oxide_document_sanitize_json(_handle, policyPtr, out var buffer, out var json, out var error);
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

    public OxideBinaryResult Canonicalize(long? dateEpoch = null)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_canonicalize_json(
            _handle,
            dateEpoch.GetValueOrDefault(),
            dateEpoch.HasValue ? 1 : 0,
            out var buffer,
            out var json,
            out var error);
        return NativeMethods.TakeOutput(status, buffer, json, error);
    }

    public OxideBinaryResult RedactTerms(IEnumerable<string> terms, bool strict = false)
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

            var status = NativeMethods.oxide_document_redact_terms_json(
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
            var status = NativeMethods.oxide_document_to_xlsx(_handle, layoutPtr, out var buffer, out var error);
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
        var status = NativeMethods.oxide_document_to_pptx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
        NativeMethods.ThrowIfError(status, error);
        return NativeMethods.TakeBuffer(buffer);
    }

    public byte[] ToDocx(bool includeImages = true)
    {
        ThrowIfDisposed();
        var status = NativeMethods.oxide_document_to_docx(_handle, includeImages ? 1 : 0, out var buffer, out var error);
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
            var status = NativeMethods.oxide_document_to_docx_with_layout(
                _handle, includeImages ? 1 : 0, layoutPtr, out var buffer, out var error);
            NativeMethods.ThrowIfError(status, error);
            return NativeMethods.TakeBuffer(buffer);
        }
        finally
        {
            if (layoutPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(layoutPtr);
        }
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
        out NativeMethods.OxideBuffer buffer,
        out IntPtr json,
        out IntPtr error);

    private OxideBinaryResult XfaModeOutput(string mode, StringOutputCall call)
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
    private readonly OxideDocument _document;

    internal Page(OxideDocument document, int pageNumber)
    {
        _document = document;
        Number = pageNumber;
    }

    public int Number { get; }

    public string Text => _document.ExtractText(Number);
}
