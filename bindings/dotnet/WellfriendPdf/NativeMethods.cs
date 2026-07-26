using System.Reflection;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace WellfriendPdf;

internal static partial class NativeMethods
{
    private const string LibraryName = "wellfriendpdf_capi";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(typeof(NativeMethods).Assembly, ResolveLibrary);
    }

    private static IntPtr ResolveLibrary(string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (!string.Equals(libraryName, LibraryName, StringComparison.Ordinal))
        {
            return IntPtr.Zero;
        }

        var explicitPath = Environment.GetEnvironmentVariable("WELLFRIENDPDF_NATIVE_LIBRARY");
        if (!string.IsNullOrWhiteSpace(explicitPath) && File.Exists(explicitPath))
        {
            return NativeLibrary.Load(explicitPath);
        }

        foreach (var candidate in NativeLibraryCandidates(assembly))
        {
            if (File.Exists(candidate))
            {
                return NativeLibrary.Load(candidate);
            }
        }

        return IntPtr.Zero;
    }

    private static IEnumerable<string> NativeLibraryCandidates(Assembly assembly)
    {
        var mapped = MapNativeLibraryName();
        var rid = RuntimeIdentifier();
        var bases = new[]
        {
            AppContext.BaseDirectory,
            Path.GetDirectoryName(assembly.Location) ?? AppContext.BaseDirectory,
            Environment.CurrentDirectory,
            Path.Combine(Environment.CurrentDirectory, "target", "debug"),
            Path.Combine(Environment.CurrentDirectory, "target", "release"),
        };

        foreach (var root in bases.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            yield return Path.Combine(root, mapped);
            yield return Path.Combine(root, "runtimes", rid, "native", mapped);
        }
    }

    private static string RuntimeIdentifier()
    {
        var os = RuntimeInformation.IsOSPlatform(OSPlatform.Windows)
            ? "win"
            : RuntimeInformation.IsOSPlatform(OSPlatform.OSX)
                ? "osx"
                : "linux";
        var arch = RuntimeInformation.ProcessArchitecture switch
        {
            Architecture.X64 => "x64",
            Architecture.Arm64 => "arm64",
            Architecture.X86 => "x86",
            Architecture.Arm => "arm",
            _ => RuntimeInformation.ProcessArchitecture.ToString().ToLowerInvariant(),
        };
        return $"{os}-{arch}";
    }

    private static string MapNativeLibraryName()
    {
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return $"{LibraryName}.dll";
        }
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            return $"lib{LibraryName}.dylib";
        }
        return $"lib{LibraryName}.so";
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct WellfriendBuffer
    {
        public IntPtr Data;
        public UIntPtr Len;
    }

    internal sealed class DocumentHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private DocumentHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_document_free(handle);
            return true;
        }
    }

    internal sealed class SignatureValidationOptionsHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureValidationOptionsHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_validation_options_free(handle);
            return true;
        }
    }

    internal sealed class SignatureTrustStoreHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureTrustStoreHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_trust_store_free(handle);
            return true;
        }
    }

    internal sealed class SignatureIntermediateStoreHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureIntermediateStoreHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_intermediate_store_free(handle);
            return true;
        }
    }

    internal sealed class SignatureEvidenceStoreHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureEvidenceStoreHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_evidence_store_free(handle);
            return true;
        }
    }

    internal sealed class SignatureRetrievalPolicyHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureRetrievalPolicyHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_retrieval_policy_free(handle);
            return true;
        }
    }

    internal sealed class SignatureValidationCancellationHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        private SignatureValidationCancellationHandle()
            : base(ownsHandle: true)
        {
        }

        protected override bool ReleaseHandle()
        {
            wellfriendpdf_signature_validation_cancellation_free(handle);
            return true;
        }
    }

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern DocumentHandle wellfriendpdf_document_open_from_bytes(
        byte[] data,
        UIntPtr len,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern DocumentHandle wellfriendpdf_document_open_from_bytes_with_password(
        byte[] data,
        UIntPtr len,
        byte[] password,
        UIntPtr passwordLen,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern DocumentHandle wellfriendpdf_document_open_pubsec_from_bytes(
        byte[] data,
        UIntPtr len,
        byte[] certificate,
        UIntPtr certificateLen,
        byte[] privateKey,
        UIntPtr privateKeyLen,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern DocumentHandle wellfriendpdf_document_open_pubsec_pfx_from_bytes(
        byte[] data,
        UIntPtr len,
        byte[] pfx,
        UIntPtr pfxLen,
        byte[] password,
        UIntPtr passwordLen,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_document_free(IntPtr document);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_string_free(IntPtr value);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_error_free(IntPtr value);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_buffer_free(WellfriendBuffer buffer);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_page_count(
        DocumentHandle document,
        out UIntPtr count,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_extract_text(
        DocumentHandle document,
        UIntPtr page,
        out IntPtr text,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_parse_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_extract_fields_json(
        DocumentHandle document,
        IntPtr docType,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_to_xlsx(
        DocumentHandle document,
        IntPtr layout,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_to_pptx(
        DocumentHandle document,
        int includeImages,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_to_docx(
        DocumentHandle document,
        int includeImages,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_to_docx_with_layout(
        DocumentHandle document,
        int includeImages,
        IntPtr layout,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pubsec_encrypt_pdf(
        DocumentHandle document,
        byte[] recipientCertificate,
        UIntPtr recipientCertificateLen,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_docx_to_pdf(
        byte[] data,
        UIntPtr len,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_xlsx_to_pdf(
        byte[] data,
        UIntPtr len,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_pptx_to_pdf(
        byte[] data,
        UIntPtr len,
        out WellfriendBuffer buffer,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_security_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_parser_report_json(
        DocumentHandle document,
        IntPtr mode,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_color_report_json(
        DocumentHandle document,
        IntPtr profile,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_validate_json(
        DocumentHandle document,
        IntPtr profile,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdfa_standards_json(
        DocumentHandle document,
        IntPtr target,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdfua_standards_json(
        DocumentHandle document,
        IntPtr target,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdfx_standards_json(
        DocumentHandle document,
        IntPtr target,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_standards_all_json(
        DocumentHandle document,
        IntPtr target,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signatures_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_sign_plan_json(
        DocumentHandle document,
        IntPtr keyPem,
        IntPtr certPem,
        UIntPtr placeholderSize,
        int certify,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_sign_pdf(
        DocumentHandle document,
        IntPtr keyPem,
        IntPtr certPem,
        UIntPtr placeholderSize,
        int certify,
        IntPtr fieldName,
        IntPtr reason,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_forms_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_extract_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_script_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_security_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_runtime_report_json(
        DocumentHandle document,
        IntPtr scriptPolicy,
        int executeEvents,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_annotations_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_rich_media_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt17_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt18_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt18b_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_form_js_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_form_action_graph_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_interactive_data_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt19_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20b_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt21_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt21_raster_vector_report_json(
        DocumentHandle document, UIntPtr page, IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt21_font_reconstruction_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_prompt21_history_report_json(
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt21_object_stream_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt21_pack_object_streams_pdf(
        DocumentHandle document, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt22_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt23_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signatures_with_options_json(
        DocumentHandle document, IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signature_validation_with_evidence_json(
        DocumentHandle document, IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_timestamp_token_validation_json(
        byte[] token, UIntPtr tokenLen, byte[] signatureValue, UIntPtr signatureValueLen,
        IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureValidationOptionsHandle wellfriendpdf_signature_validation_options_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_validation_options_free(IntPtr options);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureTrustStoreHandle wellfriendpdf_signature_trust_store_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_trust_store_free(IntPtr store);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_trust_store_add_anchor_der(
        SignatureTrustStoreHandle store, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_trust_store_add_distrusted_certificate_sha256(
        SignatureTrustStoreHandle store, IntPtr fingerprint, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureIntermediateStoreHandle wellfriendpdf_signature_intermediate_store_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_intermediate_store_free(IntPtr store);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_intermediate_store_add_der(
        SignatureIntermediateStoreHandle store, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureEvidenceStoreHandle wellfriendpdf_signature_evidence_store_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_evidence_store_free(IntPtr store);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_evidence_store_add_ocsp_der(
        SignatureEvidenceStoreHandle store, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_evidence_store_add_crl_der(
        SignatureEvidenceStoreHandle store, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_evidence_store_set_bundle_json(
        SignatureEvidenceStoreHandle store, IntPtr bundleJson, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureRetrievalPolicyHandle wellfriendpdf_signature_retrieval_policy_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_retrieval_policy_free(IntPtr policy);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_retrieval_policy_set_json(
        SignatureRetrievalPolicyHandle policy, IntPtr policyJson, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern SignatureValidationCancellationHandle wellfriendpdf_signature_validation_cancellation_new(
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_cancellation_cancel(
        SignatureValidationCancellationHandle cancellation, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void wellfriendpdf_signature_validation_cancellation_free(IntPtr cancellation);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_apply_trust_store(
        SignatureValidationOptionsHandle options, SignatureTrustStoreHandle store, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_apply_intermediate_store(
        SignatureValidationOptionsHandle options, SignatureIntermediateStoreHandle store, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_apply_evidence_store(
        SignatureValidationOptionsHandle options, SignatureEvidenceStoreHandle store, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_apply_retrieval_policy(
        SignatureValidationOptionsHandle options, SignatureRetrievalPolicyHandle policy, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_cancellation(
        SignatureValidationOptionsHandle options, SignatureValidationCancellationHandle cancellation, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_add_trust_anchor_der(
        SignatureValidationOptionsHandle options, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_add_intermediate_der(
        SignatureValidationOptionsHandle options, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_add_distrusted_certificate_sha256(
        SignatureValidationOptionsHandle options, IntPtr fingerprint, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_add_ocsp_der(
        SignatureValidationOptionsHandle options, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_add_crl_der(
        SignatureValidationOptionsHandle options, byte[] data, UIntPtr len, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_validation_time_unix(
        SignatureValidationOptionsHandle options, ulong validationTimeUnix, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_clear_validation_time(
        SignatureValidationOptionsHandle options, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_revocation_mode(
        SignatureValidationOptionsHandle options, int mode, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_retrieval_policy_json(
        SignatureValidationOptionsHandle options, IntPtr policyJson, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_algorithm_policy_json(
        SignatureValidationOptionsHandle options, IntPtr policyJson, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_evidence_bundle_json(
        SignatureValidationOptionsHandle options, IntPtr bundleJson, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_signature_validation_options_set_path_limits(
        SignatureValidationOptionsHandle options, UIntPtr maxChainDepth, UIntPtr maxPathCandidates,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signatures_with_options_handle(
        DocumentHandle document, SignatureValidationOptionsHandle options, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signature_validation_with_evidence_handle(
        DocumentHandle document, SignatureValidationOptionsHandle options, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_writer_determinism_audit_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_writer_external_diff_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_writer_closeout_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pubsec_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_aes_gcm_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdf_mac_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdf_mac_verify_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pdf_mac_create_pdf(
        DocumentHandle document, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt22_optimize_pdf(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_prompt22_office_inspect_json(
        byte[] data, UIntPtr len, IntPtr format, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_prompt22_office_to_pdf(
        byte[] data, UIntPtr len, IntPtr format, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20b_text_range_analyze_json(
        DocumentHandle document, UIntPtr page, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20b_text_range_edit_json(
        DocumentHandle document, IntPtr requestJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_provenance_json(
        DocumentHandle document, UIntPtr page, IntPtr sourceText, IntPtr replacementText,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_edit_eligibility_json(
        DocumentHandle document, IntPtr requestJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_operator_text_edit_json(
        DocumentHandle document, IntPtr requestJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_path_provenance_json(
        DocumentHandle document, UIntPtr page, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt31_path_edit_json(
        DocumentHandle document, UIntPtr page, IntPtr stableId, IntPtr operationJson,
        IntPtr optionsJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20_vector_list_json(
        DocumentHandle document, UIntPtr page, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20_text_edit_json(
        DocumentHandle document, UIntPtr page, IntPtr oldText, IntPtr newText, IntPtr mode,
        IntPtr optionsJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20_vector_edit_json(
        DocumentHandle document, UIntPtr page, IntPtr stableId, IntPtr operationJson,
        IntPtr optionsJson, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_prompt20_ink_fit_json(
        DocumentHandle document, UIntPtr page, UIntPtr annotationIndex, IntPtr optionsJson,
        int signaturePolicyOverride, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_word_pagination_audit_json(
        DocumentHandle document, IntPtr layout, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_report_json(
        DocumentHandle document, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_edit_policy_report_json(
        DocumentHandle document, IntPtr operation, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_annotation_appearance_report_json(
        DocumentHandle document, IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_nonaxis_redaction_plan_json(
        DocumentHandle document, IntPtr optionsJson, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_pages_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_interactive_report_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_chunks_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_advanced_chunks_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_semantic_bundle_json(
        DocumentHandle document,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_semantic_search_json(
        DocumentHandle document,
        IntPtr query,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_render_json(
        DocumentHandle document,
        IntPtr scriptPolicy,
        int executeEvents,
        uint dpi,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_flatten_json(
        DocumentHandle document,
        IntPtr mode,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_xfa_sanitize_json(
        DocumentHandle document,
        IntPtr mode,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_annotation_xfdf_export_json(
        DocumentHandle document, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_annotation_xfdf_import_json(
        DocumentHandle document,
        byte[] xfdf,
        UIntPtr xfdfLen,
        IntPtr optionsJson,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_annotation_appearance_generate_json(
        DocumentHandle document,
        IntPtr optionsJson,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_rich_media_sanitize_json(
        DocumentHandle document,
        IntPtr mode,
        IntPtr customJson,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_rich_media_flatten_poster_json(
        DocumentHandle document, out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_nonaxis_redaction_apply_json(
        DocumentHandle document,
        IntPtr optionsJson,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_redact_image_mask_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_redact_inline_image_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_add_json(
        DocumentHandle document, byte[] payload, UIntPtr payloadLen, IntPtr optionsJson,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_update_owner_json(
        DocumentHandle document, byte[] payload, UIntPtr payloadLen, IntPtr optionsJson,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_remove_owner_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_incremental_form_edit_json(
        DocumentHandle document, IntPtr fieldName, IntPtr value,
        [MarshalAs(UnmanagedType.I1)] bool signaturePolicyOverride,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signature_preserving_form_plan_json(
        DocumentHandle document, IntPtr fieldName, IntPtr value, IntPtr optionsJson,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_signature_preserving_form_edit_json(
        DocumentHandle document, IntPtr fieldName, IntPtr value, IntPtr optionsJson,
        [MarshalAs(UnmanagedType.I1)] bool explicitInvalidationOverride,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_incremental_annotation_edit_json(
        DocumentHandle document, IntPtr optionsJson,
        [MarshalAs(UnmanagedType.I1)] bool signaturePolicyOverride,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_incremental_page_property_edit_json(
        DocumentHandle document, IntPtr optionsJson,
        [MarshalAs(UnmanagedType.I1)] bool signaturePolicyOverride,
        out WellfriendBuffer buffer, out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_extract_json(
        DocumentHandle document, IntPtr stableId, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_remove_json(
        DocumentHandle document, IntPtr stableIdsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_associated_files_sanitize_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_form_js_sanitize_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_form_js_flatten_values_json(
        DocumentHandle document, IntPtr optionsJson, out WellfriendBuffer buffer,
        out IntPtr json, out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_sanitize_json(
        DocumentHandle document,
        IntPtr policy,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_canonicalize_json(
        DocumentHandle document,
        long dateEpoch,
        int hasDateEpoch,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_document_redact_terms_json(
        DocumentHandle document,
        IntPtr terms,
        UIntPtr termsLen,
        int strict,
        out WellfriendBuffer buffer,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_feature_report_json(
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_crypto_tamper_test_json(
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int wellfriendpdf_codec_isolation_report_json(
        IntPtr filter,
        byte[] data,
        UIntPtr len,
        IntPtr policy,
        out IntPtr json,
        out IntPtr errorOut);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr wellfriendpdf_version();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint wellfriendpdf_abi_version();

    internal static void ThrowIfError(int status, IntPtr errorOut)
    {
        if (status == 0)
        {
            if (errorOut != IntPtr.Zero)
            {
                wellfriendpdf_error_free(errorOut);
            }
            return;
        }

        var message = errorOut == IntPtr.Zero
            ? $"Wellfriend native call failed with status {status}."
            : Marshal.PtrToStringUTF8(errorOut) ?? $"Wellfriend native call failed with status {status}.";
        if (errorOut != IntPtr.Zero)
        {
            wellfriendpdf_error_free(errorOut);
        }
        throw new WellfriendPdfException(message, status);
    }

    internal static string TakeString(IntPtr value)
    {
        try
        {
            return Marshal.PtrToStringUTF8(value) ?? string.Empty;
        }
        finally
        {
            if (value != IntPtr.Zero)
            {
                wellfriendpdf_string_free(value);
            }
        }
    }

    internal static byte[] TakeBuffer(WellfriendBuffer buffer)
    {
        if (buffer.Data == IntPtr.Zero || buffer.Len == UIntPtr.Zero)
        {
            return Array.Empty<byte>();
        }

        try
        {
            var len = checked((int)buffer.Len.ToUInt64());
            var managed = new byte[len];
            Marshal.Copy(buffer.Data, managed, 0, len);
            return managed;
        }
        finally
        {
            wellfriendpdf_buffer_free(buffer);
        }
    }

    internal static string TakeJson(int status, IntPtr json, IntPtr error)
    {
        ThrowIfError(status, error);
        return TakeString(json);
    }

    internal static WellfriendBinaryResult TakeOutput(int status, WellfriendBuffer buffer, IntPtr json, IntPtr error)
    {
        ThrowIfError(status, error);
        return new WellfriendBinaryResult(TakeBuffer(buffer), TakeString(json));
    }

    internal static IntPtr StringToNativeOrNull(string? value)
    {
        return string.IsNullOrEmpty(value) ? IntPtr.Zero : Marshal.StringToCoTaskMemUTF8(value);
    }
}
