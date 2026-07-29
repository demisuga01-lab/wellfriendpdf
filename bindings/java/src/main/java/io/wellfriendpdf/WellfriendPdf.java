package io.wellfriendpdf;

import java.io.IOException;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemoryLayout;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.net.URISyntaxException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.security.cert.CertificateEncodingException;
import java.security.cert.X509Certificate;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

public final class WellfriendPdf {
    private WellfriendPdf() {
    }

    public static String featureReportJson() {
        return Native.featureReportJson();
    }

    public static String prompt21HistoryReportJson() {
        return Native.prompt21HistoryReportJson();
    }

    public static String cryptoTamperTestJson() {
        return Native.cryptoTamperTestJson();
    }

    public static String codecIsolationReportJson(String filter, byte[] encodedBytes, String policy) {
        Objects.requireNonNull(filter, "filter");
        Objects.requireNonNull(encodedBytes, "encodedBytes");
        return Native.codecIsolationReportJson(filter, encodedBytes, policy);
    }

    /**
     * Validates a caller-supplied RFC 3161 signature timestamp token against
     * the exact CMS SignerInfo.signature octets it claims to timestamp.
     */
    public static String timestampTokenValidationJson(
        byte[] tokenDer,
        byte[] signatureValue,
        String optionsJson
    ) {
        Objects.requireNonNull(tokenDer, "tokenDer");
        Objects.requireNonNull(signatureValue, "signatureValue");
        return Native.timestampTokenValidationJson(tokenDer, signatureValue, optionsJson);
    }

    public static String timestampTokenValidationJson(byte[] tokenDer, byte[] signatureValue) {
        return timestampTokenValidationJson(tokenDer, signatureValue, "{}");
    }

    public static String engineVersion() {
        return Native.engineVersion();
    }

    public static int abiVersion() {
        return Native.abiVersion();
    }

    public static final class WellfriendPdfException extends RuntimeException {
        private final int status;

        WellfriendPdfException(String message, int status) {
            super(message);
            this.status = status;
        }

        public int status() {
            return status;
        }
    }

    /**
     * Owned explicit trust-anchor store. Only certificates added here can
     * become trust anchors when the store is attached to validation options;
     * embedded or intermediate certificates never gain that status implicitly.
     */
    public static final class SignatureTrustStore implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureTrustStore() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_TRUST_STORE_NEW, "trust store");
        }

        public void addTrustAnchorDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_TRUST_STORE_ADD_ANCHOR);
        }

        public void addTrustAnchor(X509Certificate certificate) {
            Objects.requireNonNull(certificate, "certificate");
            try {
                addTrustAnchorDer(certificate.getEncoded());
            } catch (CertificateEncodingException ex) {
                throw new IllegalArgumentException("X509Certificate encoding failed", ex);
            }
        }

        /** Adds every X.509 certificate visible in a caller-selected KeyStore. */
        public void addTrustAnchors(KeyStore keyStore) {
            Objects.requireNonNull(keyStore, "keyStore");
            try {
                var aliases = keyStore.aliases();
                while (aliases.hasMoreElements()) {
                    var certificate = keyStore.getCertificate(aliases.nextElement());
                    if (certificate instanceof X509Certificate x509) {
                        addTrustAnchor(x509);
                    }
                }
            } catch (Exception ex) {
                throw new IllegalArgumentException("KeyStore enumeration failed", ex);
            }
        }

        public void addDistrustedCertificateSha256(String fingerprint) {
            Native.setSignatureValidationString(
                nativeHandle(), fingerprint, Native.SIGNATURE_TRUST_STORE_ADD_DISTRUST,
                "certificate distrust");
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureTrustStore is closed");
            return handle;
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_TRUST_STORE_FREE, "trust store");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Owned untrusted path-building certificate store. */
    public static final class SignatureIntermediateStore implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureIntermediateStore() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_INTERMEDIATE_STORE_NEW, "intermediate store");
        }

        public void addDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_INTERMEDIATE_STORE_ADD_DER);
        }

        public void add(X509Certificate certificate) {
            Objects.requireNonNull(certificate, "certificate");
            try {
                addDer(certificate.getEncoded());
            } catch (CertificateEncodingException ex) {
                throw new IllegalArgumentException("X509Certificate encoding failed", ex);
            }
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureIntermediateStore is closed");
            return handle;
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_INTERMEDIATE_STORE_FREE, "intermediate store");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /**
     * Owned supplied/replayed OCSP and CRL evidence. Adding evidence does not
     * make it good: the native engine authenticates, authorizes, scopes, and
     * freshness-checks it for each validation use.
     */
    public static final class SignatureEvidenceStore implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureEvidenceStore() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_EVIDENCE_STORE_NEW, "evidence store");
        }

        public void addOcspDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_EVIDENCE_STORE_ADD_OCSP);
        }

        public void addCrlDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_EVIDENCE_STORE_ADD_CRL);
        }

        public void importBundleJson(String bundleJson) {
            Native.setSignatureValidationJson(
                nativeHandle(), bundleJson, Native.SIGNATURE_EVIDENCE_STORE_SET_BUNDLE);
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureEvidenceStore is closed");
            return handle;
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_EVIDENCE_STORE_FREE, "evidence store");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Owned bounded AIA/OCSP/CRL retrieval policy. It starts offline. */
    public static final class SignatureRetrievalPolicy implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureRetrievalPolicy() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_RETRIEVAL_POLICY_NEW, "retrieval policy");
        }

        public void setJson(String policyJson) {
            Native.setSignatureValidationJson(
                nativeHandle(), policyJson, Native.SIGNATURE_RETRIEVAL_POLICY_SET_JSON);
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureRetrievalPolicy is closed");
            return handle;
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_RETRIEVAL_POLICY_FREE, "retrieval policy");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Cooperative cancellation source for a signature-validation operation. */
    public static final class SignatureValidationCancellation implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureValidationCancellation() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_CANCELLATION_NEW, "signature validation cancellation");
        }

        public void cancel() {
            Native.cancelSignatureValidation(nativeHandle());
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureValidationCancellation is closed");
            return handle;
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_CANCELLATION_FREE, "signature validation cancellation");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    public record BinaryResult(byte[] bytes, String reportJson) {
        public void writeBytes(Path path) throws IOException {
            Files.write(path, bytes);
        }
    }

    /**
     * Owned Prompt 24 signature-validation configuration. Certificates and
     * evidence are copied by the native layer; this handle never contains a
     * private key. Network retrieval stays disabled unless the caller supplies
     * an explicit bounded retrieval-policy JSON object with {@code enabled}.
     */
    public static final class SignatureValidationOptions implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        public SignatureValidationOptions() {
            this.handle = Native.newSignatureValidationOptions();
        }

        public void addTrustAnchorDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_OPTIONS_ADD_TRUST_ANCHOR);
        }

        /** Copies explicit anchors and distrust entries from an owned store. */
        public void applyTrustStore(SignatureTrustStore trustStore) {
            Objects.requireNonNull(trustStore, "trustStore");
            Native.applySignatureComponent(
                nativeHandle(), trustStore.nativeHandle(), Native.SIGNATURE_OPTIONS_APPLY_TRUST_STORE,
                "trust store");
        }

        public void addTrustAnchor(X509Certificate certificate) {
            addTrustAnchorDer(encodedCertificate(certificate));
        }

        public void addIntermediateDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_OPTIONS_ADD_INTERMEDIATE);
        }

        /** Copies untrusted path-building candidates from an owned store. */
        public void applyIntermediateStore(SignatureIntermediateStore intermediateStore) {
            Objects.requireNonNull(intermediateStore, "intermediateStore");
            Native.applySignatureComponent(
                nativeHandle(), intermediateStore.nativeHandle(),
                Native.SIGNATURE_OPTIONS_APPLY_INTERMEDIATE_STORE, "intermediate store");
        }

        public void addIntermediate(X509Certificate certificate) {
            addIntermediateDer(encodedCertificate(certificate));
        }

        /** Adds a SHA-256 certificate fingerprint to the selected-path deny list. */
        public void addDistrustedCertificateSha256(String fingerprint) {
            Native.setSignatureValidationString(
                nativeHandle(),
                fingerprint,
                Native.SIGNATURE_OPTIONS_ADD_DISTRUST,
                "certificate distrust"
            );
        }

        public void addOcspDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_OPTIONS_ADD_OCSP);
        }

        public void addCrlDer(byte[] der) {
            Native.addSignatureValidationDer(nativeHandle(), der, Native.SIGNATURE_OPTIONS_ADD_CRL);
        }

        /** Copies supplied/replayed evidence without making it trusted. */
        public void applyEvidenceStore(SignatureEvidenceStore evidenceStore) {
            Objects.requireNonNull(evidenceStore, "evidenceStore");
            Native.applySignatureComponent(
                nativeHandle(), evidenceStore.nativeHandle(), Native.SIGNATURE_OPTIONS_APPLY_EVIDENCE_STORE,
                "evidence store");
        }

        public void setValidationTimeUnix(long validationTimeUnix) {
            Native.setSignatureValidationTime(nativeHandle(), validationTimeUnix);
        }

        public void useSystemValidationTime() {
            Native.clearSignatureValidationTime(nativeHandle());
        }

        /**
         * Revocation mode: 0 = not checked, 1 = offline strict, 2 = offline best effort,
         * 3 = online strict, 4 = online best effort. Online modes still require an explicit
         * bounded retrieval policy and never enable network access on their own.
         */
        public void setRevocationMode(int mode) {
            if (mode < 0 || mode > 4) {
                throw new IllegalArgumentException("unsupported revocation mode");
            }
            Native.setSignatureValidationMode(nativeHandle(), mode);
        }

        public void setPathLimits(long maxChainDepth, long maxPathCandidates) {
            if (maxChainDepth <= 0 || maxPathCandidates <= 0) {
                throw new IllegalArgumentException("path limits must be positive");
            }
            Native.setSignatureValidationPathLimits(nativeHandle(), maxChainDepth, maxPathCandidates);
        }

        /**
         * Applies the native RetrievalPolicy JSON schema. Passing a policy does
         * not enable online access unless its {@code enabled} field is true.
         */
        public void setRetrievalPolicyJson(String policyJson) {
            Native.setSignatureValidationJson(nativeHandle(), policyJson, Native.SIGNATURE_OPTIONS_SET_RETRIEVAL_POLICY);
        }

        /** Copies a bounded retrieval policy; online access remains opt-in. */
        public void applyRetrievalPolicy(SignatureRetrievalPolicy policy) {
            Objects.requireNonNull(policy, "policy");
            Native.applySignatureComponent(
                nativeHandle(), policy.nativeHandle(), Native.SIGNATURE_OPTIONS_APPLY_RETRIEVAL_POLICY,
                "retrieval policy");
        }

        /** Attaches a shared cooperative cancellation token. */
        public void setCancellation(SignatureValidationCancellation cancellation) {
            Objects.requireNonNull(cancellation, "cancellation");
            Native.applySignatureComponent(
                nativeHandle(), cancellation.nativeHandle(), Native.SIGNATURE_OPTIONS_SET_CANCELLATION,
                "signature validation cancellation");
        }

        /** Applies the native SignatureAlgorithmPolicy JSON schema. */
        public void setAlgorithmPolicyJson(String policyJson) {
            Native.setSignatureValidationJson(nativeHandle(), policyJson, Native.SIGNATURE_OPTIONS_SET_ALGORITHM_POLICY);
        }

        /** Imports a replay bundle; every evidence item is revalidated at use time. */
        public void setEvidenceBundleJson(String bundleJson) {
            Native.setSignatureValidationJson(nativeHandle(), bundleJson, Native.SIGNATURE_OPTIONS_SET_EVIDENCE_BUNDLE);
        }

        private MemorySegment nativeHandle() {
            if (closed || Native.isNull(handle)) {
                throw new IllegalStateException("SignatureValidationOptions is closed");
            }
            return handle;
        }

        private static byte[] encodedCertificate(X509Certificate certificate) {
            Objects.requireNonNull(certificate, "certificate");
            try {
                return certificate.getEncoded();
            } catch (CertificateEncodingException ex) {
                throw new IllegalArgumentException("X509Certificate encoding failed", ex);
            }
        }

        @Override
        public void close() {
            if (closed) return;
            Native.freeSignatureValidationOptions(handle);
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    public static final class Document implements AutoCloseable {
        private MemorySegment handle;
        private boolean closed;

        private Document(MemorySegment handle) {
            this.handle = handle;
        }

        public static Document open(Path path) throws IOException {
            return open(path, null);
        }

        public static Document open(Path path, String password) throws IOException {
            Objects.requireNonNull(path, "path");
            return open(Files.readAllBytes(path), password);
        }

        public static Document open(byte[] bytes) {
            return open(bytes, null);
        }

        public static Document open(byte[] bytes, String password) {
            Objects.requireNonNull(bytes, "bytes");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment passwordPtr = MemorySegment.NULL;
                long passwordLen = 0;
                if (password != null) {
                    byte[] passwordBytes = password.getBytes(StandardCharsets.UTF_8);
                    passwordLen = passwordBytes.length;
                    passwordPtr = arena.allocate(Math.max(passwordBytes.length, 1));
                    if (passwordBytes.length > 0) {
                        passwordPtr.copyFrom(MemorySegment.ofArray(passwordBytes));
                    }
                }
                MemorySegment handle = (MemorySegment) Native.OPEN_WITH_PASSWORD.invokeExact(
                    data,
                    (long) bytes.length,
                    passwordPtr,
                    passwordLen,
                    err
                );
                if (Native.isNull(handle)) {
                    Native.throwError(2, err);
                }
                return new Document(handle);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend native open failed", ex);
            }
        }

        public static Document openPubSec(byte[] bytes, byte[] certificate, byte[] privateKey) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(certificate, "certificate");
            Objects.requireNonNull(privateKey, "privateKey");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment cert = arena.allocate(certificate.length);
                cert.copyFrom(MemorySegment.ofArray(certificate));
                MemorySegment key = arena.allocate(privateKey.length);
                key.copyFrom(MemorySegment.ofArray(privateKey));
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment handle = (MemorySegment) Native.OPEN_PUBSEC.invokeExact(
                    data,
                    (long) bytes.length,
                    cert,
                    (long) certificate.length,
                    key,
                    (long) privateKey.length,
                    err
                );
                if (Native.isNull(handle)) {
                    Native.throwError(2, err);
                }
                return new Document(handle);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend native PubSec open failed", ex);
            }
        }

        public static Document openPubSecPfx(byte[] bytes, byte[] pfx, byte[] password) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(pfx, "pfx");
            byte[] effectivePassword = password == null ? new byte[0] : password;
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment pfxBytes = arena.allocate(pfx.length);
                pfxBytes.copyFrom(MemorySegment.ofArray(pfx));
                MemorySegment passwordBytes = MemorySegment.NULL;
                if (effectivePassword.length > 0) {
                    passwordBytes = arena.allocate(effectivePassword.length);
                    passwordBytes.copyFrom(MemorySegment.ofArray(effectivePassword));
                }
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment handle = (MemorySegment) Native.OPEN_PUBSEC_PFX.invokeExact(
                    data,
                    (long) bytes.length,
                    pfxBytes,
                    (long) pfx.length,
                    passwordBytes,
                    (long) effectivePassword.length,
                    err
                );
                if (Native.isNull(handle)) {
                    Native.throwError(2, err);
                }
                return new Document(handle);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend native PubSec PFX open failed", ex);
            }
        }

        public int pageCount() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment count = arena.allocate(ValueLayout.JAVA_LONG);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PAGE_COUNT.invokeExact(handle, count, err);
                Native.throwError(status, err);
                return Math.toIntExact(count.get(ValueLayout.JAVA_LONG, 0));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend page_count failed", ex);
            }
        }

        public Page page(int pageNumber) {
            if (pageNumber < 1 || pageNumber > pageCount()) {
                throw new IndexOutOfBoundsException("Page numbers are 1-based");
            }
            return new Page(this, pageNumber);
        }

        public List<Page> pages() {
            int count = pageCount();
            var pages = new ArrayList<Page>(count);
            for (int idx = 1; idx <= count; idx++) {
                pages.add(new Page(this, idx));
            }
            return List.copyOf(pages);
        }

        public String extractText(int pageNumber) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.EXTRACT_TEXT.invokeExact(handle, (long) pageNumber, textOut, err);
                Native.throwError(status, err);
                return Native.takeString(textOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend extract_text failed", ex);
            }
        }

        public String parseJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PARSE_JSON.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend parse_json failed", ex);
            }
        }

        public String securityReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SECURITY_REPORT, "security_report");
        }

        public String parserReportJson(String mode) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.PARSER_REPORT, mode, "parser_report");
        }

        public String colorReportJson(String profile) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.COLOR_REPORT, profile, "color_report");
        }

        public String validateJson(String profile) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.VALIDATE, profile, "validate");
        }

        /** Clause-mapped PDF/A envelope; {@code target} may be null for detection. */
        public String validatePdfaStandardsJson(String target) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PDFA_STANDARDS, target, "pdfa_standards_validation");
        }

        public String validatePdfaStandardsJson() {
            return validatePdfaStandardsJson(null);
        }

        /** Clause-mapped PDF/UA envelope; {@code target} may be null for detection. */
        public String validatePdfuaStandardsJson(String target) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PDFUA_STANDARDS, target, "pdfua_standards_validation");
        }

        public String validatePdfuaStandardsJson() {
            return validatePdfuaStandardsJson(null);
        }

        /** Clause-mapped PDF/X envelope; {@code target} may be null for detection. */
        public String validatePdfxStandardsJson(String target) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PDFX_STANDARDS, target, "pdfx_standards_validation");
        }

        public String validatePdfxStandardsJson() {
            return validatePdfxStandardsJson(null);
        }

        /** Combined standards envelope including cross-profile conflicts. */
        public String validateAllStandardsJson(String target) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.STANDARDS_ALL, target, "standards_all_validation");
        }

        public String validateAllStandardsJson() {
            return validateAllStandardsJson(null);
        }

        /** Native post-signature cryptographic validation for this document. */
        public String signatureReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SIGNATURE_REPORT, "signature_report");
        }

        /**
         * Plans a real append-only local PEM signature. PEM material is copied
         * for the native call and never logged or retained by this Java object.
         */
        public String incrementalSigningPlanJson(
            String keyPem, String certPem, long placeholderSize, int certify
        ) {
            ensureOpen();
            return Native.incrementalSigningPlan(handle, keyPem, certPem, placeholderSize, certify);
        }

        public String incrementalSigningPlanJson(String keyPem, String certPem) {
            return incrementalSigningPlanJson(keyPem, certPem, 16 * 1024L, 0);
        }

        /**
         * Produces a true append-only signed PDF. Native code validates the
         * result after reopening it; returned bytes and JSON are both owned by
         * the Java result object after native buffers are freed.
         */
        public BinaryResult signIncremental(
            String keyPem,
            String certPem,
            long placeholderSize,
            int certify,
            String fieldName,
            String reason
        ) {
            ensureOpen();
            return Native.signIncremental(
                handle, keyPem, certPem, placeholderSize, certify, fieldName, reason);
        }

        public BinaryResult signIncremental(String keyPem, String certPem) {
            return signIncremental(keyPem, certPem, 16 * 1024L, 0, null, null);
        }

        /** Existing combined edit-policy report, named for DocMDP callers. */
        public String docMdpPermissionReportJson(String operation) {
            return editPolicyReportJson(operation);
        }

        public String docMdpPermissionReportJson() {
            return docMdpPermissionReportJson("form_value_update");
        }

        /** Existing combined edit-policy report, named for FieldMDP callers. */
        public String fieldMdpPermissionReportJson(String operation) {
            return editPolicyReportJson(operation);
        }

        public String fieldMdpPermissionReportJson() {
            return fieldMdpPermissionReportJson("form_value_update");
        }

        public String formsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.FORMS_REPORT, "forms_report");
        }

        public String xfaReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.XFA_REPORT, "xfa_report");
        }

        public String xfaExtractJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.XFA_EXTRACT, "xfa_extract");
        }

        public String xfaScriptReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.XFA_SCRIPT_REPORT, "xfa_script_report");
        }

        public String xfaSecurityReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.XFA_SECURITY_REPORT, "xfa_security_report");
        }

        public String xfaRuntimeReportJson(String scriptPolicy, boolean executeEvents) {
            ensureOpen();
            return Native.xfaRuntimeReport(handle, scriptPolicy, executeEvents);
        }

        public String annotationsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ANNOTATIONS_REPORT, "annotations_report");
        }

        public String richMediaReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.RICH_MEDIA_REPORT, "rich_media_report");
        }

        public String prompt17ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT17_REPORT, "prompt17_report");
        }

        public String prompt18ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT18_REPORT, "prompt18_report");
        }

        public String prompt18bReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT18B_REPORT, "prompt18b_report");
        }

        public String formJavaScriptReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.FORM_JS_REPORT, "form_js_report");
        }

        public String formActionGraphJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.FORM_ACTION_GRAPH, "form_action_graph");
        }

        public String interactiveDataReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.INTERACTIVE_DATA_REPORT, "interactive_data_report");
        }

        public String wordPaginationAuditJson(String layout) {
            ensureOpen();
            Objects.requireNonNull(layout, "layout");
            return Native.documentStringReport(handle, Native.WORD_PAGINATION_AUDIT, layout, "word_pagination_audit");
        }

        public String prompt19ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT19_REPORT, "prompt19_report");
        }

        public String prompt20ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT20_REPORT, "prompt20_report");
        }

        public String prompt20bReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT20B_REPORT, "prompt20b_report");
        }

        public String prompt31ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT31_REPORT, "prompt31_report");
        }

        public String prompt32ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT32_REPORT, "prompt32_report");
        }

        public String prompt33ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT33_REPORT, "prompt33_report");
        }

        public String prompt21ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT21_REPORT, "prompt21_report");
        }

        public String prompt21RasterVectorReportJson(long page, String optionsJson) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.prompt21RasterVectorReport(handle, page, optionsJson);
        }

        public String prompt21FontReconstructionReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.PROMPT21_FONT_RECONSTRUCTION_REPORT, "prompt21_font_reconstruction_report");
        }

        public String prompt21ObjectStreamReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.PROMPT21_OBJECT_STREAM_REPORT, "prompt21_object_stream_report");
        }

        public BinaryResult prompt21PackObjectStreams() {
            ensureOpen();
            return Native.documentOutput(handle, Native.PROMPT21_PACK_OBJECT_STREAMS, "prompt21_pack_object_streams");
        }

        public String prompt22ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT22_REPORT, "prompt22_report");
        }

        public String prompt23ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT23_REPORT, "prompt23_report");
        }

        public String signatureReportWithOptionsJson(String optionsJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.SIGNATURE_REPORT_WITH_OPTIONS, optionsJson, "signature_report_with_options");
        }

        public String signatureValidationWithEvidenceJson(String optionsJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.SIGNATURE_VALIDATION_WITH_EVIDENCE, optionsJson,
                "signature_validation_with_evidence");
        }

        public String signatureValidationReport(SignatureValidationOptions options) {
            ensureOpen();
            Objects.requireNonNull(options, "options");
            return Native.documentSignatureOptionsReport(
                handle, options.nativeHandle(), Native.SIGNATURE_REPORT_WITH_OPTIONS_HANDLE,
                "signature_validation");
        }

        public String signatureValidationWithEvidence(SignatureValidationOptions options) {
            ensureOpen();
            Objects.requireNonNull(options, "options");
            return Native.documentSignatureOptionsReport(
                handle, options.nativeHandle(), Native.SIGNATURE_VALIDATION_WITH_EVIDENCE_HANDLE,
                "signature_validation_with_evidence");
        }

        public String writerDeterminismAuditJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.WRITER_DETERMINISM_AUDIT, "writer_determinism_audit");
        }

        public String writerExternalDiffJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.WRITER_EXTERNAL_DIFF, "writer_external_diff");
        }

        public String writerCloseoutReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.WRITER_CLOSEOUT_REPORT, "writer_closeout_report");
        }

        public String pubsecReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PUBSEC_REPORT, "pubsec_report");
        }

        public String aesGcmReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.AES_GCM_REPORT, "aes_gcm_report");
        }

        public String pdfMacReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PDF_MAC_REPORT, "pdf_mac_report");
        }

        public String pdfMacVerifyJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PDF_MAC_VERIFY, "pdf_mac_verify");
        }

        public BinaryResult pdfMacCreate() {
            ensureOpen();
            return Native.documentOutput(handle, Native.PDF_MAC_CREATE, "pdf_mac_create");
        }

        public BinaryResult prompt22Optimize(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.PROMPT22_OPTIMIZE, optionsJson, "prompt22_optimize");
        }

        public String prompt20bTextRangeAnalyzeJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.prompt20bTextRangeAnalyze(handle, page);
        }

        public BinaryResult editTextRange(String requestJson) {
            ensureOpen();
            return Native.prompt20bTextRangeEdit(handle, requestJson);
        }

        public String prompt31ProvenanceJson(long page, String sourceText, String replacementText) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.prompt31Provenance(handle, page, sourceText, replacementText);
        }

        public String prompt31EditEligibilityJson(String requestJson) {
            ensureOpen();
            return Native.prompt31EditEligibility(handle, requestJson);
        }

        public BinaryResult prompt31OperatorTextEdit(String requestJson) {
            ensureOpen();
            return Native.prompt31OperatorTextEdit(handle, requestJson);
        }

        public String prompt31PathProvenanceJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.prompt31PathProvenance(handle, page);
        }

        public BinaryResult prompt31PathEdit(
            long page, String stableId, String operationJson, String optionsJson
        ) {
            ensureOpen();
            return Native.prompt31PathEdit(handle, page, stableId, operationJson, optionsJson);
        }

        public String prompt32SceneReportJson(String pagesJson) {
            ensureOpen();
            return Native.prompt32SceneReport(handle, pagesJson);
        }

        public String prompt32SceneSelectJson(String requestJson) {
            ensureOpen();
            return Native.prompt32SceneSelect(handle, requestJson);
        }

        public String prompt32TransactionPlanJson(String requestJson) {
            ensureOpen();
            return Native.prompt32TransactionPlan(handle, requestJson);
        }

        public BinaryResult prompt32TransactionApply(String requestJson) {
            ensureOpen();
            return Native.prompt32TransactionApply(handle, requestJson);
        }

        public String prompt32TextMapJson(String text, String direction) {
            ensureOpen();
            return Native.prompt32TextMap(handle, text, direction);
        }

        public String prompt32ShapeTextJson(String text, String direction) {
            ensureOpen();
            return Native.prompt32ShapeText(handle, text, direction);
        }

        public String prompt32FontSubsetPlanJson(String text, String direction, String policy) {
            ensureOpen();
            return Native.prompt32FontSubsetPlan(handle, text, direction, policy);
        }

        public String prompt32FontSubstitutionReportJson(String requestedFamily, String text, String policy) {
            ensureOpen();
            return Native.prompt32FontSubstitutionReport(handle, requestedFamily, text, policy);
        }

        public String prompt33LayoutAnalyzeJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_LAYOUT_ANALYZE, requestJson, "prompt33_layout_analyze");
        }

        public String prompt33SemanticLayoutJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT33_SEMANTIC_LAYOUT, "prompt33_semantic_layout");
        }

        public String prompt33ReadingOrderReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT33_READING_ORDER, "prompt33_reading_order_report");
        }

        public String prompt33FlowGraphReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT33_FLOW_GRAPH, "prompt33_flow_graph_report");
        }

        public String prompt33ReflowPreviewJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_REFLOW_PREVIEW, requestJson, "prompt33_reflow_preview");
        }

        public String prompt33OverflowReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_OVERFLOW_REPORT, requestJson, "prompt33_overflow_report");
        }

        public String prompt33ConstraintsReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_CONSTRAINTS_REPORT, requestJson, "prompt33_constraints_report");
        }

        public String prompt33ConfidenceReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_CONFIDENCE_REPORT, requestJson, "prompt33_confidence_report");
        }

        /**
         * Validates explicit Prompt 33 output bytes against this immutable
         * source document. The caller retains ownership of {@code outputPdf}.
         */
        public String prompt33ValidateReflowOutputJson(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.prompt33ValidateReflowOutput(
                handle, outputPdf, requestJson, "prompt33_validate_reflow_output");
        }

        public BinaryResult prompt33ReflowRegion(String requestJson) {
            ensureOpen();
            return Native.prompt33RequestOutput(handle, Native.PROMPT33_REFLOW_REGION, requestJson, "prompt33_reflow_region");
        }

        public BinaryResult prompt33ReflowDocument(String requestJson) {
            ensureOpen();
            return Native.prompt33RequestOutput(handle, Native.PROMPT33_REFLOW_DOCUMENT, requestJson, "prompt33_reflow_document");
        }

        /**
         * Replays the specified Prompt 33 operation against this immutable
         * preimage, verifies {@code outputPdf}, and executes its canonical
         * transaction undo. A stale output buffer is rejected.
         */
        public BinaryResult prompt33UndoReflow(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.prompt33UndoReflow(handle, outputPdf, requestJson, "prompt33_undo_reflow");
        }

        public String prompt33ReflowApproveStructureJson(String correctionJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_APPROVE_STRUCTURE, correctionJson, "prompt33_reflow_approve_structure");
        }

        public String prompt33ReflowOperationReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.PROMPT33_OPERATION_REPORT, requestJson, "prompt33_reflow_operation_report");
        }

        public String prompt34ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT34_REPORT, "prompt34_report");
        }

        public String prompt34AnalyzeJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT34_ANALYZE, "prompt34_analyze");
        }

        public String prompt34PlanJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.PROMPT34_PLAN, requestJson, "prompt34_plan");
        }

        public BinaryResult prompt34Apply(String requestJson) {
            ensureOpen();
            return Native.prompt33RequestOutput(handle, Native.PROMPT34_APPLY, requestJson, "prompt34_apply");
        }

        public BinaryResult prompt34Undo(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.prompt34Undo(handle, outputPdf, requestJson, "prompt34_undo");
        }

        public String prompt35ReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT35_REPORT, "prompt35_report");
        }

        public String prompt35AnalyzeJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PROMPT35_ANALYZE, "prompt35_analyze");
        }

        public String prompt35PlanJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.PROMPT35_PLAN, requestJson, "prompt35_plan");
        }

        public BinaryResult prompt35Apply(String requestJson) {
            ensureOpen();
            return Native.prompt33RequestOutput(handle, Native.PROMPT35_APPLY, requestJson, "prompt35_apply");
        }

        public BinaryResult prompt35Undo(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.prompt35Undo(handle, outputPdf, requestJson, "prompt35_undo");
        }

        public String prompt35VerifyResidualJson(String termsJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.PROMPT35_VERIFY_RESIDUAL, termsJson, "prompt35_verify_residual");
        }

        public String prompt20VectorListJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.prompt20VectorList(handle, page);
        }

        public BinaryResult prompt20TextEdit(
            long page, String oldText, String newText, String mode, String optionsJson
        ) {
            ensureOpen();
            return Native.prompt20TextEdit(handle, page, oldText, newText, mode, optionsJson);
        }

        public BinaryResult prompt20VectorEdit(
            long page, String stableId, String operationJson, String optionsJson
        ) {
            ensureOpen();
            return Native.prompt20VectorEdit(handle, page, stableId, operationJson, optionsJson);
        }

        public BinaryResult prompt20InkFit(
            long page, long annotationIndex, String optionsJson, boolean signaturePolicyOverride
        ) {
            ensureOpen();
            return Native.prompt20InkFit(
                handle, page, annotationIndex, optionsJson, signaturePolicyOverride);
        }

        public String associatedFilesReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ASSOCIATED_FILES_REPORT, "associated_files_report");
        }

        public String editPolicyReportJson(String operation) {
            ensureOpen();
            Objects.requireNonNull(operation, "operation");
            return Native.documentStringReport(handle, Native.EDIT_POLICY_REPORT, operation, "edit_policy_report");
        }

        public String signaturePreservingFormPlanJson(
            String fieldName,
            String value,
            String optionsJson
        ) {
            ensureOpen();
            return Native.signaturePreservingFormPlan(handle, fieldName, value, optionsJson);
        }

        public BinaryResult signaturePreservingFormEdit(
            String fieldName,
            String value,
            String optionsJson,
            boolean explicitInvalidationOverride
        ) {
            ensureOpen();
            return Native.signaturePreservingFormEdit(
                handle, fieldName, value, optionsJson, explicitInvalidationOverride);
        }

        public String annotationAppearanceReportJson(String optionsJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.ANNOTATION_APPEARANCE_REPORT, optionsJson, "annotation_appearance_report");
        }

        public String nonaxisRedactionPlanJson(String optionsJson) {
            ensureOpen();
            Objects.requireNonNull(optionsJson, "optionsJson");
            return Native.documentStringReport(handle, Native.NONAXIS_REDACTION_PLAN, optionsJson, "nonaxis_redaction_plan");
        }

        public String pagesReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.PAGES_REPORT, "pages_report");
        }

        public String interactiveReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.INTERACTIVE_REPORT, "interactive_report");
        }

        public String chunksJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.CHUNKS, "chunks");
        }

        public String advancedChunksJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ADVANCED_CHUNKS, "advanced_chunks");
        }

        public String semanticBundleJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SEMANTIC_BUNDLE, "semantic_bundle");
        }

        public String semanticSearchJson(String query) {
            ensureOpen();
            Objects.requireNonNull(query, "query");
            if (query.isBlank()) {
                throw new IllegalArgumentException("query must not be blank");
            }
            return Native.documentStringReport(handle, Native.SEMANTIC_SEARCH, query, "semantic_search");
        }

        public BinaryResult xfaRender(String scriptPolicy, boolean executeEvents, int dpi) {
            ensureOpen();
            return Native.xfaRender(handle, scriptPolicy, executeEvents, dpi);
        }

        public BinaryResult xfaFlatten(String mode) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.XFA_FLATTEN, mode, "xfa_flatten");
        }

        public BinaryResult xfaSanitize(String mode) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.XFA_SANITIZE, mode, "xfa_sanitize");
        }

        public BinaryResult annotationXfdfExport() {
            ensureOpen();
            return Native.documentOutput(handle, Native.ANNOTATION_XFDF_EXPORT, "annotation_xfdf_export");
        }

        public BinaryResult annotationXfdfImport(byte[] xfdf, String optionsJson) {
            ensureOpen();
            return Native.annotationXfdfImport(handle, xfdf, optionsJson);
        }

        public BinaryResult annotationAppearanceGenerate(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.ANNOTATION_APPEARANCE_GENERATE, optionsJson, "annotation_appearance_generate");
        }

        public BinaryResult richMediaSanitize(String mode, String customJson) {
            ensureOpen();
            return Native.documentTwoStringOutput(handle, Native.RICH_MEDIA_SANITIZE, mode, customJson, "rich_media_sanitize");
        }

        public BinaryResult richMediaFlattenPoster() {
            ensureOpen();
            return Native.documentOutput(handle, Native.RICH_MEDIA_FLATTEN_POSTER, "rich_media_flatten_poster");
        }

        public BinaryResult redactImageNonaxis(String optionsJson) {
            ensureOpen();
            Objects.requireNonNull(optionsJson, "optionsJson");
            return Native.documentStringOutput(handle, Native.NONAXIS_REDACTION_APPLY, optionsJson, "nonaxis_redaction_apply");
        }

        public BinaryResult redactImageMask(String optionsJson) {
            ensureOpen();
            Objects.requireNonNull(optionsJson, "optionsJson");
            return Native.documentStringOutput(handle, Native.REDACT_IMAGE_MASK, optionsJson, "redact_image_mask");
        }

        public BinaryResult redactInlineImage(String optionsJson) {
            ensureOpen();
            Objects.requireNonNull(optionsJson, "optionsJson");
            return Native.documentStringOutput(handle, Native.REDACT_INLINE_IMAGE, optionsJson, "redact_inline_image");
        }

        public BinaryResult associatedFileAdd(byte[] payload, String optionsJson) {
            ensureOpen();
            return Native.associatedFileAdd(handle, payload, optionsJson);
        }

        public BinaryResult associatedFileUpdateOwner(byte[] payload, String optionsJson) {
            ensureOpen();
            return Native.associatedFilePayloadMutation(
                handle, Native.ASSOCIATED_FILES_UPDATE_OWNER, payload, optionsJson,
                "associated_files_update_owner");
        }

        public BinaryResult associatedFileRemoveOwner(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(
                handle, Native.ASSOCIATED_FILES_REMOVE_OWNER, optionsJson,
                "associated_files_remove_owner");
        }

        public BinaryResult incrementalFormEdit(
            String fieldName, String value, boolean signaturePolicyOverride
        ) {
            ensureOpen();
            return Native.incrementalFormEdit(handle, fieldName, value, signaturePolicyOverride);
        }

        public BinaryResult incrementalAnnotationEdit(
            String optionsJson, boolean signaturePolicyOverride
        ) {
            ensureOpen();
            return Native.documentPolicyOutput(
                handle, Native.INCREMENTAL_ANNOTATION_EDIT, optionsJson,
                signaturePolicyOverride, "incremental_annotation_edit");
        }

        public BinaryResult incrementalPagePropertyEdit(
            String optionsJson, boolean signaturePolicyOverride
        ) {
            ensureOpen();
            return Native.documentPolicyOutput(
                handle, Native.INCREMENTAL_PAGE_PROPERTY_EDIT, optionsJson,
                signaturePolicyOverride, "incremental_page_property_edit");
        }

        public BinaryResult associatedFileExtract(String stableId) {
            ensureOpen();
            Objects.requireNonNull(stableId, "stableId");
            return Native.documentStringOutput(handle, Native.ASSOCIATED_FILES_EXTRACT, stableId, "associated_files_extract");
        }

        public BinaryResult associatedFilesRemove(String stableIdsJson) {
            ensureOpen();
            Objects.requireNonNull(stableIdsJson, "stableIdsJson");
            return Native.documentStringOutput(handle, Native.ASSOCIATED_FILES_REMOVE, stableIdsJson, "associated_files_remove");
        }

        public BinaryResult associatedFilesSanitize(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.ASSOCIATED_FILES_SANITIZE, optionsJson, "associated_files_sanitize");
        }

        public BinaryResult formJavaScriptSanitize(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.FORM_JS_SANITIZE, optionsJson, "form_js_sanitize");
        }

        public BinaryResult formJavaScriptFlattenValues(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.FORM_JS_FLATTEN_VALUES, optionsJson, "form_js_flatten_values");
        }

        public BinaryResult sanitize(String policy) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.SANITIZE, policy, "sanitize");
        }

        public BinaryResult canonicalize(Long dateEpoch) {
            ensureOpen();
            return Native.canonicalize(handle, dateEpoch);
        }

        public BinaryResult redactTerms(List<String> terms, boolean strict) {
            ensureOpen();
            return Native.redactTerms(handle, terms, strict);
        }

        public byte[] toDocx(boolean includeImages) {
            ensureOpen();
            return Native.documentToBytes(handle, Native.TO_DOCX, includeImages ? 1 : 0);
        }

        public byte[] toDocx(String layout, boolean includeImages) {
            ensureOpen();
            Objects.requireNonNull(layout, "layout");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment layoutPtr = arena.allocateFrom(layout);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.TO_DOCX_WITH_LAYOUT.invokeExact(
                    handle, includeImages ? 1 : 0, layoutPtr, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend to_docx_with_layout failed", ex);
            }
        }

        public byte[] toXlsx(String layout) {
            ensureOpen();
            Objects.requireNonNull(layout, "layout");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment layoutPtr = arena.allocateFrom(layout);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.TO_XLSX.invokeExact(handle, layoutPtr, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend to_xlsx failed", ex);
            }
        }

        public byte[] toPptx(boolean includeImages) {
            ensureOpen();
            return Native.documentToBytes(handle, Native.TO_PPTX, includeImages ? 1 : 0);
        }

        public byte[] pubsecEncryptPdf(byte[] recipientCertificate) {
            ensureOpen();
            return Native.pubsecEncrypt(handle, recipientCertificate);
        }

        @Override
        public void close() {
            if (!closed) {
                try {
                    Native.FREE_DOC.invokeExact(handle);
                } catch (Throwable ex) {
                    throw new IllegalStateException("Wellfriend document free failed", ex);
                } finally {
                    closed = true;
                    handle = MemorySegment.NULL;
                }
            }
        }

        private void ensureOpen() {
            if (closed) {
                throw new IllegalStateException("Document is closed");
            }
        }
    }

    public record Page(Document document, int number) {
        public String text() {
            return document.extractText(number);
        }
    }

    public static final class Office {
        private Office() {
        }

        public static byte[] docxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.DOCX_TO_PDF);
        }

        public static byte[] xlsxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.XLSX_TO_PDF);
        }

        public static byte[] pptxToPdf(byte[] bytes) {
            return Native.officeToPdf(bytes, Native.PPTX_TO_PDF);
        }

        public static String prompt22InspectJson(byte[] bytes, String format) {
            return Native.prompt22OfficeInspect(bytes, format);
        }

        public static BinaryResult prompt22ToPdf(byte[] bytes, String format) {
            return Native.prompt22OfficeToPdf(bytes, format);
        }
    }

    private static final class Native {
        private static final Linker LINKER = Linker.nativeLinker();
        private static final Arena LOOKUP_ARENA = Arena.ofAuto();
        private static final SymbolLookup LOOKUP = loadLibrary();
        private static final MemoryLayout BUFFER_LAYOUT = MemoryLayout.structLayout(
            ValueLayout.ADDRESS.withName("data"),
            ValueLayout.JAVA_LONG.withName("len")
        );
        private static final long BUFFER_LEN_OFFSET = ValueLayout.ADDRESS.byteSize();
        private static final long ADDRESS_SIZE = ValueLayout.ADDRESS.byteSize();

        private static final MethodHandle OPEN = downcall(
            "wellfriendpdf_document_open_from_bytes",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle OPEN_WITH_PASSWORD = downcall(
            "wellfriendpdf_document_open_from_bytes_with_password",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle OPEN_PUBSEC = downcall(
            "wellfriendpdf_document_open_pubsec_from_bytes",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle OPEN_PUBSEC_PFX = downcall(
            "wellfriendpdf_document_open_pubsec_pfx_from_bytes",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FREE_DOC = downcall(
            "wellfriendpdf_document_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle STRING_FREE = downcall(
            "wellfriendpdf_string_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle ERROR_FREE = downcall(
            "wellfriendpdf_error_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle BUFFER_FREE = downcall(
            "wellfriendpdf_buffer_free",
            FunctionDescriptor.ofVoid(BUFFER_LAYOUT)
        );
        private static final MethodHandle PAGE_COUNT = downcall(
            "wellfriendpdf_document_page_count",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EXTRACT_TEXT = downcall(
            "wellfriendpdf_document_extract_text",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PARSE_JSON = downcall(
            "wellfriendpdf_document_parse_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_XLSX = downcall(
            "wellfriendpdf_document_to_xlsx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_PPTX = downcall(
            "wellfriendpdf_document_to_pptx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_DOCX = downcall(
            "wellfriendpdf_document_to_docx",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TO_DOCX_WITH_LAYOUT = downcall(
            "wellfriendpdf_document_to_docx_with_layout",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PUBSEC_ENCRYPT = downcall(
            "wellfriendpdf_document_pubsec_encrypt_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DOCX_TO_PDF = downcall(
            "wellfriendpdf_docx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle XLSX_TO_PDF = downcall(
            "wellfriendpdf_xlsx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PPTX_TO_PDF = downcall(
            "wellfriendpdf_pptx_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SECURITY_REPORT = documentReport("wellfriendpdf_document_security_report_json");
        private static final MethodHandle PARSER_REPORT = documentStringReport("wellfriendpdf_document_parser_report_json");
        private static final MethodHandle COLOR_REPORT = documentStringReport("wellfriendpdf_document_color_report_json");
        private static final MethodHandle VALIDATE = documentStringReport("wellfriendpdf_document_validate_json");
        private static final MethodHandle PDFA_STANDARDS =
            documentStringReport("wellfriendpdf_document_pdfa_standards_json");
        private static final MethodHandle PDFUA_STANDARDS =
            documentStringReport("wellfriendpdf_document_pdfua_standards_json");
        private static final MethodHandle PDFX_STANDARDS =
            documentStringReport("wellfriendpdf_document_pdfx_standards_json");
        private static final MethodHandle STANDARDS_ALL =
            documentStringReport("wellfriendpdf_document_standards_all_json");
        private static final MethodHandle SIGNATURE_REPORT =
            documentReport("wellfriendpdf_document_signatures_json");
        private static final MethodHandle INCREMENTAL_SIGNING_PLAN = downcall(
            "wellfriendpdf_document_sign_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle INCREMENTAL_SIGN = downcall(
            "wellfriendpdf_document_sign_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FORMS_REPORT = documentReport("wellfriendpdf_document_forms_report_json");
        private static final MethodHandle XFA_REPORT = documentReport("wellfriendpdf_document_xfa_report_json");
        private static final MethodHandle XFA_EXTRACT = documentReport("wellfriendpdf_document_xfa_extract_json");
        private static final MethodHandle XFA_SCRIPT_REPORT = documentReport("wellfriendpdf_document_xfa_script_report_json");
        private static final MethodHandle XFA_SECURITY_REPORT = documentReport("wellfriendpdf_document_xfa_security_report_json");
        private static final MethodHandle XFA_RUNTIME_REPORT = downcall(
            "wellfriendpdf_document_xfa_runtime_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ANNOTATIONS_REPORT = documentReport("wellfriendpdf_document_annotations_report_json");
        private static final MethodHandle RICH_MEDIA_REPORT = documentReport("wellfriendpdf_document_rich_media_report_json");
        private static final MethodHandle PROMPT17_REPORT = documentReport("wellfriendpdf_document_prompt17_report_json");
        private static final MethodHandle PROMPT18_REPORT = documentReport("wellfriendpdf_document_prompt18_report_json");
        private static final MethodHandle PROMPT18B_REPORT = documentReport("wellfriendpdf_document_prompt18b_report_json");
        private static final MethodHandle FORM_JS_REPORT = documentReport("wellfriendpdf_document_form_js_report_json");
        private static final MethodHandle FORM_ACTION_GRAPH = documentReport("wellfriendpdf_document_form_action_graph_json");
        private static final MethodHandle INTERACTIVE_DATA_REPORT = documentReport("wellfriendpdf_document_interactive_data_report_json");
        private static final MethodHandle WORD_PAGINATION_AUDIT = documentStringReport("wellfriendpdf_document_word_pagination_audit_json");
        private static final MethodHandle PROMPT19_REPORT = documentReport("wellfriendpdf_document_prompt19_report_json");
        private static final MethodHandle PROMPT20_REPORT = documentReport("wellfriendpdf_document_prompt20_report_json");
        private static final MethodHandle PROMPT20B_REPORT = documentReport("wellfriendpdf_document_prompt20b_report_json");
        private static final MethodHandle PROMPT31_REPORT = documentReport("wellfriendpdf_document_prompt31_report_json");
        private static final MethodHandle PROMPT32_REPORT = documentReport("wellfriendpdf_document_prompt32_report_json");
        private static final MethodHandle PROMPT33_REPORT = documentReport("wellfriendpdf_document_prompt33_report_json");
        private static final MethodHandle PROMPT34_REPORT = documentReport("wellfriendpdf_document_prompt34_report_json");
        private static final MethodHandle PROMPT34_ANALYZE = documentReport("wellfriendpdf_document_prompt34_analyze_json");
        private static final MethodHandle PROMPT34_PLAN =
            documentStringReport("wellfriendpdf_document_prompt34_plan_json");
        private static final MethodHandle PROMPT35_REPORT = documentReport("wellfriendpdf_document_prompt35_report_json");
        private static final MethodHandle PROMPT35_ANALYZE = documentReport("wellfriendpdf_document_prompt35_analyze_json");
        private static final MethodHandle PROMPT35_PLAN =
            documentStringReport("wellfriendpdf_document_prompt35_plan_json");
        private static final MethodHandle PROMPT35_VERIFY_RESIDUAL =
            documentStringReport("wellfriendpdf_document_prompt35_verify_residual_json");
        private static final MethodHandle PROMPT21_REPORT = documentReport("wellfriendpdf_document_prompt21_report_json");
        private static final MethodHandle PROMPT21_RASTER_VECTOR_REPORT = downcall(
            "wellfriendpdf_document_prompt21_raster_vector_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT21_FONT_RECONSTRUCTION_REPORT =
            documentReport("wellfriendpdf_document_prompt21_font_reconstruction_report_json");
        private static final MethodHandle PROMPT21_HISTORY_REPORT = downcall(
            "wellfriendpdf_prompt21_history_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT21_OBJECT_STREAM_REPORT =
            documentReport("wellfriendpdf_document_prompt21_object_stream_report_json");
        private static final MethodHandle PROMPT21_PACK_OBJECT_STREAMS = downcall(
            "wellfriendpdf_document_prompt21_pack_object_streams_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT22_REPORT =
            documentReport("wellfriendpdf_document_prompt22_report_json");
        private static final MethodHandle PROMPT23_REPORT =
            documentReport("wellfriendpdf_document_prompt23_report_json");
        private static final MethodHandle SIGNATURE_REPORT_WITH_OPTIONS =
            documentStringReport("wellfriendpdf_document_signatures_with_options_json");
        private static final MethodHandle SIGNATURE_VALIDATION_WITH_EVIDENCE =
            documentStringReport("wellfriendpdf_document_signature_validation_with_evidence_json");
        private static final MethodHandle SIGNATURE_TRUST_STORE_NEW = downcall(
            "wellfriendpdf_signature_trust_store_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_TRUST_STORE_FREE = downcall(
            "wellfriendpdf_signature_trust_store_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_TRUST_STORE_ADD_ANCHOR = downcall(
            "wellfriendpdf_signature_trust_store_add_anchor_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_TRUST_STORE_ADD_DISTRUST = downcall(
            "wellfriendpdf_signature_trust_store_add_distrusted_certificate_sha256",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_INTERMEDIATE_STORE_NEW = downcall(
            "wellfriendpdf_signature_intermediate_store_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_INTERMEDIATE_STORE_FREE = downcall(
            "wellfriendpdf_signature_intermediate_store_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_INTERMEDIATE_STORE_ADD_DER = downcall(
            "wellfriendpdf_signature_intermediate_store_add_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_EVIDENCE_STORE_NEW = downcall(
            "wellfriendpdf_signature_evidence_store_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_EVIDENCE_STORE_FREE = downcall(
            "wellfriendpdf_signature_evidence_store_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_EVIDENCE_STORE_ADD_OCSP = downcall(
            "wellfriendpdf_signature_evidence_store_add_ocsp_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_EVIDENCE_STORE_ADD_CRL = downcall(
            "wellfriendpdf_signature_evidence_store_add_crl_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_EVIDENCE_STORE_SET_BUNDLE = downcall(
            "wellfriendpdf_signature_evidence_store_set_bundle_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_RETRIEVAL_POLICY_NEW = downcall(
            "wellfriendpdf_signature_retrieval_policy_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_RETRIEVAL_POLICY_FREE = downcall(
            "wellfriendpdf_signature_retrieval_policy_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_RETRIEVAL_POLICY_SET_JSON = downcall(
            "wellfriendpdf_signature_retrieval_policy_set_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_CANCELLATION_NEW = downcall(
            "wellfriendpdf_signature_validation_cancellation_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_CANCELLATION_FREE = downcall(
            "wellfriendpdf_signature_validation_cancellation_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_CANCELLATION_CANCEL = downcall(
            "wellfriendpdf_signature_validation_cancellation_cancel",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_NEW = downcall(
            "wellfriendpdf_signature_validation_options_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_FREE = downcall(
            "wellfriendpdf_signature_validation_options_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_ADD_TRUST_ANCHOR = downcall(
            "wellfriendpdf_signature_validation_options_add_trust_anchor_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_ADD_INTERMEDIATE = downcall(
            "wellfriendpdf_signature_validation_options_add_intermediate_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_ADD_DISTRUST = downcall(
            "wellfriendpdf_signature_validation_options_add_distrusted_certificate_sha256",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_ADD_OCSP = downcall(
            "wellfriendpdf_signature_validation_options_add_ocsp_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_ADD_CRL = downcall(
            "wellfriendpdf_signature_validation_options_add_crl_der",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_VALIDATION_TIME = downcall(
            "wellfriendpdf_signature_validation_options_set_validation_time_unix",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_CLEAR_VALIDATION_TIME = downcall(
            "wellfriendpdf_signature_validation_options_clear_validation_time",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_REVOCATION_MODE = downcall(
            "wellfriendpdf_signature_validation_options_set_revocation_mode",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_RETRIEVAL_POLICY = downcall(
            "wellfriendpdf_signature_validation_options_set_retrieval_policy_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_ALGORITHM_POLICY = downcall(
            "wellfriendpdf_signature_validation_options_set_algorithm_policy_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_EVIDENCE_BUNDLE = downcall(
            "wellfriendpdf_signature_validation_options_set_evidence_bundle_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_PATH_LIMITS = downcall(
            "wellfriendpdf_signature_validation_options_set_path_limits",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_APPLY_TRUST_STORE = downcall(
            "wellfriendpdf_signature_validation_options_apply_trust_store",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_APPLY_INTERMEDIATE_STORE = downcall(
            "wellfriendpdf_signature_validation_options_apply_intermediate_store",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_APPLY_EVIDENCE_STORE = downcall(
            "wellfriendpdf_signature_validation_options_apply_evidence_store",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_APPLY_RETRIEVAL_POLICY = downcall(
            "wellfriendpdf_signature_validation_options_apply_retrieval_policy",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_OPTIONS_SET_CANCELLATION = downcall(
            "wellfriendpdf_signature_validation_options_set_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_REPORT_WITH_OPTIONS_HANDLE = downcall(
            "wellfriendpdf_document_signatures_with_options_handle",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_VALIDATION_WITH_EVIDENCE_HANDLE = downcall(
            "wellfriendpdf_document_signature_validation_with_evidence_handle",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TIMESTAMP_TOKEN_VALIDATION = downcall(
            "wellfriendpdf_timestamp_token_validation_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle WRITER_DETERMINISM_AUDIT =
            documentReport("wellfriendpdf_document_writer_determinism_audit_json");
        private static final MethodHandle WRITER_EXTERNAL_DIFF =
            documentReport("wellfriendpdf_document_writer_external_diff_json");
        private static final MethodHandle WRITER_CLOSEOUT_REPORT =
            documentReport("wellfriendpdf_document_writer_closeout_report_json");
        private static final MethodHandle PUBSEC_REPORT =
            documentReport("wellfriendpdf_document_pubsec_report_json");
        private static final MethodHandle AES_GCM_REPORT =
            documentReport("wellfriendpdf_document_aes_gcm_report_json");
        private static final MethodHandle PDF_MAC_REPORT =
            documentReport("wellfriendpdf_document_pdf_mac_report_json");
        private static final MethodHandle PDF_MAC_VERIFY =
            documentReport("wellfriendpdf_document_pdf_mac_verify_json");
        private static final MethodHandle PDF_MAC_CREATE = downcall(
            "wellfriendpdf_document_pdf_mac_create_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT22_OPTIMIZE = downcall(
            "wellfriendpdf_document_prompt22_optimize_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT22_OFFICE_INSPECT = downcall(
            "wellfriendpdf_prompt22_office_inspect_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT22_OFFICE_TO_PDF = downcall(
            "wellfriendpdf_prompt22_office_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20B_TEXT_RANGE_ANALYZE = downcall(
            "wellfriendpdf_document_prompt20b_text_range_analyze_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20B_TEXT_RANGE_EDIT = downcall(
            "wellfriendpdf_document_prompt20b_text_range_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT31_PROVENANCE = downcall(
            "wellfriendpdf_document_prompt31_provenance_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT31_EDIT_ELIGIBILITY = downcall(
            "wellfriendpdf_document_prompt31_edit_eligibility_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT31_OPERATOR_TEXT_EDIT = downcall(
            "wellfriendpdf_document_prompt31_operator_text_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT31_PATH_PROVENANCE = downcall(
            "wellfriendpdf_document_prompt31_path_provenance_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT31_PATH_EDIT = downcall(
            "wellfriendpdf_document_prompt31_path_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_SCENE_REPORT = downcall(
            "wellfriendpdf_document_prompt32_scene_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_SCENE_SELECT = downcall(
            "wellfriendpdf_document_prompt32_scene_select_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_TRANSACTION_PLAN = downcall(
            "wellfriendpdf_document_prompt32_transaction_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_TRANSACTION_APPLY = downcall(
            "wellfriendpdf_document_prompt32_transaction_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_TEXT_MAP = downcall(
            "wellfriendpdf_document_prompt32_text_map_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_SHAPE_TEXT = downcall(
            "wellfriendpdf_document_prompt32_shape_text_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_FONT_SUBSET_PLAN = downcall(
            "wellfriendpdf_document_prompt32_font_subset_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT32_FONT_SUBSTITUTION_REPORT = downcall(
            "wellfriendpdf_document_prompt32_font_substitution_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT33_LAYOUT_ANALYZE =
            documentStringReport("wellfriendpdf_document_prompt33_layout_analyze_json");
        private static final MethodHandle PROMPT33_SEMANTIC_LAYOUT =
            documentReport("wellfriendpdf_document_prompt33_semantic_layout_json");
        private static final MethodHandle PROMPT33_READING_ORDER =
            documentReport("wellfriendpdf_document_prompt33_reading_order_report_json");
        private static final MethodHandle PROMPT33_FLOW_GRAPH =
            documentReport("wellfriendpdf_document_prompt33_flow_graph_report_json");
        private static final MethodHandle PROMPT33_REFLOW_PREVIEW =
            documentStringReport("wellfriendpdf_document_prompt33_reflow_preview_json");
        private static final MethodHandle PROMPT33_OVERFLOW_REPORT =
            documentStringReport("wellfriendpdf_document_prompt33_overflow_report_json");
        private static final MethodHandle PROMPT33_CONSTRAINTS_REPORT =
            documentStringReport("wellfriendpdf_document_prompt33_constraints_report_json");
        private static final MethodHandle PROMPT33_CONFIDENCE_REPORT =
            documentStringReport("wellfriendpdf_document_prompt33_confidence_report_json");
        private static final MethodHandle PROMPT33_VALIDATE_REFLOW_OUTPUT = downcall(
            "wellfriendpdf_document_prompt33_validate_reflow_output_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT33_REFLOW_REGION = downcall(
            "wellfriendpdf_document_prompt33_reflow_region_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT33_REFLOW_DOCUMENT = downcall(
            "wellfriendpdf_document_prompt33_reflow_document_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT33_UNDO_REFLOW = downcall(
            "wellfriendpdf_document_prompt33_undo_reflow_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT33_APPROVE_STRUCTURE =
            documentStringReport("wellfriendpdf_document_prompt33_reflow_approve_structure_json");
        private static final MethodHandle PROMPT33_OPERATION_REPORT =
            documentStringReport("wellfriendpdf_document_prompt33_reflow_operation_report_json");
        private static final MethodHandle PROMPT34_APPLY = downcall(
            "wellfriendpdf_document_prompt34_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT34_UNDO = downcall(
            "wellfriendpdf_document_prompt34_undo_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT35_APPLY = downcall(
            "wellfriendpdf_document_prompt35_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT35_UNDO = downcall(
            "wellfriendpdf_document_prompt35_undo_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20_VECTOR_LIST = downcall(
            "wellfriendpdf_document_prompt20_vector_list_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20_TEXT_EDIT = downcall(
            "wellfriendpdf_document_prompt20_text_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20_VECTOR_EDIT = downcall(
            "wellfriendpdf_document_prompt20_vector_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROMPT20_INK_FIT = downcall(
            "wellfriendpdf_document_prompt20_ink_fit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_REPORT = documentReport("wellfriendpdf_document_associated_files_report_json");
        private static final MethodHandle EDIT_POLICY_REPORT = documentStringReport("wellfriendpdf_document_edit_policy_report_json");
        private static final MethodHandle SIGNATURE_PRESERVING_FORM_PLAN = downcall(
            "wellfriendpdf_document_signature_preserving_form_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SIGNATURE_PRESERVING_FORM_EDIT = downcall(
            "wellfriendpdf_document_signature_preserving_form_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_BYTE, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ANNOTATION_APPEARANCE_REPORT = documentStringReport("wellfriendpdf_document_annotation_appearance_report_json");
        private static final MethodHandle NONAXIS_REDACTION_PLAN = documentStringReport("wellfriendpdf_document_nonaxis_redaction_plan_json");
        private static final MethodHandle PAGES_REPORT = documentReport("wellfriendpdf_document_pages_report_json");
        private static final MethodHandle INTERACTIVE_REPORT = documentReport("wellfriendpdf_document_interactive_report_json");
        private static final MethodHandle CHUNKS = documentReport("wellfriendpdf_document_chunks_json");
        private static final MethodHandle ADVANCED_CHUNKS = documentReport("wellfriendpdf_document_advanced_chunks_json");
        private static final MethodHandle SEMANTIC_BUNDLE = documentReport("wellfriendpdf_document_semantic_bundle_json");
        private static final MethodHandle SEMANTIC_SEARCH = documentStringReport("wellfriendpdf_document_semantic_search_json");
        private static final MethodHandle XFA_RENDER = downcall(
            "wellfriendpdf_document_xfa_render_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle XFA_FLATTEN = downcall(
            "wellfriendpdf_document_xfa_flatten_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle XFA_SANITIZE = downcall(
            "wellfriendpdf_document_xfa_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ANNOTATION_XFDF_EXPORT = downcall(
            "wellfriendpdf_document_annotation_xfdf_export_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ANNOTATION_XFDF_IMPORT = downcall(
            "wellfriendpdf_document_annotation_xfdf_import_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ANNOTATION_APPEARANCE_GENERATE = downcall(
            "wellfriendpdf_document_annotation_appearance_generate_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RICH_MEDIA_SANITIZE = downcall(
            "wellfriendpdf_document_rich_media_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RICH_MEDIA_FLATTEN_POSTER = downcall(
            "wellfriendpdf_document_rich_media_flatten_poster_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle NONAXIS_REDACTION_APPLY = downcall(
            "wellfriendpdf_document_nonaxis_redaction_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle REDACT_IMAGE_MASK = downcall(
            "wellfriendpdf_document_redact_image_mask_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle REDACT_INLINE_IMAGE = downcall(
            "wellfriendpdf_document_redact_inline_image_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_ADD = downcall(
            "wellfriendpdf_document_associated_files_add_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_UPDATE_OWNER = downcall(
            "wellfriendpdf_document_associated_files_update_owner_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_REMOVE_OWNER = downcall(
            "wellfriendpdf_document_associated_files_remove_owner_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle INCREMENTAL_FORM_EDIT = downcall(
            "wellfriendpdf_document_incremental_form_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_BYTE, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle INCREMENTAL_ANNOTATION_EDIT = downcall(
            "wellfriendpdf_document_incremental_annotation_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_BYTE, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle INCREMENTAL_PAGE_PROPERTY_EDIT = downcall(
            "wellfriendpdf_document_incremental_page_property_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_BYTE, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_EXTRACT = downcall(
            "wellfriendpdf_document_associated_files_extract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_REMOVE = downcall(
            "wellfriendpdf_document_associated_files_remove_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ASSOCIATED_FILES_SANITIZE = downcall(
            "wellfriendpdf_document_associated_files_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FORM_JS_SANITIZE = downcall(
            "wellfriendpdf_document_form_js_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FORM_JS_FLATTEN_VALUES = downcall(
            "wellfriendpdf_document_form_js_flatten_values_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SANITIZE = downcall(
            "wellfriendpdf_document_sanitize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle CANONICALIZE = downcall(
            "wellfriendpdf_document_canonicalize_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle REDACT = downcall(
            "wellfriendpdf_document_redact_terms_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle FEATURE_REPORT = downcall(
            "wellfriendpdf_feature_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle CRYPTO_TAMPER_TEST = downcall(
            "wellfriendpdf_crypto_tamper_test_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle CODEC_ISOLATION_REPORT = downcall(
            "wellfriendpdf_codec_isolation_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle VERSION = downcall(
            "wellfriendpdf_version",
            FunctionDescriptor.of(ValueLayout.ADDRESS)
        );
        private static final MethodHandle ABI_VERSION = downcall(
            "wellfriendpdf_abi_version",
            FunctionDescriptor.of(ValueLayout.JAVA_INT)
        );

        private static SymbolLookup loadLibrary() {
            Path path = findNativeLibrary();
            if (path == null) {
                throw new IllegalStateException(
                    "Could not locate wellfriendpdf_capi native library. Set WELLFRIENDPDF_NATIVE_LIBRARY or place it under target/debug, target/release, or runtimes/<rid>/native.");
            }
            return SymbolLookup.libraryLookup(path, LOOKUP_ARENA);
        }

        private static Path findNativeLibrary() {
            String explicit = System.getenv("WELLFRIENDPDF_NATIVE_LIBRARY");
            if (explicit != null && !explicit.isBlank() && Files.exists(Path.of(explicit))) {
                return Path.of(explicit);
            }

            String mapped = mappedLibraryName();
            String rid = runtimeIdentifier();
            Path cwd = Path.of("").toAbsolutePath();
            List<Path> roots = new ArrayList<>();
            roots.add(cwd);
            roots.add(cwd.resolve("target/debug"));
            roots.add(cwd.resolve("target/release"));
            Path packageBase = packageBase();
            if (packageBase != null) {
                roots.add(packageBase);
            }
            for (Path root : roots) {
                Path direct = root.resolve(mapped);
                if (Files.exists(direct)) {
                    return direct;
                }
                Path ridPath = root.resolve("runtimes").resolve(rid).resolve("native").resolve(mapped);
                if (Files.exists(ridPath)) {
                    return ridPath;
                }
            }
            return null;
        }

        private static Path packageBase() {
            try {
                Path location = Path.of(WellfriendPdf.class.getProtectionDomain().getCodeSource().getLocation().toURI());
                return Files.isRegularFile(location) ? location.getParent() : location;
            } catch (NullPointerException | SecurityException | URISyntaxException ex) {
                return null;
            }
        }

        private static String mappedLibraryName() {
            String os = System.getProperty("os.name", "").toLowerCase();
            if (os.contains("win")) {
                return "wellfriendpdf_capi.dll";
            }
            if (os.contains("mac") || os.contains("darwin")) {
                return "libwellfriendpdf_capi.dylib";
            }
            return "libwellfriendpdf_capi.so";
        }

        private static String runtimeIdentifier() {
            String os = System.getProperty("os.name", "").toLowerCase();
            String osPart = os.contains("win") ? "win" : os.contains("mac") || os.contains("darwin") ? "osx" : "linux";
            String arch = System.getProperty("os.arch", "").toLowerCase();
            String archPart = arch.contains("aarch64") || arch.contains("arm64") ? "arm64" : arch.contains("86") && !arch.contains("64") ? "x86" : "x64";
            return osPart + "-" + archPart;
        }

        private static MethodHandle documentReport(String name) {
            return downcall(
                name,
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
            );
        }

        private static MethodHandle documentStringReport(String name) {
            return downcall(
                name,
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
            );
        }

        private static MethodHandle downcall(String name, FunctionDescriptor descriptor) {
            MemorySegment symbol = LOOKUP.find(name)
                .orElseThrow(() -> new UnsatisfiedLinkError("Missing native symbol " + name));
            return LINKER.downcallHandle(symbol, descriptor);
        }

        private static String featureReportJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) FEATURE_REPORT.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend feature_report failed", ex);
            }
        }

        private static String prompt21HistoryReportJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT21_HISTORY_REPORT.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt21_history_report failed", ex);
            }
        }

        private static String cryptoTamperTestJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) CRYPTO_TAMPER_TEST.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend crypto_tamper_test failed", ex);
            }
        }

        private static String codecIsolationReportJson(String filter, byte[] encodedBytes, String policy) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment filterPtr = arena.allocateFrom(filter);
                MemorySegment data = encodedBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(encodedBytes.length);
                if (encodedBytes.length > 0) {
                    data.copyFrom(MemorySegment.ofArray(encodedBytes));
                }
                MemorySegment policyPtr = policy == null || policy.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(policy);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) CODEC_ISOLATION_REPORT.invokeExact(
                    filterPtr,
                    data,
                    (long) encodedBytes.length,
                    policyPtr,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend codec isolation report failed", ex);
            }
        }

        private static String timestampTokenValidationJson(
            byte[] tokenDer,
            byte[] signatureValue,
            String optionsJson
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment token = tokenDer.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(tokenDer.length);
                if (tokenDer.length > 0) {
                    token.copyFrom(MemorySegment.ofArray(tokenDer));
                }
                MemorySegment signature = signatureValue.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(signatureValue.length);
                if (signatureValue.length > 0) {
                    signature.copyFrom(MemorySegment.ofArray(signatureValue));
                }
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) TIMESTAMP_TOKEN_VALIDATION.invokeExact(
                    token,
                    (long) tokenDer.length,
                    signature,
                    (long) signatureValue.length,
                    options,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend timestamp_token_validation failed", ex);
            }
        }

        private static String engineVersion() {
            try (Arena ignored = Arena.ofConfined()) {
                MemorySegment ptr = (MemorySegment) VERSION.invokeExact();
                if (isNull(ptr)) {
                    return "";
                }
                try {
                    return ptr.reinterpret(Long.MAX_VALUE).getString(0);
                } finally {
                    STRING_FREE.invokeExact(ptr);
                }
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend version query failed", ex);
            }
        }

        private static int abiVersion() {
            try {
                return (int) ABI_VERSION.invokeExact();
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend ABI version query failed", ex);
            }
        }

        private static MemorySegment newSignatureValidationOptions() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment options = (MemorySegment) SIGNATURE_OPTIONS_NEW.invokeExact(error);
                if (isNull(options)) {
                    throwError(2, error);
                }
                return options;
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation options creation failed", ex);
            }
        }

        private static void freeSignatureValidationOptions(MemorySegment options) {
            if (isNull(options)) return;
            try {
                SIGNATURE_OPTIONS_FREE.invokeExact(options);
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation options release failed", ex);
            }
        }

        private static MemorySegment newSignatureComponent(MethodHandle constructor, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment component = (MemorySegment) constructor.invokeExact(error);
                if (isNull(component)) {
                    throwError(2, error);
                }
                return component;
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " creation failed", ex);
            }
        }

        private static void freeSignatureComponent(
            MemorySegment component, MethodHandle destructor, String operation
        ) {
            if (isNull(component)) return;
            try {
                destructor.invokeExact(component);
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " release failed", ex);
            }
        }

        private static void applySignatureComponent(
            MemorySegment options, MemorySegment component, MethodHandle method, String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(options, component, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " attachment failed", ex);
            }
        }

        private static void cancelSignatureValidation(MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_CANCELLATION_CANCEL.invokeExact(cancellation, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation cancellation failed", ex);
            }
        }

        private static void addSignatureValidationDer(MemorySegment options, byte[] der, MethodHandle method) {
            Objects.requireNonNull(der, "der");
            if (der.length == 0) throw new IllegalArgumentException("DER input must not be empty");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment bytes = arena.allocate(der.length);
                bytes.copyFrom(MemorySegment.ofArray(der));
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(options, bytes, (long) der.length, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation DER input failed", ex);
            }
        }

        private static void setSignatureValidationTime(MemorySegment options, long validationTimeUnix) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_OPTIONS_SET_VALIDATION_TIME.invokeExact(
                    options, validationTimeUnix, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation time configuration failed", ex);
            }
        }

        private static void clearSignatureValidationTime(MemorySegment options) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_OPTIONS_CLEAR_VALIDATION_TIME.invokeExact(options, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation clock configuration failed", ex);
            }
        }

        private static void setSignatureValidationMode(MemorySegment options, int mode) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_OPTIONS_SET_REVOCATION_MODE.invokeExact(options, mode, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend revocation-policy configuration failed", ex);
            }
        }

        private static void setSignatureValidationPathLimits(
            MemorySegment options, long maxChainDepth, long maxPathCandidates
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_OPTIONS_SET_PATH_LIMITS.invokeExact(
                    options, maxChainDepth, maxPathCandidates, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend path-limit configuration failed", ex);
            }
        }

        private static void setSignatureValidationJson(
            MemorySegment options, String json, MethodHandle method
        ) {
            Objects.requireNonNull(json, "json");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment value = arena.allocateFrom(json);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(options, value, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature validation JSON configuration failed", ex);
            }
        }

        private static void setSignatureValidationString(
            MemorySegment options, String value, MethodHandle method, String operation
        ) {
            Objects.requireNonNull(value, "value");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment nativeValue = arena.allocateFrom(value);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(options, nativeValue, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " configuration failed", ex);
            }
        }

        private static String documentSignatureOptionsReport(
            MemorySegment document, MemorySegment options, MethodHandle method, String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(document, options, jsonOut, error);
                throwError(status, error);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String incrementalSigningPlan(
            MemorySegment handle, String keyPem, String certPem, long placeholderSize, int certify
        ) {
            validateIncrementalSigningArguments(keyPem, certPem, placeholderSize, certify);
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment key = arena.allocateFrom(keyPem);
                MemorySegment cert = arena.allocateFrom(certPem);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) INCREMENTAL_SIGNING_PLAN.invokeExact(
                    handle, key, cert, placeholderSize, certify, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend incremental signing plan failed", ex);
            }
        }

        private static BinaryResult signIncremental(
            MemorySegment handle,
            String keyPem,
            String certPem,
            long placeholderSize,
            int certify,
            String fieldName,
            String reason
        ) {
            validateIncrementalSigningArguments(keyPem, certPem, placeholderSize, certify);
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment key = arena.allocateFrom(keyPem);
                MemorySegment cert = arena.allocateFrom(certPem);
                MemorySegment field = fieldName == null || fieldName.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(fieldName);
                MemorySegment reasonValue = reason == null || reason.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(reason);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) INCREMENTAL_SIGN.invokeExact(
                    handle, key, cert, placeholderSize, certify, field, reasonValue,
                    buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend incremental signing failed", ex);
            }
        }

        private static void validateIncrementalSigningArguments(
            String keyPem, String certPem, long placeholderSize, int certify
        ) {
            Objects.requireNonNull(keyPem, "keyPem");
            Objects.requireNonNull(certPem, "certPem");
            if (keyPem.isBlank() || certPem.isBlank()) {
                throw new IllegalArgumentException("keyPem and certPem must not be blank");
            }
            if (placeholderSize <= 0) {
                throw new IllegalArgumentException("placeholderSize must be positive");
            }
            if (certify < 0 || certify > 3) {
                throw new IllegalArgumentException("certify must be 0 or 1 through 3");
            }
        }

        private static String documentReport(MemorySegment handle, MethodHandle method, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String documentStringReport(MemorySegment handle, MethodHandle method, String value, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment arg = value == null || value.isBlank() ? MemorySegment.NULL : arena.allocateFrom(value);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, arg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String xfaRuntimeReport(MemorySegment handle, String scriptPolicy, boolean executeEvents) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment policy = scriptPolicy == null || scriptPolicy.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(scriptPolicy);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) XFA_RUNTIME_REPORT.invokeExact(
                    handle, policy, executeEvents ? 1 : 0, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend xfa_runtime_report failed", ex);
            }
        }

        private static BinaryResult xfaRender(
            MemorySegment handle,
            String scriptPolicy,
            boolean executeEvents,
            int dpi
        ) {
            if (dpi <= 0) {
                throw new IllegalArgumentException("dpi must be positive");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment policy = scriptPolicy == null || scriptPolicy.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(scriptPolicy);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) XFA_RENDER.invokeExact(
                    handle, policy, executeEvents ? 1 : 0, dpi, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend xfa_render failed", ex);
            }
        }

        private static BinaryResult documentStringOutput(MemorySegment handle, MethodHandle method, String value, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment arg = value == null || value.isBlank() ? MemorySegment.NULL : arena.allocateFrom(value);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, arg, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String prompt20VectorList(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20_VECTOR_LIST.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt20_vector_list failed", ex);
            }
        }

        private static String prompt21RasterVectorReport(
            MemorySegment handle, long page, String optionsJson
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT21_RASTER_VECTOR_REPORT.invokeExact(
                    handle, page, options, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt21_raster_vector_report failed", ex);
            }
        }

        private static String prompt20bTextRangeAnalyze(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20B_TEXT_RANGE_ANALYZE.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt20b_text_range_analyze failed", ex); }
        }

        private static BinaryResult prompt20bTextRangeEdit(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20B_TEXT_RANGE_EDIT.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt20b_text_range_edit failed", ex); }
        }

        private static String prompt31Provenance(
            MemorySegment handle, long page, String sourceText, String replacementText
        ) {
            Objects.requireNonNull(sourceText, "sourceText");
            Objects.requireNonNull(replacementText, "replacementText");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment source = arena.allocateFrom(sourceText);
                MemorySegment replacement = arena.allocateFrom(replacementText);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT31_PROVENANCE.invokeExact(
                    handle, page, source, replacement, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt31_provenance failed", ex); }
        }

        private static String prompt31EditEligibility(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT31_EDIT_ELIGIBILITY.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt31_edit_eligibility failed", ex); }
        }

        private static BinaryResult prompt31OperatorTextEdit(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT31_OPERATOR_TEXT_EDIT.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt31_operator_text_edit failed", ex); }
        }

        private static String prompt31PathProvenance(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT31_PATH_PROVENANCE.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt31_path_provenance failed", ex); }
        }

        private static BinaryResult prompt31PathEdit(
            MemorySegment handle, long page, String stableId, String operationJson,
            String optionsJson
        ) {
            Objects.requireNonNull(stableId, "stableId");
            Objects.requireNonNull(operationJson, "operationJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment id = arena.allocateFrom(stableId);
                MemorySegment operation = arena.allocateFrom(operationJson);
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT31_PATH_EDIT.invokeExact(
                    handle, page, id, operation, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt31_path_edit failed", ex); }
        }

        private static String prompt32SceneReport(MemorySegment handle, String pagesJson) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment pages = pagesJson == null || pagesJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(pagesJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_SCENE_REPORT.invokeExact(handle, pages, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_scene_report failed", ex); }
        }

        private static String prompt32SceneSelect(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_SCENE_SELECT.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_scene_select failed", ex); }
        }

        private static String prompt32TransactionPlan(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_TRANSACTION_PLAN.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_transaction_plan failed", ex); }
        }

        private static BinaryResult prompt32TransactionApply(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_TRANSACTION_APPLY.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_transaction_apply failed", ex); }
        }

        private static BinaryResult prompt33RequestOutput(
            MemorySegment handle, MethodHandle method, String requestJson, String operation
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult prompt33UndoReflow(
            MemorySegment handle, byte[] outputPdf, String requestJson, String operation
        ) {
            Objects.requireNonNull(outputPdf, "outputPdf");
            Objects.requireNonNull(requestJson, "requestJson");
            if (requestJson.isBlank()) {
                throw new IllegalArgumentException("requestJson must not be blank");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = outputPdf.length == 0 ? MemorySegment.NULL : arena.allocate(outputPdf.length);
                if (outputPdf.length > 0) {
                    output.copyFrom(MemorySegment.ofArray(outputPdf));
                }
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT33_UNDO_REFLOW.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult prompt34Undo(
            MemorySegment handle, byte[] outputPdf, String requestJson, String operation
        ) {
            Objects.requireNonNull(outputPdf, "outputPdf");
            Objects.requireNonNull(requestJson, "requestJson");
            if (requestJson.isBlank()) {
                throw new IllegalArgumentException("requestJson must not be blank");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = outputPdf.length == 0 ? MemorySegment.NULL : arena.allocate(outputPdf.length);
                if (outputPdf.length > 0) {
                    output.copyFrom(MemorySegment.ofArray(outputPdf));
                }
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT34_UNDO.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult prompt35Undo(
            MemorySegment handle, byte[] outputPdf, String requestJson, String operation
        ) {
            Objects.requireNonNull(outputPdf, "outputPdf");
            Objects.requireNonNull(requestJson, "requestJson");
            if (requestJson.isBlank()) {
                throw new IllegalArgumentException("requestJson must not be blank");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = outputPdf.length == 0 ? MemorySegment.NULL : arena.allocate(outputPdf.length);
                if (outputPdf.length > 0) {
                    output.copyFrom(MemorySegment.ofArray(outputPdf));
                }
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT35_UNDO.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String prompt33ValidateReflowOutput(
            MemorySegment handle, byte[] outputPdf, String requestJson, String operation
        ) {
            Objects.requireNonNull(outputPdf, "outputPdf");
            Objects.requireNonNull(requestJson, "requestJson");
            if (requestJson.isBlank()) {
                throw new IllegalArgumentException("requestJson must not be blank");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = outputPdf.length == 0 ? MemorySegment.NULL : arena.allocate(outputPdf.length);
                if (outputPdf.length > 0) {
                    output.copyFrom(MemorySegment.ofArray(outputPdf));
                }
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT33_VALIDATE_REFLOW_OUTPUT.invokeExact(
                    handle, output, (long) outputPdf.length, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String prompt32TextMap(MemorySegment handle, String text, String direction) {
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment directionArg = direction == null || direction.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(direction);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_TEXT_MAP.invokeExact(handle, textArg, directionArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_text_map failed", ex); }
        }

        private static String prompt32ShapeText(MemorySegment handle, String text, String direction) {
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment directionArg = direction == null || direction.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(direction);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_SHAPE_TEXT.invokeExact(handle, textArg, directionArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_shape_text failed", ex); }
        }

        private static String prompt32FontSubsetPlan(
            MemorySegment handle, String text, String direction, String policy
        ) {
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment directionArg = direction == null || direction.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(direction);
                MemorySegment policyArg = policy == null || policy.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(policy);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_FONT_SUBSET_PLAN.invokeExact(
                    handle, textArg, directionArg, policyArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_font_subset_plan failed", ex); }
        }

        private static String prompt32FontSubstitutionReport(
            MemorySegment handle, String requestedFamily, String text, String policy
        ) {
            Objects.requireNonNull(requestedFamily, "requestedFamily");
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment familyArg = arena.allocateFrom(requestedFamily);
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment policyArg = policy == null || policy.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(policy);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT32_FONT_SUBSTITUTION_REPORT.invokeExact(
                    handle, familyArg, textArg, policyArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend prompt32_font_substitution_report failed", ex); }
        }

        private static BinaryResult prompt20TextEdit(
            MemorySegment handle, long page, String oldText, String newText,
            String mode, String optionsJson
        ) {
            Objects.requireNonNull(oldText, "oldText");
            Objects.requireNonNull(newText, "newText");
            Objects.requireNonNull(mode, "mode");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment oldArg = arena.allocateFrom(oldText);
                MemorySegment newArg = arena.allocateFrom(newText);
                MemorySegment modeArg = arena.allocateFrom(mode);
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20_TEXT_EDIT.invokeExact(
                    handle, page, oldArg, newArg, modeArg, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt20_text_edit failed", ex);
            }
        }

        private static BinaryResult prompt20VectorEdit(
            MemorySegment handle, long page, String stableId, String operationJson,
            String optionsJson
        ) {
            Objects.requireNonNull(stableId, "stableId");
            Objects.requireNonNull(operationJson, "operationJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment id = arena.allocateFrom(stableId);
                MemorySegment operation = arena.allocateFrom(operationJson);
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20_VECTOR_EDIT.invokeExact(
                    handle, page, id, operation, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt20_vector_edit failed", ex);
            }
        }

        private static BinaryResult prompt20InkFit(
            MemorySegment handle, long page, long annotationIndex, String optionsJson,
            boolean signaturePolicyOverride
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT20_INK_FIT.invokeExact(
                    handle, page, annotationIndex, options,
                    signaturePolicyOverride ? 1 : 0, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt20_ink_fit failed", ex);
            }
        }

        private static BinaryResult documentOutput(MemorySegment handle, MethodHandle method, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult documentTwoStringOutput(
            MemorySegment handle,
            MethodHandle method,
            String first,
            String second,
            String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment firstArg = first == null || first.isBlank() ? MemorySegment.NULL : arena.allocateFrom(first);
                MemorySegment secondArg = second == null || second.isBlank() ? MemorySegment.NULL : arena.allocateFrom(second);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, firstArg, secondArg, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult annotationXfdfImport(
            MemorySegment handle,
            byte[] xfdf,
            String optionsJson
        ) {
            Objects.requireNonNull(xfdf, "xfdf");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = xfdf.length == 0 ? MemorySegment.NULL : arena.allocate(xfdf.length);
                if (xfdf.length > 0) {
                    data.copyFrom(MemorySegment.ofArray(xfdf));
                }
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ANNOTATION_XFDF_IMPORT.invokeExact(
                    handle, data, (long) xfdf.length, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend annotation_xfdf_import failed", ex);
            }
        }

        private static BinaryResult associatedFileAdd(
            MemorySegment handle,
            byte[] payload,
            String optionsJson
        ) {
            return associatedFilePayloadMutation(
                handle, ASSOCIATED_FILES_ADD, payload, optionsJson,
                "associated_files_add");
        }

        private static BinaryResult associatedFilePayloadMutation(
            MemorySegment handle,
            MethodHandle call,
            byte[] payload,
            String optionsJson,
            String operation
        ) {
            Objects.requireNonNull(payload, "payload");
            Objects.requireNonNull(optionsJson, "optionsJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = payload.length == 0 ? MemorySegment.NULL : arena.allocate(payload.length);
                if (payload.length > 0) {
                    data.copyFrom(MemorySegment.ofArray(payload));
                }
                MemorySegment options = arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) call.invokeExact(
                    handle, data, (long) payload.length, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult documentPolicyOutput(
            MemorySegment handle,
            MethodHandle call,
            String optionsJson,
            boolean signaturePolicyOverride,
            String operation
        ) {
            Objects.requireNonNull(optionsJson, "optionsJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment options = arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) call.invokeExact(
                    handle, options, (byte) (signaturePolicyOverride ? 1 : 0),
                    buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult incrementalFormEdit(
            MemorySegment handle,
            String fieldName,
            String value,
            boolean signaturePolicyOverride
        ) {
            Objects.requireNonNull(fieldName, "fieldName");
            Objects.requireNonNull(value, "value");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment field = arena.allocateFrom(fieldName);
                MemorySegment text = arena.allocateFrom(value);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) INCREMENTAL_FORM_EDIT.invokeExact(
                    handle, field, text, (byte) (signaturePolicyOverride ? 1 : 0),
                    buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend incremental_form_edit failed", ex);
            }
        }

        private static String signaturePreservingFormPlan(
            MemorySegment handle,
            String fieldName,
            String value,
            String optionsJson
        ) {
            Objects.requireNonNull(fieldName, "fieldName");
            Objects.requireNonNull(value, "value");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment field = arena.allocateFrom(fieldName);
                MemorySegment text = arena.allocateFrom(value);
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_PRESERVING_FORM_PLAN.invokeExact(
                    handle, field, text, options, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature_preserving_form_plan failed", ex);
            }
        }

        private static BinaryResult signaturePreservingFormEdit(
            MemorySegment handle,
            String fieldName,
            String value,
            String optionsJson,
            boolean explicitInvalidationOverride
        ) {
            Objects.requireNonNull(fieldName, "fieldName");
            Objects.requireNonNull(value, "value");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment field = arena.allocateFrom(fieldName);
                MemorySegment text = arena.allocateFrom(value);
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SIGNATURE_PRESERVING_FORM_EDIT.invokeExact(
                    handle,
                    field,
                    text,
                    options,
                    (byte) (explicitInvalidationOverride ? 1 : 0),
                    buffer,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend signature_preserving_form_edit failed", ex);
            }
        }

        private static BinaryResult canonicalize(MemorySegment handle, Long dateEpoch) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                long epoch = dateEpoch == null ? 0L : dateEpoch.longValue();
                int status = (int) CANONICALIZE.invokeExact(handle, epoch, dateEpoch == null ? 0 : 1, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend canonicalize failed", ex);
            }
        }

        private static BinaryResult redactTerms(MemorySegment handle, List<String> terms, boolean strict) {
            Objects.requireNonNull(terms, "terms");
            List<String> clean = terms.stream().filter(t -> t != null && !t.isBlank()).toList();
            if (clean.isEmpty()) {
                throw new IllegalArgumentException("At least one non-empty redaction term is required");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment termArray = arena.allocate(ADDRESS_SIZE * clean.size());
                for (int i = 0; i < clean.size(); i++) {
                    termArray.set(ValueLayout.ADDRESS, ADDRESS_SIZE * i, arena.allocateFrom(clean.get(i)));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) REDACT.invokeExact(handle, termArray, (long) clean.size(), strict ? 1 : 0, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend redact_terms failed", ex);
            }
        }

        private static void throwError(int status, MemorySegment errorPtr) throws Throwable {
            if (status == 0) {
                MemorySegment maybeError = errorPtr.get(ValueLayout.ADDRESS, 0);
                if (!isNull(maybeError)) {
                    ERROR_FREE.invokeExact(maybeError);
                }
                return;
            }
            MemorySegment err = errorPtr.get(ValueLayout.ADDRESS, 0);
            String message = isNull(err)
                ? "Wellfriend native call failed with status " + status
                : err.reinterpret(Long.MAX_VALUE).getString(0);
            if (!isNull(err)) {
                ERROR_FREE.invokeExact(err);
            }
            throw new WellfriendPdfException(message, status);
        }

        private static String takeString(MemorySegment outPtr) throws Throwable {
            MemorySegment ptr = outPtr.get(ValueLayout.ADDRESS, 0);
            if (isNull(ptr)) {
                return "";
            }
            try {
                return ptr.reinterpret(Long.MAX_VALUE).getString(0);
            } finally {
                STRING_FREE.invokeExact(ptr);
            }
        }

        private static byte[] documentToBytes(MemorySegment handle, MethodHandle method, int flag) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, flag, buffer, err);
                throwError(status, err);
                return takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend document conversion failed", ex);
            }
        }

        private static byte[] pubsecEncrypt(MemorySegment handle, byte[] recipientCertificate) {
            Objects.requireNonNull(recipientCertificate, "recipientCertificate");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment cert = arena.allocate(recipientCertificate.length);
                cert.copyFrom(MemorySegment.ofArray(recipientCertificate));
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PUBSEC_ENCRYPT.invokeExact(
                    handle, cert, (long) recipientCertificate.length, buffer, err);
                throwError(status, err);
                return takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend PubSec encrypt failed", ex);
            }
        }

        private static byte[] officeToPdf(byte[] bytes, MethodHandle method) {
            Objects.requireNonNull(bytes, "bytes");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(data, (long) bytes.length, buffer, err);
                throwError(status, err);
                return takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend office-to-pdf conversion failed", ex);
            }
        }

        private static String prompt22OfficeInspect(byte[] bytes, String format) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(format, "format");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment formatPtr = arena.allocateFrom(format);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT22_OFFICE_INSPECT.invokeExact(
                    data, (long) bytes.length, formatPtr, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt22_office_inspect failed", ex);
            }
        }

        private static BinaryResult prompt22OfficeToPdf(byte[] bytes, String format) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(format, "format");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment formatPtr = arena.allocateFrom(format);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) PROMPT22_OFFICE_TO_PDF.invokeExact(
                    data, (long) bytes.length, formatPtr, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prompt22_office_to_pdf failed", ex);
            }
        }

        private static byte[] takeBuffer(MemorySegment buffer) throws Throwable {
            MemorySegment data = buffer.get(ValueLayout.ADDRESS, 0);
            long len = buffer.get(ValueLayout.JAVA_LONG, BUFFER_LEN_OFFSET);
            if (isNull(data) || len <= 0) {
                return new byte[0];
            }
            try {
                return data.reinterpret(len).toArray(ValueLayout.JAVA_BYTE);
            } finally {
                BUFFER_FREE.invokeExact(buffer);
            }
        }

        private static boolean isNull(MemorySegment segment) {
            return segment == null || segment.equals(MemorySegment.NULL) || segment.address() == 0;
        }
    }
}
