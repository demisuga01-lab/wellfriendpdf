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
import java.math.BigDecimal;
import java.math.BigInteger;
import java.net.URISyntaxException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.security.cert.CertificateEncodingException;
import java.security.cert.X509Certificate;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.concurrent.CancellationException;
import java.util.function.BooleanSupplier;
import java.util.function.Consumer;

public final class WellfriendPdf {
    private WellfriendPdf() {
    }

    private static void requireOwnerThread(Thread ownerThread, String type) {
        if (Thread.currentThread() != ownerThread) {
            throw new IllegalStateException(type + " is thread-confined; use and close it on its creating thread");
        }
    }

    public static String featureReportJson() {
        return Native.featureReportJson();
    }

    public static String runtimeCapabilitiesJson(String configJson) {
        return Native.runtimeCapabilitiesJson(configJson);
    }

    public static String runtimeCapabilitiesJson() {
        return runtimeCapabilitiesJson(null);
    }

    public static String universalEditingCapabilitiesV2Json() {
        return Native.universalEditingCapabilitiesV2Json();
    }

    public static String universalEditingApprovalV2Json(
        String planJson,
        String decisionJson
    ) {
        Objects.requireNonNull(planJson, "planJson");
        Objects.requireNonNull(decisionJson, "decisionJson");
        return Native.universalEditingApprovalV2Json(planJson, decisionJson);
    }

    public static String runtimeConfigJson(String configJson) {
        return Native.runtimeConfigJson(configJson);
    }

    public static String runtimeConfigJson() {
        return runtimeConfigJson(null);
    }

    public static String ocrProviderMatrixJson() {
        return Native.ocrProviderMatrixJson();
    }

    public static String writer_historyHistoryReportJson() {
        return Native.writer_historyHistoryReportJson();
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

    /**
     * Authenticates a canonical paint-partition publication receipt with a
     * caller-held HMAC-SHA-256 key. Never expose a server-held key to an
     * untrusted client; remote deployments should use the authenticated HTTP
     * preview/apply routes instead.
     */
    public static String authenticateTextRangePaintPartitionReceipt(
        String publicationReceiptJson,
        String keyId,
        String audience,
        long issuedAtUnix,
        long expiresAtUnix,
        byte[] hmacKey
    ) {
        Objects.requireNonNull(publicationReceiptJson, "publicationReceiptJson");
        Objects.requireNonNull(keyId, "keyId");
        Objects.requireNonNull(audience, "audience");
        Objects.requireNonNull(hmacKey, "hmacKey");
        if (issuedAtUnix < 0 || expiresAtUnix <= issuedAtUnix) {
            throw new IllegalArgumentException("receipt expiry must be later than issuance");
        }
        if (hmacKey.length < 32 || hmacKey.length > 256)
            throw new IllegalArgumentException("HMAC key must contain 32..=256 bytes");
        return Native.authenticateTextRangePaintPartitionReceipt(
            publicationReceiptJson, keyId, audience, issuedAtUnix, expiresAtUnix, hmacKey);
    }

    public static AuthenticatedPaintPartitionPublicationReceipt authenticateTextRangePaintPartitionReceipt(
        PaintPartitionPublicationReceipt publicationReceipt,
        String keyId,
        String audience,
        long issuedAtUnix,
        long expiresAtUnix,
        byte[] hmacKey
    ) {
        Objects.requireNonNull(publicationReceipt, "publicationReceipt");
        String envelope = authenticateTextRangePaintPartitionReceipt(
            publicationReceipt.toJson(), keyId, audience, issuedAtUnix, expiresAtUnix, hmacKey);
        return new AuthenticatedPaintPartitionPublicationReceipt(reportFromEnvelope(
            envelope,
            "advanced_editing_closeout_authenticated_paint_partition_publication_receipt"));
    }

    /** Verifies a host-authenticated receipt and returns the nested ordinary
     * content-bound publication receipt envelope. */
    public static String verifyAuthenticatedTextRangePaintPartitionReceipt(
        String authenticatedReceiptJson,
        String expectedKeyId,
        String expectedAudience,
        long nowUnix,
        long allowedFutureSkewSecs,
        byte[] hmacKey
    ) {
        Objects.requireNonNull(authenticatedReceiptJson, "authenticatedReceiptJson");
        Objects.requireNonNull(expectedKeyId, "expectedKeyId");
        Objects.requireNonNull(expectedAudience, "expectedAudience");
        Objects.requireNonNull(hmacKey, "hmacKey");
        if (nowUnix < 0 || allowedFutureSkewSecs < 0) {
            throw new IllegalArgumentException("verification timestamps must be non-negative");
        }
        if (hmacKey.length < 32 || hmacKey.length > 256)
            throw new IllegalArgumentException("HMAC key must contain 32..=256 bytes");
        return Native.verifyAuthenticatedTextRangePaintPartitionReceipt(
            authenticatedReceiptJson,
            expectedKeyId,
            expectedAudience,
            nowUnix,
            allowedFutureSkewSecs,
            hmacKey);
    }

    public static PaintPartitionPublicationReceipt verifyAuthenticatedTextRangePaintPartitionReceipt(
        AuthenticatedPaintPartitionPublicationReceipt authenticatedReceipt,
        String expectedKeyId,
        String expectedAudience,
        long nowUnix,
        long allowedFutureSkewSecs,
        byte[] hmacKey
    ) {
        Objects.requireNonNull(authenticatedReceipt, "authenticatedReceipt");
        String envelope = verifyAuthenticatedTextRangePaintPartitionReceipt(
            authenticatedReceipt.toJson(), expectedKeyId, expectedAudience,
            nowUnix, allowedFutureSkewSecs, hmacKey);
        return new PaintPartitionPublicationReceipt(reportFromEnvelope(
            envelope,
            "advanced_editing_closeout_verified_paint_partition_publication_receipt"));
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
        private final Thread ownerThread = Thread.currentThread();
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
            requireOwnerThread(ownerThread, "SignatureTrustStore");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureTrustStore is closed");
            return handle;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "SignatureTrustStore");
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_TRUST_STORE_FREE, "trust store");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Owned untrusted path-building certificate store. */
    public static final class SignatureIntermediateStore implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
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
            requireOwnerThread(ownerThread, "SignatureIntermediateStore");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureIntermediateStore is closed");
            return handle;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "SignatureIntermediateStore");
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
        private final Thread ownerThread = Thread.currentThread();
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
            requireOwnerThread(ownerThread, "SignatureEvidenceStore");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureEvidenceStore is closed");
            return handle;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "SignatureEvidenceStore");
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_EVIDENCE_STORE_FREE, "evidence store");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Owned bounded AIA/OCSP/CRL retrieval policy. It starts offline. */
    public static final class SignatureRetrievalPolicy implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
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
            requireOwnerThread(ownerThread, "SignatureRetrievalPolicy");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureRetrievalPolicy is closed");
            return handle;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "SignatureRetrievalPolicy");
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_RETRIEVAL_POLICY_FREE, "retrieval policy");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Cooperative cancellation source for a signature-validation operation. */
    public static final class SignatureValidationCancellation implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;
        private boolean closed;

        public SignatureValidationCancellation() {
            this.handle = Native.newSignatureComponent(
                Native.SIGNATURE_CANCELLATION_NEW, "signature validation cancellation");
        }

        public synchronized void cancel() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureValidationCancellation is closed");
            Native.cancelSignatureValidation(handle);
        }

        private synchronized MemorySegment nativeHandle() {
            requireOwnerThread(ownerThread, "SignatureValidationCancellation");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("SignatureValidationCancellation is closed");
            return handle;
        }

        @Override
        public synchronized void close() {
            requireOwnerThread(ownerThread, "SignatureValidationCancellation");
            if (closed) return;
            Native.freeSignatureComponent(handle, Native.SIGNATURE_CANCELLATION_FREE, "signature validation cancellation");
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Cooperative cancellation source for a contract render operation. */
    public static final class RenderCancellation implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;
        private boolean closed;

        public RenderCancellation() {
            this.handle = Native.newRenderCancellation();
        }

        public synchronized void cancel() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("RenderCancellation is closed");
            Native.cancelRender(handle);
        }

        public synchronized boolean isCancelled() {
            if (closed || Native.isNull(handle)) throw new IllegalStateException("RenderCancellation is closed");
            return Native.isRenderCancelled(handle);
        }

        private synchronized MemorySegment nativeHandle() {
            requireOwnerThread(ownerThread, "RenderCancellation");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("RenderCancellation is closed");
            return handle;
        }

        @Override
        public synchronized void close() {
            requireOwnerThread(ownerThread, "RenderCancellation");
            if (closed) return;
            Native.freeRenderCancellation(handle);
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    /** Retained PDF-native editor. Session calls/close are confined to the creating
     * thread; a supplied RenderCancellation can be signalled from another thread.
     * This class never writes files. Checkpoint requires the exact preview receipt. */
    public static final class StoryEditSession implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;

        public StoryEditSession(byte[] bytes) { this(bytes, (RenderCancellation) null); }
        public StoryEditSession(byte[] bytes, RenderCancellation cancellation) {
            Objects.requireNonNull(bytes, "bytes");
            if (bytes.length == 0 || bytes.length > 256 * 1024 * 1024)
                throw new IllegalArgumentException("Story input must be 1..=256 MiB");
            handle = StoryNative.open(bytes, cancellation == null ? MemorySegment.NULL : cancellation.nativeHandle());
        }
        public StoryEditSession(byte[] bytes, String password) { this(bytes, password, null); }
        public StoryEditSession(
                byte[] bytes,
                String password,
                RenderCancellation cancellation) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(password, "password");
            if (bytes.length == 0 || bytes.length > 256 * 1024 * 1024)
                throw new IllegalArgumentException("Story input must be 1..=256 MiB");
            byte[] passwordBytes = StoryNative.utf8(password);
            try {
                handle = StoryNative.openWithPassword(
                    bytes, passwordBytes,
                    cancellation == null ? MemorySegment.NULL : cancellation.nativeHandle());
            } finally {
                java.util.Arrays.fill(passwordBytes, (byte) 0);
            }
        }
        /** Opens encrypted input with exact permissions/owner password bytes.
         * User/open passwords are rejected because session bytes are an
         * unencrypted working revision. The caller retains ownership of the
         * array; native code copies it into zeroized temporary native memory and
         * the retained session stores no credential. */
        public static StoryEditSession openWithPasswordBytes(byte[] bytes, byte[] password) {
            return openWithPasswordBytes(bytes, password, null);
        }
        public static StoryEditSession openWithPasswordBytes(
                byte[] bytes,
                byte[] password,
                RenderCancellation cancellation) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(password, "password");
            if (bytes.length == 0 || bytes.length > 256 * 1024 * 1024)
                throw new IllegalArgumentException("Story input must be 1..=256 MiB");
            MemorySegment handle = StoryNative.openWithPassword(
                bytes, password,
                cancellation == null ? MemorySegment.NULL : cancellation.nativeHandle());
            return new StoryEditSession(handle);
        }
        private StoryEditSession(MemorySegment handle) { this.handle = handle; }
        private MemorySegment nativeHandle() {
            requireOwnerThread(ownerThread, "StoryEditSession");
            if (Native.isNull(handle)) throw new IllegalStateException("StoryEditSession is closed");
            return handle;
        }
        public String commandJson(String command) { return commandJson(command, null); }
        public String commandJson(String command, RenderCancellation cancellation) {
            return new String(StoryNative.command(nativeHandle(), StoryNative.utf8(command),
                cancellation == null ? MemorySegment.NULL : cancellation.nativeHandle()), StandardCharsets.UTF_8);
        }
        public String statusJson() { return commandJson("{\"op\":\"status\"}"); }
        public String savedStoriesJson() { return commandJson("{\"op\":\"saved_stories\"}"); }
        public String pagesJson() { return commandJson("{\"op\":\"pages\"}"); }
        public String previewJson(String requestJson, RenderCancellation cancellation) {
            StoryNative.checkJsonLength(requestJson);
            return commandJson("{\"op\":\"preview\",\"request\":" + requestJson + "}", cancellation);
        }
        public String previewJson(String requestJson) { return previewJson(requestJson, null); }
        public String checkpointJson(String requestJson, String receiptJson, RenderCancellation cancellation) {
            StoryNative.checkJsonLength(requestJson);
            StoryNative.checkJsonLength(receiptJson);
            return commandJson("{\"op\":\"checkpoint\",\"request\":" + requestJson + ",\"receipt\":" + receiptJson + "}", cancellation);
        }
        public String checkpointJson(String requestJson, String receiptJson) { return checkpointJson(requestJson, receiptJson, null); }
        public boolean undo() { return undo(null); }
        public boolean undo(RenderCancellation cancellation) { return Boolean.parseBoolean(commandJson("{\"op\":\"undo\"}", cancellation)); }
        public boolean redo() { return redo(null); }
        public boolean redo(RenderCancellation cancellation) { return Boolean.parseBoolean(commandJson("{\"op\":\"redo\"}", cancellation)); }
        public byte[] bytes() { return StoryNative.bytes(nativeHandle()); }
        public byte[] renderPagePng(long page, int dpi, RenderCancellation cancellation) {
            if (page < 1 || dpi < 1 || dpi > 300) throw new IllegalArgumentException("Invalid page/DPI");
            return StoryNative.render(nativeHandle(), page, dpi,
                cancellation == null ? MemorySegment.NULL : cancellation.nativeHandle());
        }
        public byte[] renderPagePng(long page, int dpi) { return renderPagePng(page, dpi, null); }
        @Override public void close() {
            requireOwnerThread(ownerThread, "StoryEditSession");
            if (!Native.isNull(handle)) {
                StoryNative.free(handle);
                handle = MemorySegment.NULL;
            }
        }
    }

    /** Caller-owned non-progressive render cache for contract rendering. */
    public static final class RenderCache implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;
        private boolean closed;

        public RenderCache() {
            this.handle = Native.newRenderCache();
        }

        public void clear() {
            Native.clearRenderCache(nativeHandle());
        }

        public String applyRenderInvalidationPlanJson(String planJson) {
            Objects.requireNonNull(planJson, "planJson");
            return Native.applyRenderCacheInvalidationPlan(nativeHandle(), planJson);
        }

        private MemorySegment nativeHandle() {
            requireOwnerThread(ownerThread, "RenderCache");
            if (closed || Native.isNull(handle)) throw new IllegalStateException("RenderCache is closed");
            return handle;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "RenderCache");
            if (closed) return;
            Native.freeRenderCache(handle);
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    public enum PaintPartitionTextMode {
        SafePatch("safe_patch"),
        ParagraphReflowHorizontal("paragraph_reflow_horizontal"),
        ParagraphReflowRtl("paragraph_reflow_rtl"),
        ParagraphReflowVertical("paragraph_reflow_vertical"),
        OverlayFallback("overlay_fallback"),
        Unsupported("unsupported");

        private final String wire;
        PaintPartitionTextMode(String wire) { this.wire = wire; }
        String wire() { return wire; }
    }

    public enum PaintPartitionStylePolicy {
        InheritLeading("inherit_leading"),
        InheritTrailing("inherit_trailing"),
        PreservePerSegment("preserve_per_segment"),
        ExplicitSupplied("explicit_supplied");

        private final String wire;
        PaintPartitionStylePolicy(String wire) { this.wire = wire; }
        String wire() { return wire; }
    }

    public enum PaintPartitionOverflowPolicy {
        Error("error"), Clip("clip"), ExpandRegion("expand_region");
        private final String wire;
        PaintPartitionOverflowPolicy(String wire) { this.wire = wire; }
        String wire() { return wire; }
    }

    public enum PaintPartitionAlignment {
        Left("left"), Right("right"), Center("center"), Start("start"), End("end"), Justify("justify");
        private final String wire;
        PaintPartitionAlignment(String wire) { this.wire = wire; }
        String wire() { return wire; }
    }

    public enum PaintPartitionOrderPolicy {
        RequireSingleSourceTextObject("require_single_source_text_object"),
        AnchorAfterFirstSourceTextObject("anchor_after_first_source_text_object"),
        AnchorAfterLastSourceTextObject("anchor_after_last_source_text_object");
        private final String wire;
        PaintPartitionOrderPolicy(String wire) { this.wire = wire; }
        String wire() { return wire; }
    }

    public record PaintPartitionExplicitLine(
        String logicalText,
        String visualText,
        Map<String, Object> bidi,
        boolean insertedVisualHyphen
    ) {
        public PaintPartitionExplicitLine {
            Objects.requireNonNull(logicalText, "logicalText");
            Objects.requireNonNull(visualText, "visualText");
        }

        Map<String, Object> toMap() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("logical_text", logicalText);
            value.put("visual_text", visualText);
            if (bidi != null) value.put("bidi", bidi);
            value.put("inserted_visual_hyphen", insertedVisualHyphen);
            return value;
        }
    }

    public record PaintPartitionPlacement(
        long sourceTextObject,
        long[] replacementScalarRange,
        double[] region,
        List<PaintPartitionExplicitLine> finalLines
    ) {
        public PaintPartitionPlacement {
            requireNonNegative(sourceTextObject, "sourceTextObject");
            requireRange(replacementScalarRange, "replacementScalarRange");
            requireRegion(region, "region");
            replacementScalarRange = replacementScalarRange.clone();
            region = region.clone();
            finalLines = finalLines == null ? null : List.copyOf(finalLines);
        }

        @Override public long[] replacementScalarRange() { return replacementScalarRange.clone(); }
        @Override public double[] region() { return region.clone(); }

        Map<String, Object> toMap() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("source_text_object", sourceTextObject);
            value.put("replacement_scalar_range", List.of(replacementScalarRange[0], replacementScalarRange[1]));
            value.put("region", doubleList(region));
            if (finalLines != null) value.put("final_lines", finalLines.stream().map(PaintPartitionExplicitLine::toMap).toList());
            return value;
        }
    }

    public record PaintPartitionTextOptions(
        double[] region,
        double fontSize,
        double lineSpacing,
        long maxLinesOrColumns,
        PaintPartitionOverflowPolicy overflowPolicy,
        boolean signaturePolicyOverride,
        boolean deterministic,
        PaintPartitionAlignment alignment,
        boolean justifyLastLine,
        double maxWordSpacing,
        double maxCharacterSpacing,
        Long targetStreamObject,
        Integer targetStreamGeneration,
        long[] targetDecodedByteRange,
        PaintPartitionOrderPolicy paintOrderPolicy,
        List<PaintPartitionPlacement> paintPartitions
    ) {
        public PaintPartitionTextOptions {
            requireRegion(region, "region");
            if (!Double.isFinite(fontSize) || fontSize <= 0) throw new IllegalArgumentException("fontSize must be finite and positive");
            if (!Double.isFinite(lineSpacing) || lineSpacing <= 0) throw new IllegalArgumentException("lineSpacing must be finite and positive");
            if (maxLinesOrColumns <= 0) throw new IllegalArgumentException("maxLinesOrColumns must be positive");
            Objects.requireNonNull(overflowPolicy, "overflowPolicy");
            Objects.requireNonNull(alignment, "alignment");
            Objects.requireNonNull(paintOrderPolicy, "paintOrderPolicy");
            if (!Double.isFinite(maxWordSpacing) || maxWordSpacing < 0) throw new IllegalArgumentException("maxWordSpacing must be finite and non-negative");
            if (!Double.isFinite(maxCharacterSpacing) || maxCharacterSpacing < 0) throw new IllegalArgumentException("maxCharacterSpacing must be finite and non-negative");
            if (targetStreamObject != null) requireNonNegative(targetStreamObject, "targetStreamObject");
            if (targetStreamGeneration != null && (targetStreamGeneration < 0 || targetStreamGeneration > 65535)) throw new IllegalArgumentException("targetStreamGeneration must fit u16");
            if (targetDecodedByteRange != null) requireRange(targetDecodedByteRange, "targetDecodedByteRange");
            region = region.clone();
            targetDecodedByteRange = targetDecodedByteRange == null ? null : targetDecodedByteRange.clone();
            paintPartitions = paintPartitions == null ? List.of() : List.copyOf(paintPartitions);
        }

        @Override public double[] region() { return region.clone(); }
        @Override public long[] targetDecodedByteRange() {
            return targetDecodedByteRange == null ? null : targetDecodedByteRange.clone();
        }

        public static PaintPartitionTextOptions defaults() {
            return new PaintPartitionTextOptions(
                new double[] {36, 36, 576, 756}, 12, 1.2, 4096,
                PaintPartitionOverflowPolicy.Error, false, true,
                PaintPartitionAlignment.Left, false, 0.5, 0.05,
                null, null, null,
                PaintPartitionOrderPolicy.RequireSingleSourceTextObject, List.of());
        }

        Map<String, Object> toMap() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("region", doubleList(region));
            value.put("font_size", fontSize);
            value.put("line_spacing", lineSpacing);
            value.put("max_lines_or_columns", maxLinesOrColumns);
            value.put("overflow_policy", overflowPolicy.wire());
            value.put("signature_policy_override", signaturePolicyOverride);
            value.put("deterministic", deterministic);
            value.put("alignment", alignment.wire());
            value.put("justify_last_line", justifyLastLine);
            value.put("max_word_spacing", maxWordSpacing);
            value.put("max_character_spacing", maxCharacterSpacing);
            if (targetStreamObject != null) value.put("target_stream_object", targetStreamObject);
            if (targetStreamGeneration != null) value.put("target_stream_generation", targetStreamGeneration);
            if (targetDecodedByteRange != null) value.put("target_decoded_byte_range", List.of(targetDecodedByteRange[0], targetDecodedByteRange[1]));
            value.put("paint_order_policy", paintOrderPolicy.wire());
            value.put("paint_partitions", paintPartitions.stream().map(PaintPartitionPlacement::toMap).toList());
            return value;
        }
    }

    public record PaintPartitionTextRangeRequest(
        long page,
        long logicalStart,
        long logicalEnd,
        String replacementText,
        PaintPartitionTextMode mode,
        PaintPartitionStylePolicy stylePolicy,
        PaintPartitionTextOptions options,
        List<PaintPartitionExplicitLine> finalLines
    ) {
        public PaintPartitionTextRangeRequest {
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            if (logicalStart < 0 || logicalEnd < logicalStart) throw new IllegalArgumentException("logical range is invalid");
            Objects.requireNonNull(replacementText, "replacementText");
            Objects.requireNonNull(mode, "mode");
            stylePolicy = stylePolicy == null ? PaintPartitionStylePolicy.InheritLeading : stylePolicy;
            finalLines = finalLines == null ? null : List.copyOf(finalLines);
        }

        public String toJson() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("page", page);
            value.put("logical_start", logicalStart);
            value.put("logical_end", logicalEnd);
            value.put("replacement_text", replacementText);
            value.put("mode", mode.wire());
            value.put("style_policy", stylePolicy.wire());
            if (options != null) value.put("options", options.toMap());
            if (finalLines != null) value.put("final_lines", finalLines.stream().map(PaintPartitionExplicitLine::toMap).toList());
            return Json.write(value);
        }
    }

    public record PaintPartitionApprovalEntry(
        long sourceTextObject,
        double[] region,
        List<PaintPartitionExplicitLine> finalLines
    ) {
        public PaintPartitionApprovalEntry {
            requireNonNegative(sourceTextObject, "sourceTextObject");
            requireRegion(region, "region");
            region = region.clone();
            finalLines = finalLines == null ? null : List.copyOf(finalLines);
        }

        @Override public double[] region() { return region.clone(); }

        Map<String, Object> toMap() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("source_text_object", sourceTextObject);
            value.put("region", doubleList(region));
            if (finalLines != null) value.put("final_lines", finalLines.stream().map(PaintPartitionExplicitLine::toMap).toList());
            return value;
        }
    }

    public record PaintPartitionApproval(
        String proposalId,
        String fontSha256,
        List<PaintPartitionApprovalEntry> partitions
    ) {
        public PaintPartitionApproval {
            if (proposalId == null || proposalId.isBlank()) throw new IllegalArgumentException("proposalId is required");
            if (fontSha256 != null) requireDigest(fontSha256, "fontSha256");
            partitions = List.copyOf(Objects.requireNonNull(partitions, "partitions"));
            if (partitions.isEmpty()) throw new IllegalArgumentException("at least one approved partition is required");
        }

        public String toJson() {
            LinkedHashMap<String, Object> value = new LinkedHashMap<>();
            value.put("proposal_id", proposalId);
            if (fontSha256 != null) value.put("font_sha256", fontSha256);
            value.put("partitions", partitions.stream().map(PaintPartitionApprovalEntry::toMap).toList());
            return Json.write(value);
        }
    }

    public static final class PaintPartitionProposal {
        private final LinkedHashMap<String, Object> report;
        private PaintPartitionProposal(LinkedHashMap<String, Object> report) {
            this.report = report;
            if (!"advanced_editing.paint-partition-proposal.v1".equals(requireString(report, "schema_version")))
                throw new IllegalArgumentException("unsupported paint-partition proposal schema");
            requireDigest(requireString(report, "input_sha256"), "input_sha256");
            requireDigest(requireString(report, "request_sha256"), "request_sha256");
            requireDigest(requireString(report, "proposal_id"), "proposal_id");
            requireDigest(requireString(report, "replacement_sha256"), "replacement_sha256");
            if (requireLong(report, "page") < 1) throw new IllegalArgumentException("proposal page must be one-based");
            requireLongPair(requireList(report, "logical_range"), "logical_range");
            List<?> candidates = requireList(report, "candidates");
            if (candidates.isEmpty()) throw new IllegalArgumentException("proposal candidates must not be empty");
            for (Object candidate : candidates) {
                LinkedHashMap<String, Object> fields = requireObject(candidate, "proposal candidate");
                requireLongPair(requireList(fields, "replacement_scalar_range"), "replacement_scalar_range");
            }
        }
        public static PaintPartitionProposal fromEnvelopeJson(String json) {
            return new PaintPartitionProposal(reportFromEnvelope(
                json, "advanced_editing_closeout_paint_partition_proposal"));
        }
        public String proposalId() { return requireString(report, "proposal_id"); }
        public String toJson() { return Json.write(report); }
    }

    public static final class PaintPartitionPublicationReceipt {
        private final LinkedHashMap<String, Object> value;
        private PaintPartitionPublicationReceipt(LinkedHashMap<String, Object> value) {
            this.value = value;
            if (!"advanced_editing.paint-partition-publication-receipt.v1".equals(requireString(value, "schema_version")))
                throw new IllegalArgumentException("unsupported publication receipt schema");
            requireString(value, "proposal_id");
            for (String field : List.of("input_sha256", "request_sha256", "approval_sha256", "candidate_output_sha256", "preview_evidence_sha256", "receipt_id"))
                requireDigest(requireString(value, field), field);
            Object font = value.get("font_sha256");
            if (font != null) {
                if (!(font instanceof String digest)) throw new IllegalArgumentException("font_sha256 must be a string or null");
                requireDigest(digest, "font_sha256");
            }
        }
        public static PaintPartitionPublicationReceipt fromJson(String json) {
            return new PaintPartitionPublicationReceipt(requireObject(Json.parse(json), "publication receipt"));
        }
        public String proposalId() { return requireString(value, "proposal_id"); }
        public String toJson() { return Json.write(value); }
    }

    public static final class AuthenticatedPaintPartitionPublicationReceipt {
        private final LinkedHashMap<String, Object> value;
        private AuthenticatedPaintPartitionPublicationReceipt(LinkedHashMap<String, Object> value) {
            this.value = value;
            if (!"advanced_editing.paint-partition-authenticated-publication-receipt.v1".equals(requireString(value, "schema_version")))
                throw new IllegalArgumentException("unsupported authenticated receipt schema");
            requireString(value, "key_id");
            requireString(value, "audience");
            long issued = requireLong(value, "issued_at_unix");
            long expires = requireLong(value, "expires_at_unix");
            if (issued < 0 || expires <= issued) throw new IllegalArgumentException("authenticated receipt timestamps are invalid");
            requireDigest(requireString(value, "hmac_sha256"), "hmac_sha256");
            new PaintPartitionPublicationReceipt(requireObject(value.get("publication_receipt"), "publication_receipt"));
        }
        public static AuthenticatedPaintPartitionPublicationReceipt fromJson(String json) {
            return new AuthenticatedPaintPartitionPublicationReceipt(requireObject(Json.parse(json), "authenticated receipt"));
        }
        public String toJson() { return Json.write(value); }
    }

    public static final class PaintPartitionPreview {
        private final LinkedHashMap<String, Object> report;
        private final PaintPartitionPublicationReceipt receipt;
        private PaintPartitionPreview(LinkedHashMap<String, Object> report) {
            this.report = report;
            this.receipt = new PaintPartitionPublicationReceipt(
                requireObject(report.get("publication_receipt"), "publication_receipt"));
        }
        public static PaintPartitionPreview fromEnvelopeJson(String json) {
            return new PaintPartitionPreview(reportFromEnvelope(
                json, "advanced_editing_closeout_paint_partition_preview"));
        }
        public PaintPartitionPublicationReceipt publicationReceipt() { return receipt; }
        public String reportJson() { return Json.write(report); }
    }

    private static LinkedHashMap<String, Object> reportFromEnvelope(String json, String expectedKind) {
        LinkedHashMap<String, Object> envelope = requireObject(Json.parse(Objects.requireNonNull(json, "json")), "envelope");
        if (requireLong(envelope, "schema_version") != 1 || !expectedKind.equals(requireString(envelope, "kind")))
            throw new IllegalArgumentException("unexpected SDK envelope schema or kind");
        return requireObject(envelope.get("report"), "report");
    }

    private static LinkedHashMap<String, Object> requireObject(Object value, String name) {
        if (!(value instanceof Map<?, ?> map)) throw new IllegalArgumentException(name + " must be an object");
        LinkedHashMap<String, Object> copy = new LinkedHashMap<>();
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (!(entry.getKey() instanceof String key)) throw new IllegalArgumentException(name + " has a non-string key");
            copy.put(key, entry.getValue());
        }
        return copy;
    }

    private static String requireString(Map<?, ?> value, String name) {
        Object field = value.get(name);
        if (!(field instanceof String text) || text.isBlank()) throw new IllegalArgumentException(name + " must be a non-empty string");
        return text;
    }

    private static List<?> requireList(Map<?, ?> value, String name) {
        Object field = value.get(name);
        if (!(field instanceof List<?> list)) throw new IllegalArgumentException(name + " must be an array");
        return list;
    }

    private static long requireLong(Map<?, ?> value, String name) {
        Object field = value.get(name);
        if (!(field instanceof Number number)) throw new IllegalArgumentException(name + " must be numeric");
        try {
            if (number instanceof BigDecimal decimal) return decimal.longValueExact();
            if (number instanceof BigInteger integer) return integer.longValueExact();
            return number.longValue();
        } catch (ArithmeticException ex) {
            throw new IllegalArgumentException(name + " must be an in-range integer", ex);
        }
    }

    private static void requireDigest(String value, String name) {
        if (!value.matches("[0-9a-f]{64}")) throw new IllegalArgumentException(name + " must be a lowercase SHA-256 digest");
    }

    private static void requireLongPair(List<?> value, String name) {
        if (value.size() != 2 || !(value.get(0) instanceof Number) || !(value.get(1) instanceof Number))
            throw new IllegalArgumentException(name + " must be a two-value numeric range");
        long start = requireLong(Map.of("value", value.get(0)), "value");
        long end = requireLong(Map.of("value", value.get(1)), "value");
        if (start < 0 || end < start) throw new IllegalArgumentException(name + " is not ordered");
    }

    private static long requireNonNegative(long value, String name) {
        if (value < 0) throw new IllegalArgumentException(name + " must be non-negative");
        return value;
    }

    private static void requireRange(long[] value, String name) {
        if (value == null || value.length != 2 || value[0] < 0 || value[1] < value[0])
            throw new IllegalArgumentException(name + " must be an ordered two-value range");
    }

    private static void requireRegion(double[] value, String name) {
        if (value == null || value.length != 4 || Arrays.stream(value).anyMatch(coordinate -> !Double.isFinite(coordinate))
            || value[2] <= value[0] || value[3] <= value[1])
            throw new IllegalArgumentException(name + " must be a finite non-empty [x0,y0,x1,y1] region");
    }

    private static List<Double> doubleList(double[] values) {
        return Arrays.stream(values).boxed().toList();
    }

    private static void requireSameProposal(
        PaintPartitionProposal proposal,
        PaintPartitionApproval approval
    ) {
        if (!proposal.proposalId().equals(approval.proposalId()))
            throw new IllegalArgumentException("approval belongs to a different proposal");
    }

    public record BinaryResult(byte[] bytes, String reportJson) {
        public void writeBytes(Path path) throws IOException {
            Files.write(path, bytes);
        }
    }

    public static final class RenderContract {
        private static final long SCHEMA_VERSION = 1;

        private final LinkedHashMap<String, Object> values;

        private RenderContract(LinkedHashMap<String, Object> values) {
            this.values = values;
            validate();
        }

        public static RenderContract fromJson(String json) {
            Objects.requireNonNull(json, "json");
            Object parsed = Json.parse(json);
            if (!(parsed instanceof Map<?, ?> map)) {
                throw new IllegalArgumentException("render contract JSON must be an object");
            }
            return new RenderContract(copyObject(map));
        }

        public String toJson() {
            validate();
            return Json.write(values);
        }

        public long width() {
            return requiredLong("width");
        }

        public long height() {
            return requiredLong("height");
        }

        public long stride() {
            return requiredLong("stride");
        }

        public long surfaceByteLength() {
            return Math.multiplyExact(stride(), height());
        }

        public String pixelFormat() {
            return requiredString("pixel_format");
        }

        public RenderContract withSurface(long width, long height, PixelFormat pixelFormat) {
            return withSurface(width, height, pixelFormat, AlphaMode.Premultiplied, -1, false, false);
        }

        public RenderContract withSurface(
            long width,
            long height,
            PixelFormat pixelFormat,
            AlphaMode alphaMode,
            long stride,
            boolean grayscale,
            boolean reverseByteOrder
        ) {
            Objects.requireNonNull(pixelFormat, "pixelFormat");
            Objects.requireNonNull(alphaMode, "alphaMode");
            if (width <= 0 || height <= 0) {
                throw new IllegalArgumentException("render contract surface dimensions must be positive");
            }
            long minimumStride = Math.multiplyExact(width, bytesPerPixel(pixelFormat));
            long effectiveStride = stride < 0 ? minimumStride : stride;
            if (effectiveStride < minimumStride) {
                throw new IllegalArgumentException(
                    "render contract stride " + effectiveStride + " is below the required " + minimumStride);
            }
            LinkedHashMap<String, Object> next = copyValues();
            next.put("width", width);
            next.put("height", height);
            next.put("pixel_format", pixelFormat.name());
            next.put("alpha_mode", alphaMode.name());
            next.put("stride", effectiveStride);
            next.put("grayscale", grayscale);
            next.put("reverse_byte_order", reverseByteOrder);
            return new RenderContract(next);
        }

        public RenderContract withClip(int x, int y, long width, long height) {
            if (width <= 0 || height <= 0) {
                throw new IllegalArgumentException("render contract clip must be non-empty");
            }
            LinkedHashMap<String, Object> clip = new LinkedHashMap<>();
            clip.put("x", (long) x);
            clip.put("y", (long) y);
            clip.put("width", width);
            clip.put("height", height);
            LinkedHashMap<String, Object> next = copyValues();
            next.put("clip", clip);
            return new RenderContract(next);
        }

        public RenderContract withoutClip() {
            LinkedHashMap<String, Object> next = copyValues();
            next.put("clip", null);
            return new RenderContract(next);
        }

        public RenderContract withDeviceTransform(double a, double b, double c, double d, double e, double f) {
            LinkedHashMap<String, Object> transform = new LinkedHashMap<>();
            transform.put("values", List.of(
                unsignedBits(a),
                unsignedBits(b),
                unsignedBits(c),
                unsignedBits(d),
                unsignedBits(e),
                unsignedBits(f)
            ));
            LinkedHashMap<String, Object> next = copyValues();
            next.put("transform", transform);
            return new RenderContract(next);
        }

        public RenderContract withBackground(int r, int g, int b) {
            return withBackground(r, g, b, 255);
        }

        public RenderContract withBackground(int r, int g, int b, int a) {
            LinkedHashMap<String, Object> background = new LinkedHashMap<>();
            background.put("r", checkedByte(r, "r"));
            background.put("g", checkedByte(g, "g"));
            background.put("b", checkedByte(b, "b"));
            background.put("a", checkedByte(a, "a"));
            LinkedHashMap<String, Object> next = copyValues();
            next.put("background", background);
            return new RenderContract(next);
        }

        public RenderContract withPageBox(PageBox pageBox) {
            return withEnum("page_box", pageBox, "pageBox");
        }

        public RenderContract withExecutionMode(ExecutionMode executionMode) {
            return withEnum("execution_mode", executionMode, "executionMode");
        }

        public RenderContract withBackend(BackendSelection backend) {
            return withEnum("backend", backend, "backend");
        }

        public RenderContract withCompositing(CompositingPolicy compositing) {
            return withEnum("compositing", compositing, "compositing");
        }

        public RenderContract withAnnotations(AnnotationPolicy annotations) {
            return withEnum("annotations", annotations, "annotations");
        }

        public RenderContract withForms(FormPolicy forms) {
            return withEnum("forms", forms, "forms");
        }

        public RenderContract withOptionalContent(String optionalContent) {
            Objects.requireNonNull(optionalContent, "optionalContent");
            if (optionalContent.isBlank()) {
                throw new IllegalArgumentException("render contract optional_content must be a non-empty string");
            }
            LinkedHashMap<String, Object> next = copyValues();
            next.put("optional_content", optionalContent);
            return new RenderContract(next);
        }

        public RenderContract withSmoothing(SmoothingPolicy smoothing) {
            Objects.requireNonNull(smoothing, "smoothing");
            LinkedHashMap<String, Object> next = copyValues();
            next.put("text_smoothing", smoothing.name());
            next.put("image_smoothing", smoothing.name());
            next.put("path_smoothing", smoothing.name());
            return new RenderContract(next);
        }

        public RenderContract withTextSmoothing(SmoothingPolicy textSmoothing) {
            return withEnum("text_smoothing", textSmoothing, "textSmoothing");
        }

        public RenderContract withImageSmoothing(SmoothingPolicy imageSmoothing) {
            return withEnum("image_smoothing", imageSmoothing, "imageSmoothing");
        }

        public RenderContract withPathSmoothing(SmoothingPolicy pathSmoothing) {
            return withEnum("path_smoothing", pathSmoothing, "pathSmoothing");
        }

        public RenderContract withSubpixelText(SmoothingPolicy subpixelText) {
            return withEnum("subpixel_text", subpixelText, "subpixelText");
        }

        public RenderContract withColorScheme(ColorScheme colorScheme) {
            return withEnum("color_scheme", colorScheme, "colorScheme");
        }

        public RenderContract withPrintProfile(PrintProfile printProfile) {
            return withEnum("print_profile", printProfile, "printProfile");
        }

        public RenderContract withHalftone(HalftonePolicy halftone) {
            return withEnum("halftone", halftone, "halftone");
        }

        public RenderContract withOverprint(OverprintPolicy overprint) {
            return withEnum("overprint", overprint, "overprint");
        }

        public RenderContract withRenderingIntent(RenderingIntent renderingIntent) {
            return withEnum("rendering_intent", renderingIntent, "renderingIntent");
        }

        public RenderContract withColorManagement(ColorManagementPolicy colorManagement) {
            return withEnum("color_management", colorManagement, "colorManagement");
        }

        public RenderContract withExactness(ExactnessPolicy exactness) {
            return withEnum("exactness", exactness, "exactness");
        }

        public RenderContract withDeterminism(DeterminismPolicy determinism) {
            return withEnum("determinism", determinism, "determinism");
        }

        public RenderContract withResourceBudget(
            Long maxPixels,
            Long maxDecodedBytes,
            Long maxTemporaryBytes,
            Long maxCacheBytes
        ) {
            LinkedHashMap<String, Object> budget = objectValue("resource_budget");
            if (maxPixels != null) budget.put("max_pixels", requireNonNegative(maxPixels, "maxPixels"));
            if (maxDecodedBytes != null) budget.put("max_decoded_bytes", requireNonNegative(maxDecodedBytes, "maxDecodedBytes"));
            if (maxTemporaryBytes != null) budget.put("max_temporary_bytes", requireNonNegative(maxTemporaryBytes, "maxTemporaryBytes"));
            if (maxCacheBytes != null) budget.put("max_cache_bytes", requireNonNegative(maxCacheBytes, "maxCacheBytes"));
            LinkedHashMap<String, Object> next = copyValues();
            next.put("resource_budget", budget);
            return new RenderContract(next);
        }

        private RenderContract withEnum(String key, Enum<?> value, String name) {
            Objects.requireNonNull(value, name);
            LinkedHashMap<String, Object> next = copyValues();
            next.put(key, value.name());
            return new RenderContract(next);
        }

        public void validate() {
            if (requiredLong("schema_version") != SCHEMA_VERSION) {
                throw new IllegalArgumentException("render contract schema is unsupported");
            }
            if (requiredLong("page_number") < 1) {
                throw new IllegalArgumentException("render contract page_number must be 1-based");
            }
            long width = requiredLong("width");
            long height = requiredLong("height");
            if (width <= 0 || height <= 0) {
                throw new IllegalArgumentException("render contract output width and height must be non-zero");
            }
            String pixelFormat = requiredString("pixel_format");
            long minimumStride = Math.multiplyExact(width, bytesPerPixel(pixelFormat));
            if (requiredLong("stride") < minimumStride) {
                throw new IllegalArgumentException("render contract stride is below the required byte count");
            }
            validateTransform();
            validateByteObject("background", "r", "g", "b", "a");
            LinkedHashMap<String, Object> budget = objectValue("resource_budget");
            long maxPixels = requiredLong(budget, "max_pixels");
            if (Math.multiplyExact(width, height) > maxPixels) {
                throw new IllegalArgumentException("render contract exceeds max_pixels budget");
            }
            Object clip = values.get("clip");
            if (clip != null) {
                if (!(clip instanceof Map<?, ?> clipMap)) {
                    throw new IllegalArgumentException("render contract clip must be an object or null");
                }
                if (requiredLong(clipMap, "width") <= 0 || requiredLong(clipMap, "height") <= 0) {
                    throw new IllegalArgumentException("render contract clip must have non-zero dimensions");
                }
            }
            requiredEnum("alpha_mode", AlphaMode.class);
            requiredEnum("page_box", PageBox.class);
            requiredEnum("execution_mode", ExecutionMode.class);
            requiredEnum("backend", BackendSelection.class);
            requiredEnum("compositing", CompositingPolicy.class);
            requiredEnum("annotations", AnnotationPolicy.class);
            requiredEnum("forms", FormPolicy.class);
            requiredString("optional_content");
            requiredEnum("text_smoothing", SmoothingPolicy.class);
            requiredEnum("image_smoothing", SmoothingPolicy.class);
            requiredEnum("path_smoothing", SmoothingPolicy.class);
            requiredEnum("subpixel_text", SmoothingPolicy.class);
            requiredEnum("color_scheme", ColorScheme.class);
            requiredEnum("print_profile", PrintProfile.class);
            requiredEnum("halftone", HalftonePolicy.class);
            requiredEnum("overprint", OverprintPolicy.class);
            requiredEnum("rendering_intent", RenderingIntent.class);
            requiredEnum("color_management", ColorManagementPolicy.class);
            requiredEnum("exactness", ExactnessPolicy.class);
            requiredEnum("determinism", DeterminismPolicy.class);
            requiredBoolean("grayscale");
            requiredBoolean("reverse_byte_order");
        }

        public static long bytesPerPixel(PixelFormat pixelFormat) {
            Objects.requireNonNull(pixelFormat, "pixelFormat");
            return bytesPerPixel(pixelFormat.name());
        }

        private static long bytesPerPixel(String pixelFormat) {
            return switch (pixelFormat) {
                case "Rgba8", "Bgra8" -> 4;
                case "Rgb8", "Bgr8" -> 3;
                case "Gray8" -> 1;
                default -> throw new IllegalArgumentException("unsupported render contract pixel format: " + pixelFormat);
            };
        }

        private void validateTransform() {
            LinkedHashMap<String, Object> transform = objectValue("transform");
            Object values = transform.get("values");
            if (!(values instanceof List<?> matrix) || matrix.size() != 6) {
                throw new IllegalArgumentException("render contract transform must contain six values");
            }
            double a = matrixDouble(matrix.get(0));
            double b = matrixDouble(matrix.get(1));
            double c = matrixDouble(matrix.get(2));
            double d = matrixDouble(matrix.get(3));
            matrixDouble(matrix.get(4));
            matrixDouble(matrix.get(5));
            if (Math.abs(a * d - b * c) < 1e-10) {
                throw new IllegalArgumentException("render contract transform must be invertible");
            }
        }

        private static double matrixDouble(Object value) {
            if (!(value instanceof Number number)) {
                throw new IllegalArgumentException("render contract transform values must be numbers");
            }
            final long bits;
            try {
                bits = number instanceof BigDecimal decimal
                    ? decimal.longValueExact()
                    : number instanceof BigInteger integer
                        ? integer.longValueExact()
                        : number.longValue();
            } catch (ArithmeticException ex) {
                throw new IllegalArgumentException(
                    "render contract transform values must be in-range integers", ex);
            }
            double decoded = Double.longBitsToDouble(bits);
            if (!Double.isFinite(decoded)) {
                throw new IllegalArgumentException("render contract transform must contain only finite values");
            }
            return decoded;
        }

        private void validateByteObject(String objectName, String... fields) {
            LinkedHashMap<String, Object> object = objectValue(objectName);
            for (String field : fields) {
                checkedByte(requiredLong(object, field), field);
            }
        }

        private LinkedHashMap<String, Object> objectValue(String name) {
            Object value = values.get(name);
            if (!(value instanceof Map<?, ?> map)) {
                throw new IllegalArgumentException("render contract " + name + " must be an object");
            }
            return copyObject(map);
        }

        private long requiredLong(String name) {
            return requiredLong(values, name);
        }

        private static long requiredLong(Map<?, ?> object, String name) {
            Object value = object.get(name);
            if (!(value instanceof Number number)) {
                throw new IllegalArgumentException("render contract " + name + " must be numeric");
            }
            if (number instanceof BigDecimal decimal) {
                try {
                    return decimal.longValueExact();
                } catch (ArithmeticException ex) {
                    throw new IllegalArgumentException(
                        "render contract " + name + " must be an in-range integer", ex);
                }
            }
            return number.longValue();
        }

        private String requiredString(String name) {
            Object value = values.get(name);
            if (!(value instanceof String text) || text.isBlank()) {
                throw new IllegalArgumentException("render contract " + name + " must be a non-empty string");
            }
            return text;
        }

        private <E extends Enum<E>> void requiredEnum(String name, Class<E> enumType) {
            String value = requiredString(name);
            try {
                Enum.valueOf(enumType, value);
            } catch (IllegalArgumentException ex) {
                throw new IllegalArgumentException("unsupported render contract " + name + ": " + value, ex);
            }
        }

        private void requiredBoolean(String name) {
            if (!(values.get(name) instanceof Boolean)) {
                throw new IllegalArgumentException("render contract " + name + " must be boolean");
            }
        }

        private LinkedHashMap<String, Object> copyValues() {
            return copyObject(values);
        }

        private static LinkedHashMap<String, Object> copyObject(Map<?, ?> map) {
            LinkedHashMap<String, Object> copy = new LinkedHashMap<>();
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!(entry.getKey() instanceof String key)) {
                    throw new IllegalArgumentException("render contract object key must be a string");
                }
                copy.put(key, copyJsonValue(entry.getValue()));
            }
            return copy;
        }

        private static Object copyJsonValue(Object value) {
            if (value == null || value instanceof String || value instanceof Boolean) {
                return value;
            }
            if (value instanceof Number number) {
                return number instanceof BigInteger || number instanceof BigDecimal
                    ? number
                    : number.longValue();
            }
            if (value instanceof Map<?, ?> map) {
                return copyObject(map);
            }
            if (value instanceof List<?> list) {
                List<Object> copy = new ArrayList<>(list.size());
                for (Object item : list) {
                    copy.add(copyJsonValue(item));
                }
                return List.copyOf(copy);
            }
            throw new IllegalArgumentException("unsupported render contract JSON value");
        }

        private static long checkedByte(long value, String name) {
            if (value < 0 || value > 255) {
                throw new IllegalArgumentException("render contract byte field out of range: " + name);
            }
            return value;
        }

        private static long requireNonNegative(long value, String name) {
            if (value < 0) {
                throw new IllegalArgumentException(name + " must be non-negative");
            }
            return value;
        }

        private static BigInteger unsignedBits(double value) {
            return new BigInteger(Long.toUnsignedString(Double.doubleToRawLongBits(value)));
        }

        public enum PixelFormat {
            Rgba8,
            Bgra8,
            Rgb8,
            Bgr8,
            Gray8
        }

        public enum AlphaMode {
            Premultiplied,
            Straight,
            Opaque
        }

        public enum PageBox {
            Media,
            Crop,
            Bleed,
            Trim,
            Art
        }

        public enum ExecutionMode {
            Standard,
            Research
        }

        public enum BackendSelection {
            ScalarReference,
            StandardCpu,
            ResearchHybrid
        }

        public enum SmoothingPolicy {
            Disabled,
            Antialiased,
            Subpixel
        }

        public enum AnnotationPolicy {
            Include,
            Exclude
        }

        public enum FormPolicy {
            Include,
            Exclude
        }

        public enum ColorScheme {
            Light,
            Dark,
            ForcedMonochrome
        }

        public enum PrintProfile {
            Display,
            Print,
            Proof
        }

        public enum HalftonePolicy {
            Disabled,
            Screen
        }

        public enum OverprintPolicy {
            Disabled,
            Preview,
            PreserveSeparations
        }

        public enum RenderingIntent {
            RelativeColorimetric,
            AbsoluteColorimetric,
            Perceptual,
            Saturation
        }

        public enum ColorManagementPolicy {
            PortableQcms,
            NativeLittleCms,
            DeterministicFallback
        }

        public enum ExactnessPolicy {
            Compatibility,
            HighQualityExact
        }

        public enum DeterminismPolicy {
            Required,
            BestEffortResearch
        }

        public enum CompositingPolicy {
            Compatibility,
            HighQuality
        }
    }

    private static final class Json {
        private Json() {
        }

        static Object parse(String input) {
            Parser parser = new Parser(input);
            Object value = parser.parseValue();
            parser.skipWhitespace();
            if (!parser.isEof()) {
                throw new IllegalArgumentException("unexpected trailing JSON content");
            }
            return value;
        }

        static String write(Object value) {
            StringBuilder output = new StringBuilder();
            writeValue(output, value);
            return output.toString();
        }

        private static void writeValue(StringBuilder output, Object value) {
            if (value == null) {
                output.append("null");
            } else if (value instanceof String text) {
                writeString(output, text);
            } else if (value instanceof Boolean bool) {
                output.append(bool);
            } else if (value instanceof BigInteger || value instanceof Long || value instanceof Integer
                || value instanceof Short || value instanceof Byte) {
                output.append(value);
            } else if (value instanceof BigDecimal decimal) {
                output.append(decimal.stripTrailingZeros().toPlainString());
            } else if (value instanceof Double || value instanceof Float) {
                double decimal = ((Number) value).doubleValue();
                if (!Double.isFinite(decimal)) {
                    throw new IllegalArgumentException("JSON numbers must be finite");
                }
                output.append(BigDecimal.valueOf(decimal).stripTrailingZeros().toPlainString());
            } else if (value instanceof Number) {
                throw new IllegalArgumentException("unsupported JSON number type");
            } else if (value instanceof Map<?, ?> map) {
                output.append('{');
                boolean first = true;
                for (Map.Entry<?, ?> entry : map.entrySet()) {
                    if (!first) output.append(',');
                    first = false;
                    if (!(entry.getKey() instanceof String key)) {
                        throw new IllegalArgumentException("JSON object key must be a string");
                    }
                    writeString(output, key);
                    output.append(':');
                    writeValue(output, entry.getValue());
                }
                output.append('}');
            } else if (value instanceof Iterable<?> iterable) {
                output.append('[');
                boolean first = true;
                for (Object item : iterable) {
                    if (!first) output.append(',');
                    first = false;
                    writeValue(output, item);
                }
                output.append(']');
            } else {
                throw new IllegalArgumentException("unsupported JSON value type");
            }
        }

        private static void writeString(StringBuilder output, String text) {
            output.append('"');
            for (int index = 0; index < text.length(); index++) {
                char ch = text.charAt(index);
                switch (ch) {
                    case '"' -> output.append("\\\"");
                    case '\\' -> output.append("\\\\");
                    case '\b' -> output.append("\\b");
                    case '\f' -> output.append("\\f");
                    case '\n' -> output.append("\\n");
                    case '\r' -> output.append("\\r");
                    case '\t' -> output.append("\\t");
                    default -> {
                        if (ch < 0x20) {
                            output.append(String.format("\\u%04x", (int) ch));
                        } else {
                            output.append(ch);
                        }
                    }
                }
            }
            output.append('"');
        }

        private static final class Parser {
            private final String input;
            private int index;

            Parser(String input) {
                this.input = Objects.requireNonNull(input, "input");
            }

            Object parseValue() {
                skipWhitespace();
                if (isEof()) throw new IllegalArgumentException("unexpected end of JSON");
                char ch = input.charAt(index);
                return switch (ch) {
                    case '{' -> parseObject();
                    case '[' -> parseArray();
                    case '"' -> parseString();
                    case 't' -> parseLiteral("true", Boolean.TRUE);
                    case 'f' -> parseLiteral("false", Boolean.FALSE);
                    case 'n' -> parseLiteral("null", null);
                    default -> {
                        if (ch == '-' || Character.isDigit(ch)) {
                            yield parseNumber();
                        }
                        throw new IllegalArgumentException("unexpected JSON character at " + index);
                    }
                };
            }

            LinkedHashMap<String, Object> parseObject() {
                expect('{');
                LinkedHashMap<String, Object> object = new LinkedHashMap<>();
                skipWhitespace();
                if (consume('}')) return object;
                while (true) {
                    skipWhitespace();
                    String key = parseString();
                    skipWhitespace();
                    expect(':');
                    object.put(key, parseValue());
                    skipWhitespace();
                    if (consume('}')) return object;
                    expect(',');
                }
            }

            List<Object> parseArray() {
                expect('[');
                ArrayList<Object> array = new ArrayList<>();
                skipWhitespace();
                if (consume(']')) return List.copyOf(array);
                while (true) {
                    array.add(parseValue());
                    skipWhitespace();
                    if (consume(']')) return List.copyOf(array);
                    expect(',');
                }
            }

            String parseString() {
                expect('"');
                StringBuilder text = new StringBuilder();
                while (!isEof()) {
                    char ch = input.charAt(index++);
                    if (ch == '"') {
                        return text.toString();
                    }
                    if (ch == '\\') {
                        if (isEof()) throw new IllegalArgumentException("unterminated JSON escape");
                        char escaped = input.charAt(index++);
                        switch (escaped) {
                            case '"' -> text.append('"');
                            case '\\' -> text.append('\\');
                            case '/' -> text.append('/');
                            case 'b' -> text.append('\b');
                            case 'f' -> text.append('\f');
                            case 'n' -> text.append('\n');
                            case 'r' -> text.append('\r');
                            case 't' -> text.append('\t');
                            case 'u' -> text.append(parseUnicodeEscape());
                            default -> throw new IllegalArgumentException("unsupported JSON escape");
                        }
                    } else {
                        text.append(ch);
                    }
                }
                throw new IllegalArgumentException("unterminated JSON string");
            }

            char parseUnicodeEscape() {
                if (index + 4 > input.length()) {
                    throw new IllegalArgumentException("truncated JSON unicode escape");
                }
                int value = Integer.parseInt(input.substring(index, index + 4), 16);
                index += 4;
                return (char) value;
            }

            Object parseNumber() {
                int start = index;
                if (consume('-')) {
                    if (isEof()) throw new IllegalArgumentException("invalid JSON number");
                }
                if (consume('0')) {
                    // Leading zero is allowed only as the entire integer part.
                } else {
                    while (!isEof() && Character.isDigit(input.charAt(index))) {
                        index++;
                    }
                }
                boolean decimal = false;
                if (!isEof() && input.charAt(index) == '.') {
                    decimal = true;
                    index++;
                    int fractionStart = index;
                    while (!isEof() && Character.isDigit(input.charAt(index))) index++;
                    if (fractionStart == index) throw new IllegalArgumentException("invalid JSON fraction");
                }
                if (!isEof() && (input.charAt(index) == 'e' || input.charAt(index) == 'E')) {
                    decimal = true;
                    index++;
                    if (!isEof() && (input.charAt(index) == '+' || input.charAt(index) == '-')) index++;
                    int exponentStart = index;
                    while (!isEof() && Character.isDigit(input.charAt(index))) index++;
                    if (exponentStart == index) throw new IllegalArgumentException("invalid JSON exponent");
                }
                String text = input.substring(start, index);
                try {
                    if (decimal) return new BigDecimal(text);
                    BigInteger integer = new BigInteger(text);
                    if (integer.bitLength() < 63) {
                        return integer.longValue();
                    }
                    if (integer.signum() >= 0 && integer.bitLength() <= 64) {
                        return integer;
                    }
                    throw new IllegalArgumentException("JSON integer is out of range");
                } catch (NumberFormatException ex) {
                    throw new IllegalArgumentException("invalid JSON number", ex);
                }
            }

            Object parseLiteral(String literal, Object value) {
                if (!input.startsWith(literal, index)) {
                    throw new IllegalArgumentException("invalid JSON literal");
                }
                index += literal.length();
                return value;
            }

            void skipWhitespace() {
                while (!isEof()) {
                    char ch = input.charAt(index);
                    if (ch == ' ' || ch == '\n' || ch == '\r' || ch == '\t') {
                        index++;
                    } else {
                        return;
                    }
                }
            }

            boolean consume(char expected) {
                if (!isEof() && input.charAt(index) == expected) {
                    index++;
                    return true;
                }
                return false;
            }

            void expect(char expected) {
                if (!consume(expected)) {
                    throw new IllegalArgumentException("expected JSON character '" + expected + "' at " + index);
                }
            }

            boolean isEof() {
                return index >= input.length();
            }
        }
    }

    /**
     * Owned Signature Validation signature-validation configuration. Certificates and
     * evidence are copied by the native layer; this handle never contains a
     * private key. Network retrieval stays disabled unless the caller supplies
     * an explicit bounded retrieval-policy JSON object with {@code enabled}.
     */
    public static final class SignatureValidationOptions implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
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
            requireOwnerThread(ownerThread, "SignatureValidationOptions");
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
            requireOwnerThread(ownerThread, "SignatureValidationOptions");
            if (closed) return;
            Native.freeSignatureValidationOptions(handle);
            handle = MemorySegment.NULL;
            closed = true;
        }
    }

    public static final class Document implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;
        private boolean closed;

        private Document(MemorySegment handle) {
            this.handle = handle;
        }

        public static Document open(Path path) throws IOException {
            return open(path, (String) null);
        }

        public static Document open(Path path, String password) throws IOException {
            Objects.requireNonNull(path, "path");
            return open(Files.readAllBytes(path), password);
        }

        public static Document openWithPasswordBytes(Path path, byte[] password) throws IOException {
            Objects.requireNonNull(path, "path");
            return openWithPasswordBytes(Files.readAllBytes(path), password);
        }

        public static Document open(byte[] bytes) {
            return open(bytes, (String) null);
        }

        public static Document open(byte[] bytes, String password) {
            if (password == null) return openWithoutPassword(bytes);
            byte[] passwordBytes = password.getBytes(StandardCharsets.UTF_8);
            try {
                return openWithPasswordBytes(bytes, passwordBytes);
            } finally {
                Arrays.fill(passwordBytes, (byte) 0);
            }
        }

        public static Document openWithPasswordBytes(byte[] bytes, byte[] password) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(password, "password");
            return openNative(bytes, password);
        }

        private static Document openWithoutPassword(byte[] bytes) {
            Objects.requireNonNull(bytes, "bytes");
            return openNative(bytes, null);
        }

        private static Document openNative(byte[] bytes, byte[] password) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment passwordPtr = MemorySegment.NULL;
                long passwordLen = 0;
                if (password != null) {
                    passwordLen = password.length;
                    passwordPtr = arena.allocate(Math.max(password.length, 1));
                    if (password.length > 0) {
                        passwordPtr.copyFrom(MemorySegment.ofArray(password));
                    }
                }
                try {
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
                } finally {
                    if (password != null) passwordPtr.fill((byte) 0);
                }
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

        public void registerFontBytes(String name, byte[] fontBytes) {
            ensureOpen();
            Objects.requireNonNull(name, "name");
            Objects.requireNonNull(fontBytes, "fontBytes");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment namePtr = arena.allocateFrom(name);
                MemorySegment data = arena.allocate(Math.max(fontBytes.length, 1));
                if (fontBytes.length > 0) {
                    data.copyFrom(MemorySegment.ofArray(fontBytes));
                }
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.REGISTER_FONT_BYTES.invokeExact(
                    handle,
                    namePtr,
                    data,
                    (long) fontBytes.length,
                    err
                );
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend register_font_bytes failed", ex);
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

        public byte[] renderPagePng(int pageNumber, int dpi) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG.invokeExact(handle, (long) pageNumber, dpi, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render_page_png failed", ex);
            }
        }

        public byte[] renderPagePng(int pageNumber) {
            return renderPagePng(pageNumber, 72);
        }

        public BinaryResult renderPagePngWithFontSubstitutionReportJson(
            int pageNumber,
            int dpi,
            String mode
        ) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment modePtr = mode == null ? MemorySegment.NULL : arena.allocateFrom(mode);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_FONT_SUBSTITUTION_REPORT.invokeExact(
                    handle, (long) pageNumber, dpi, modePtr, buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render_page_png font substitution report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithFontSubstitutionReportJson(int pageNumber, int dpi) {
            return renderPagePngWithFontSubstitutionReportJson(pageNumber, dpi, "compat");
        }

        public byte[] renderPageJpeg(int pageNumber, int dpi, byte quality) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_JPEG.invokeExact(handle, (long) pageNumber, dpi, quality, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render_page_jpeg failed", ex);
            }
        }

        public String defaultRenderContractJson(int pageNumber, int dpi, String mode) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment modePtr = mode == null ? MemorySegment.NULL : arena.allocateFrom(mode);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.DEFAULT_RENDER_CONTRACT.invokeExact(
                    handle, (long) pageNumber, dpi, modePtr, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend default render contract failed", ex);
            }
        }

        public String defaultRenderContractJson(int pageNumber, int dpi) {
            return defaultRenderContractJson(pageNumber, dpi, "compat");
        }

        public String backendPlanArenaReportJson(int pageNumber, int dpi, String mode) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment modePtr = mode == null ? MemorySegment.NULL : arena.allocateFrom(mode);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.BACKEND_PLAN_ARENA_REPORT.invokeExact(
                    handle, (long) pageNumber, dpi, modePtr, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend backend plan arena report failed", ex);
            }
        }

        public String backendPlanArenaReportJson(int pageNumber, int dpi) {
            return backendPlanArenaReportJson(pageNumber, dpi, "compat");
        }

        public String backendPlanArenaReportForContractJson(String contractJson) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.BACKEND_PLAN_ARENA_REPORT_FOR_CONTRACT.invokeExact(
                    handle, contract, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract backend plan arena report failed", ex);
            }
        }

        public String backendPlanArenaReport(RenderContract contract) {
            Objects.requireNonNull(contract, "contract");
            return backendPlanArenaReportForContractJson(contract.toJson());
        }

        public String prepressPlateReportJson(int pageNumber, int dpi) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1) throw new IllegalArgumentException("page and dpi must be positive");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PREPRESS_PLATE_REPORT.invokeExact(
                    handle, (long) pageNumber, dpi, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend prepress plate report failed", ex);
            }
        }

        public String prepressPlateReportJson(int pageNumber) {
            return prepressPlateReportJson(pageNumber, 72);
        }

        public RenderContract defaultRenderContract(int pageNumber, int dpi, String mode) {
            return RenderContract.fromJson(defaultRenderContractJson(pageNumber, dpi, mode));
        }

        public RenderContract defaultRenderContract(int pageNumber, int dpi) {
            return defaultRenderContract(pageNumber, dpi, "compat");
        }

        public byte[] renderPagePngWithContractJson(String contractJson) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT.invokeExact(
                    handle, contract, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract PNG rendering failed", ex);
            }
        }

        public byte[] renderPagePngWithContractJson(String contractJson, RenderCache cache) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(cache, "cache");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_CACHE.invokeExact(
                    handle, contract, cache.nativeHandle(), buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cached contract PNG rendering failed", ex);
            }
        }

        public byte[] renderPagePngWithContractJson(
            String contractJson,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(cancellation, "cancellation");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable contract PNG rendering failed", ex);
            }
        }

        public byte[] renderPagePng(RenderContract contract) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractJson(contract.toJson());
        }

        public byte[] renderPagePng(RenderContract contract, RenderCache cache) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractJson(contract.toJson(), cache);
        }

        public byte[] renderPagePng(RenderContract contract, RenderCancellation cancellation) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractJson(contract.toJson(), cancellation);
        }

        public BinaryResult renderPagePngWithContractAndFontSubstitutionReportJson(
            String contractJson
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT.invokeExact(
                    handle, contract, buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract PNG font substitution report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithContractAndFontSubstitutionReportJson(
            String contractJson,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(cancellation, "cancellation");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable contract PNG font substitution report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithFontSubstitutionReport(RenderContract contract) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractAndFontSubstitutionReportJson(contract.toJson());
        }

        public BinaryResult renderPagePngWithFontSubstitutionReport(
            RenderContract contract,
            RenderCancellation cancellation
        ) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractAndFontSubstitutionReportJson(contract.toJson(), cancellation);
        }

        public BinaryResult renderPagePngWithContractAndRenderReportJson(
            String contractJson
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_REPORT.invokeExact(
                    handle, contract, buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract PNG render report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithContractAndRenderCacheReportJson(
            String contractJson,
            RenderCache cache
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(cache, "cache");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_CACHE_REPORT.invokeExact(
                    handle, contract, cache.nativeHandle(), buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cached contract PNG render report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithContractAndRenderReportJson(
            String contractJson,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(cancellation, "cancellation");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_REPORT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), buffer, jsonOut, err);
                Native.throwError(status, err);
                return new BinaryResult(Native.takeBuffer(buffer), Native.takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable contract PNG render report failed", ex);
            }
        }

        public BinaryResult renderPagePngWithRenderReport(RenderContract contract) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractAndRenderReportJson(contract.toJson());
        }

        public BinaryResult renderPagePngWithRenderCacheReport(RenderContract contract, RenderCache cache) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractAndRenderCacheReportJson(contract.toJson(), cache);
        }

        public BinaryResult renderPagePngWithRenderReport(
            RenderContract contract,
            RenderCancellation cancellation
        ) {
            Objects.requireNonNull(contract, "contract");
            return renderPagePngWithContractAndRenderReportJson(contract.toJson(), cancellation);
        }

        public void renderPageIntoBufferWithContractJson(String contractJson, ByteBuffer output) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT.invokeExact(
                    handle, contract, surface, surface.byteSize(), err);
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend caller-owned contract rendering failed", ex);
            }
        }

        public void renderPageIntoBufferWithContractJson(
            String contractJson,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            Objects.requireNonNull(cancellation, "cancellation");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), surface, surface.byteSize(), err);
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable caller-owned contract rendering failed", ex);
            }
        }

        public void renderPageIntoBuffer(RenderContract contract, ByteBuffer output) {
            Objects.requireNonNull(contract, "contract");
            renderPageIntoBufferWithContractJson(contract.toJson(), output);
        }

        public void renderPageIntoBuffer(
            RenderContract contract,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            Objects.requireNonNull(contract, "contract");
            renderPageIntoBufferWithContractJson(contract.toJson(), output, cancellation);
        }

        public String renderPageIntoBufferWithContractAndFontSubstitutionReportJson(
            String contractJson,
            ByteBuffer output
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT.invokeExact(
                    handle, contract, surface, surface.byteSize(), jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract buffer font substitution report failed", ex);
            }
        }

        public String renderPageIntoBufferWithContractAndFontSubstitutionReportJson(
            String contractJson,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            Objects.requireNonNull(cancellation, "cancellation");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), surface, surface.byteSize(), jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable contract buffer font substitution report failed", ex);
            }
        }

        public String renderPageIntoBufferWithFontSubstitutionReport(RenderContract contract, ByteBuffer output) {
            Objects.requireNonNull(contract, "contract");
            return renderPageIntoBufferWithContractAndFontSubstitutionReportJson(contract.toJson(), output);
        }

        public String renderPageIntoBufferWithFontSubstitutionReport(
            RenderContract contract,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            Objects.requireNonNull(contract, "contract");
            return renderPageIntoBufferWithContractAndFontSubstitutionReportJson(
                contract.toJson(), output, cancellation);
        }

        public String renderPageIntoBufferWithContractAndRenderReportJson(
            String contractJson,
            ByteBuffer output
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_RENDER_REPORT.invokeExact(
                    handle, contract, surface, surface.byteSize(), jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend contract buffer render report failed", ex);
            }
        }

        public String renderPageIntoBufferWithContractAndRenderReportJson(
            String contractJson,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            Objects.requireNonNull(output, "output");
            Objects.requireNonNull(cancellation, "cancellation");
            if (!output.isDirect()) {
                throw new IllegalArgumentException("caller-owned render output must be a direct ByteBuffer");
            }
            if (output.isReadOnly()) {
                throw new IllegalArgumentException("caller-owned render output must be writable");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment surface = MemorySegment.ofBuffer(output);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_RENDER_REPORT_AND_CANCELLATION.invokeExact(
                    handle, contract, cancellation.nativeHandle(), surface, surface.byteSize(), jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable contract buffer render report failed", ex);
            }
        }

        public String renderPageIntoBufferWithRenderReport(RenderContract contract, ByteBuffer output) {
            Objects.requireNonNull(contract, "contract");
            return renderPageIntoBufferWithContractAndRenderReportJson(contract.toJson(), output);
        }

        public String renderPageIntoBufferWithRenderReport(
            RenderContract contract,
            ByteBuffer output,
            RenderCancellation cancellation
        ) {
            Objects.requireNonNull(contract, "contract");
            return renderPageIntoBufferWithContractAndRenderReportJson(
                contract.toJson(), output, cancellation);
        }

        public ProgressiveRenderSession progressiveRenderSession(
            int pageNumber, int dpi, int tileWidth, int tileHeight, String mode
        ) {
            ensureOpen();
            if (pageNumber < 1 || dpi < 1 || (tileWidth == 0) != (tileHeight == 0)) {
                throw new IllegalArgumentException("page and dpi must be positive; tile dimensions must both be positive or both be zero for adaptive sizing");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment modePtr = mode == null ? MemorySegment.NULL : arena.allocateFrom(mode);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment session = (MemorySegment) Native.PROGRESSIVE_NEW.invokeExact(
                    handle, (long) pageNumber, dpi, tileWidth, tileHeight, modePtr, err);
                if (Native.isNull(session)) {
                    Native.throwError(2, err);
                }
                return new ProgressiveRenderSession(session);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive session creation failed", ex);
            }
        }

        public ProgressiveRenderSession progressiveRenderSession(
            RenderContract contract, int tileWidth, int tileHeight
        ) {
            ensureOpen();
            Objects.requireNonNull(contract, "contract");
            if ((tileWidth == 0) != (tileHeight == 0)) {
                throw new IllegalArgumentException("tile dimensions must both be positive or both be zero for adaptive sizing");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contractPtr = arena.allocateFrom(contract.toJson());
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment session = (MemorySegment) Native.PROGRESSIVE_NEW_WITH_CONTRACT_JSON.invokeExact(
                    handle, contractPtr, tileWidth, tileHeight, err);
                if (Native.isNull(session)) {
                    Native.throwError(2, err);
                }
                return new ProgressiveRenderSession(session);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive contract session creation failed", ex);
            }
        }

        public ProgressiveRenderSession progressiveRenderSession(RenderContract contract) {
            return progressiveRenderSession(contract, 256, 256);
        }

        public ProgressiveRenderSession progressiveRenderSession(int pageNumber) {
            return progressiveRenderSession(pageNumber, 72, 256, 256, "compat");
        }

        public ProgressiveRenderSession adaptiveProgressiveRenderSession(
            int pageNumber, int dpi, String mode
        ) {
            return progressiveRenderSession(pageNumber, dpi, 0, 0, mode);
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

        public String documentViewsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.DOCUMENT_VIEWS_REPORT, "document_views_report");
        }

        public String imageDecodeCapabilityReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle,
                Native.IMAGE_DECODE_CAPABILITY_REPORT,
                "image_decode_capability_report");
        }

        public String progressiveImageDecodeLifecycleReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle,
                Native.PROGRESSIVE_IMAGE_DECODE_LIFECYCLE_REPORT,
                requestJson,
                "progressive_image_decode_lifecycle_report");
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

        public String annotation_media_redactionReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ANNOTATION_MEDIA_REDACTION_REPORT, "annotation_media_redaction_report");
        }

        public String secure_mutationReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SECURE_MUTATION_REPORT, "secure_mutation_report");
        }

        public String secure_mutation_closeoutReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SECURE_MUTATION_CLOSEOUT_REPORT, "secure_mutation_closeout_report");
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

        public String form_action_policyReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.FORM_ACTION_POLICY_REPORT, "form_action_policy_report");
        }

        public String advanced_editingReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ADVANCED_EDITING_REPORT, "advanced_editing_report");
        }

        public String advanced_editing_closeoutReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.ADVANCED_EDITING_CLOSEOUT_REPORT, "advanced_editing_closeout_report");
        }

        public String source_editingReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.SOURCE_EDITING_REPORT, "source_editing_report");
        }

        public String universalEditingAnalyzeV2Json(String optionsJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.UNIVERSAL_EDITING_ANALYZE_V2, optionsJson,
                "universal_editing_analyze_v2");
        }

        public String universalEditingAnalyzeV2Json() {
            return universalEditingAnalyzeV2Json(null);
        }

        public String universalRenderQualificationV2Json(String optionsJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.UNIVERSAL_RENDER_QUALIFICATION_V2, optionsJson,
                "universal_render_qualification_v2");
        }

        public String universalRenderQualificationV2Json() {
            return universalRenderQualificationV2Json(null);
        }

        public String universalEditingInspectObjectV2Json(long objectNumber, int generation) {
            ensureOpen();
            return Native.documentObjectReport(
                handle, objectNumber, generation, "universal_editing_inspect_object_v2");
        }

        public String universalEditingInspectObjectV2Json(long objectNumber) {
            return universalEditingInspectObjectV2Json(objectNumber, 0);
        }

        public String universalEditingPlanV2Json(String requestJson) {
            ensureOpen();
            Objects.requireNonNull(requestJson, "requestJson");
            return Native.documentStringReport(
                handle, Native.UNIVERSAL_EDITING_PLAN_V2, requestJson,
                "universal_editing_plan_v2");
        }

        /** Preview a revision-bound transfer of one saved native Figure between saved stories. */
        public String storyFigureTransferPreviewJson(String requestJson) {
            ensureOpen();
            Objects.requireNonNull(requestJson, "requestJson");
            return Native.documentStringReport(
                handle, Native.STORY_FIGURE_TRANSFER_PREVIEW, requestJson,
                "story_figure_transfer_preview");
        }

        /** Apply the exact planSha256 returned by storyFigureTransferPreviewJson. */
        public BinaryResult storyFigureTransferApply(
            String requestJson,
            String approvedPlanSha256
        ) {
            ensureOpen();
            Objects.requireNonNull(requestJson, "requestJson");
            Objects.requireNonNull(approvedPlanSha256, "approvedPlanSha256");
            return Native.documentTwoStringOutput(
                handle, Native.STORY_FIGURE_TRANSFER_APPLY, requestJson,
                approvedPlanSha256, "story_figure_transfer_apply");
        }

        /** Read-only PNG byte arrays for a canonical scoped-text candidate; no PDF publication. */
        public String universalEditingScopedPreviewV2Json(String planJson, String optionsJson) {
            ensureOpen();
            Objects.requireNonNull(planJson, "planJson");
            return Native.documentTwoStringReport(handle, Native.UNIVERSAL_EDITING_SCOPED_PREVIEW_V2,
                planJson, optionsJson, "universal_editing_scoped_preview_v2");
        }

        public String universalEditingScopedPreviewV2Json(String planJson) {
            return universalEditingScopedPreviewV2Json(planJson, null);
        }

        public BinaryResult universalEditingApplyV2(
            String planJson,
            String approvalJson
        ) {
            ensureOpen();
            Objects.requireNonNull(planJson, "planJson");
            return Native.documentTwoStringOutput(
                handle, Native.UNIVERSAL_EDITING_APPLY_V2, planJson,
                approvalJson, "universal_editing_apply_v2");
        }

        public BinaryResult universalEditingApplyV2(String planJson) {
            return universalEditingApplyV2(planJson, null);
        }

        public BinaryResult universalEditingApplyV2WithOutputCredentials(
            String planJson,
            String outputUserPassword,
            String outputOwnerPassword,
            String approvalJson
        ) {
            Objects.requireNonNull(outputUserPassword, "outputUserPassword");
            byte[] userBytes = outputUserPassword.getBytes(StandardCharsets.UTF_8);
            byte[] ownerBytes = outputOwnerPassword == null
                ? null
                : outputOwnerPassword.getBytes(StandardCharsets.UTF_8);
            try {
                return universalEditingApplyV2WithOutputCredentialBytes(
                    planJson, userBytes, ownerBytes, approvalJson);
            } finally {
                Arrays.fill(userBytes, (byte) 0);
                if (ownerBytes != null) Arrays.fill(ownerBytes, (byte) 0);
            }
        }

        public BinaryResult universalEditingApplyV2WithOutputCredentialBytes(
            String planJson,
            byte[] outputUserPassword,
            byte[] outputOwnerPassword,
            String approvalJson
        ) {
            ensureOpen();
            Objects.requireNonNull(planJson, "planJson");
            Objects.requireNonNull(outputUserPassword, "outputUserPassword");
            return Native.universalEditingApplyWithOutputCredentialBytes(
                handle, planJson, approvalJson, outputUserPassword,
                outputOwnerPassword, "universal_editing_apply_v2_with_output_credential_bytes");
        }

        /** Execute ECBES over multiple canonical universal-edit candidates. */
        public BinaryResult ecbesUniversalEdit(String requestJson) {
            ensureOpen();
            Objects.requireNonNull(requestJson, "requestJson");
            return Native.documentStringOutput(
                handle, Native.ECBES_UNIVERSAL_EDIT, requestJson, "ecbes_universal_edit");
        }

        /** Execute ECBES with binary-safe apply-only output credentials. */
        public BinaryResult ecbesUniversalEditWithOutputCredentialBytes(
            String requestJson,
            byte[] outputUserPassword,
            byte[] outputOwnerPassword
        ) {
            ensureOpen();
            Objects.requireNonNull(requestJson, "requestJson");
            Objects.requireNonNull(outputUserPassword, "outputUserPassword");
            return Native.ecbesUniversalEditWithOutputCredentialBytes(
                handle, requestJson, outputUserPassword, outputOwnerPassword,
                "ecbes_universal_edit_with_output_credential_bytes");
        }

        public String editing_transactionsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.EDITING_TRANSACTIONS_REPORT, "editing_transactions_report");
        }

        public String text_reflowReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.TEXT_REFLOW_REPORT, "text_reflow_report");
        }

        public String writer_historyReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.WRITER_HISTORY_REPORT, "writer_history_report");
        }

        public String writer_historyRasterVectorReportJson(long page, String optionsJson) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.writer_historyRasterVectorReport(handle, page, optionsJson);
        }

        public String writer_historyFontReconstructionReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.WRITER_HISTORY_FONT_RECONSTRUCTION_REPORT, "writer_history_font_reconstruction_report");
        }

        public String writer_historyObjectStreamReportJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.WRITER_HISTORY_OBJECT_STREAM_REPORT, "writer_history_object_stream_report");
        }

        public BinaryResult writer_historyPackObjectStreams() {
            ensureOpen();
            return Native.documentOutput(handle, Native.WRITER_HISTORY_PACK_OBJECT_STREAMS, "writer_history_pack_object_streams");
        }

        public String compression_officeReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.COMPRESSION_OFFICE_REPORT, "compression_office_report");
        }

        public String crypto_writerReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.CRYPTO_WRITER_REPORT, "crypto_writer_report");
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

        public BinaryResult compression_officeOptimize(String optionsJson) {
            ensureOpen();
            return Native.documentStringOutput(handle, Native.COMPRESSION_OFFICE_OPTIMIZE, optionsJson, "compression_office_optimize");
        }

        public String advanced_editing_closeoutTextRangeAnalyzeJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.advanced_editing_closeoutTextRangeAnalyze(handle, page);
        }

        /**
         * Produces a non-mutating, exact-revision proposal that apportions a
         * multi-run replacement across its original PDF paint slots.
         */
        public String proposeTextRangePaintPartitions(String requestJson) {
            ensureOpen();
            return Native.advanced_editing_closeoutPaintPartitionPropose(handle, requestJson);
        }

        public PaintPartitionProposal proposeTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request
        ) {
            Objects.requireNonNull(request, "request");
            return PaintPartitionProposal.fromEnvelopeJson(
                proposeTextRangePaintPartitions(request.toJson()));
        }

        /** Renders bounded before/candidate PNG byte arrays without returning
         * or publishing the candidate PDF bytes. */
        public String previewTextRangePaintPartitions(
            String requestJson, String proposalJson, String approvalJson
        ) {
            return previewTextRangePaintPartitions(
                requestJson, proposalJson, approvalJson, null, null);
        }

        public String previewTextRangePaintPartitions(
            String requestJson,
            String proposalJson,
            String approvalJson,
            byte[] fontBytes,
            String optionsJson
        ) {
            ensureOpen();
            return Native.advanced_editing_closeoutPaintPartitionPreview(
                handle, requestJson, proposalJson, approvalJson, fontBytes, optionsJson);
        }

        public PaintPartitionPreview previewTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval,
            byte[] fontBytes,
            String optionsJson
        ) {
            Objects.requireNonNull(request, "request");
            Objects.requireNonNull(proposal, "proposal");
            Objects.requireNonNull(approval, "approval");
            requireSameProposal(proposal, approval);
            return PaintPartitionPreview.fromEnvelopeJson(previewTextRangePaintPartitions(
                request.toJson(), proposal.toJson(), approval.toJson(), fontBytes, optionsJson));
        }

        public PaintPartitionPreview previewTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval
        ) {
            return previewTextRangePaintPartitions(request, proposal, approval, null, null);
        }

        public BinaryResult editTextRange(String requestJson) {
            ensureOpen();
            return Native.advanced_editing_closeoutTextRangeEdit(handle, requestJson);
        }

        /** Applies reviewed regions/final lines only when request, proposal and
         * input revision still match the proposal identity exactly. */
        public BinaryResult applyTextRangePaintPartitions(
            String requestJson, String proposalJson, String approvalJson
        ) {
            return applyTextRangePaintPartitions(
                requestJson, proposalJson, approvalJson, null);
        }

        public BinaryResult applyTextRangePaintPartitions(
            String requestJson,
            String proposalJson,
            String approvalJson,
            byte[] fontBytes
        ) {
            ensureOpen();
            return Native.advanced_editing_closeoutPaintPartitionApply(
                handle, requestJson, proposalJson, approvalJson, fontBytes);
        }

        public BinaryResult applyTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval,
            byte[] fontBytes
        ) {
            Objects.requireNonNull(request, "request");
            Objects.requireNonNull(proposal, "proposal");
            Objects.requireNonNull(approval, "approval");
            requireSameProposal(proposal, approval);
            return applyTextRangePaintPartitions(
                request.toJson(), proposal.toJson(), approval.toJson(), fontBytes);
        }

        public BinaryResult applyTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval
        ) {
            return applyTextRangePaintPartitions(request, proposal, approval, null);
        }

        /** Applies only the candidate covered by the canonical preview's
         * publication receipt. */
        public BinaryResult applyReviewedTextRangePaintPartitions(
            String requestJson,
            String proposalJson,
            String approvalJson,
            String publicationReceiptJson,
            byte[] fontBytes
        ) {
            ensureOpen();
            return Native.advanced_editing_closeoutPaintPartitionApplyReviewed(
                handle, requestJson, proposalJson, approvalJson,
                publicationReceiptJson, fontBytes);
        }

        public BinaryResult applyReviewedTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval,
            PaintPartitionPublicationReceipt publicationReceipt,
            byte[] fontBytes
        ) {
            Objects.requireNonNull(request, "request");
            Objects.requireNonNull(proposal, "proposal");
            Objects.requireNonNull(approval, "approval");
            Objects.requireNonNull(publicationReceipt, "publicationReceipt");
            requireSameProposal(proposal, approval);
            if (!publicationReceipt.proposalId().equals(proposal.proposalId()))
                throw new IllegalArgumentException("publication receipt belongs to a different proposal");
            return applyReviewedTextRangePaintPartitions(
                request.toJson(), proposal.toJson(), approval.toJson(),
                publicationReceipt.toJson(), fontBytes);
        }

        public BinaryResult applyReviewedTextRangePaintPartitions(
            PaintPartitionTextRangeRequest request,
            PaintPartitionProposal proposal,
            PaintPartitionApproval approval,
            PaintPartitionPublicationReceipt publicationReceipt
        ) {
            return applyReviewedTextRangePaintPartitions(
                request, proposal, approval, publicationReceipt, null);
        }

        public String authoredTypedTableSourcesJson() {
            ensureOpen();
            return Native.documentReport(
                handle, Native.AUTHORED_TYPED_TABLE_SOURCES, "authored_typed_table_sources");
        }

        public BinaryResult mutateAuthoredTypedTable(String requestJson) {
            return mutateAuthoredTypedTable(requestJson, null);
        }

        public BinaryResult mutateAuthoredTypedTable(String requestJson, byte[] fontBytes) {
            ensureOpen();
            return Native.authoredTypedTableMutate(handle, requestJson, fontBytes);
        }

        public String source_editingProvenanceJson(long page, String sourceText, String replacementText) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.source_editingProvenance(handle, page, sourceText, replacementText);
        }

        public String source_editingEditEligibilityJson(String requestJson) {
            ensureOpen();
            return Native.source_editingEditEligibility(handle, requestJson);
        }

        public BinaryResult source_editingOperatorTextEdit(String requestJson) {
            ensureOpen();
            return Native.source_editingOperatorTextEdit(handle, requestJson);
        }

        public String source_editingPathProvenanceJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.source_editingPathProvenance(handle, page);
        }

        public BinaryResult source_editingPathEdit(
            long page, String stableId, String operationJson, String optionsJson
        ) {
            ensureOpen();
            return Native.source_editingPathEdit(handle, page, stableId, operationJson, optionsJson);
        }

        public String editing_transactionsSceneReportJson(String pagesJson) {
            ensureOpen();
            return Native.editing_transactionsSceneReport(handle, pagesJson);
        }

        public String editing_transactionsSceneSelectJson(String requestJson) {
            ensureOpen();
            return Native.editing_transactionsSceneSelect(handle, requestJson);
        }

        public String editing_transactionsTransactionPlanJson(String requestJson) {
            ensureOpen();
            return Native.editing_transactionsTransactionPlan(handle, requestJson);
        }

        public BinaryResult editing_transactionsTransactionApply(String requestJson) {
            ensureOpen();
            return Native.editing_transactionsTransactionApply(handle, requestJson);
        }

        public BinaryResult editing_transactionsTransactionApplyWithRenderInvalidation(
                String requestJson,
                String renderInvalidationOptionsJson) {
            ensureOpen();
            return Native.editing_transactionsTransactionApplyWithRenderInvalidation(
                handle,
                requestJson,
                renderInvalidationOptionsJson
            );
        }

        public String editing_transactionsTextMapJson(String text, String direction) {
            ensureOpen();
            return Native.editing_transactionsTextMap(handle, text, direction);
        }

        public String editing_transactionsShapeTextJson(String text, String direction) {
            ensureOpen();
            return Native.editing_transactionsShapeText(handle, text, direction);
        }

        public String editing_transactionsFontSubsetPlanJson(String text, String direction, String policy) {
            ensureOpen();
            return Native.editing_transactionsFontSubsetPlan(handle, text, direction, policy);
        }

        public String editing_transactionsFontSubstitutionReportJson(String requestedFamily, String text, String policy) {
            ensureOpen();
            return Native.editing_transactionsFontSubstitutionReport(handle, requestedFamily, text, policy);
        }

        public String text_reflowLayoutAnalyzeJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_LAYOUT_ANALYZE, requestJson, "text_reflow_layout_analyze");
        }

        public String text_reflowSemanticLayoutJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.TEXT_REFLOW_SEMANTIC_LAYOUT, "text_reflow_semantic_layout");
        }

        public String text_reflowReadingOrderReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.TEXT_REFLOW_READING_ORDER, "text_reflow_reading_order_report");
        }

        public String text_reflowFlowGraphReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.TEXT_REFLOW_FLOW_GRAPH, "text_reflow_flow_graph_report");
        }

        public String text_reflowReflowPreviewJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_REFLOW_PREVIEW, requestJson, "text_reflow_reflow_preview");
        }

        public String text_reflowOverflowReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_OVERFLOW_REPORT, requestJson, "text_reflow_overflow_report");
        }

        public String text_reflowConstraintsReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_CONSTRAINTS_REPORT, requestJson, "text_reflow_constraints_report");
        }

        public String text_reflowConfidenceReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_CONFIDENCE_REPORT, requestJson, "text_reflow_confidence_report");
        }

        /**
         * Validates explicit text reflow output bytes against this immutable
         * source document. The caller retains ownership of {@code outputPdf}.
         */
        public String text_reflowValidateReflowOutputJson(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.text_reflowValidateReflowOutput(
                handle, outputPdf, requestJson, "text_reflow_validate_reflow_output");
        }

        public BinaryResult text_reflowReflowRegion(String requestJson) {
            ensureOpen();
            return Native.text_reflowRequestOutput(handle, Native.TEXT_REFLOW_REFLOW_REGION, requestJson, "text_reflow_reflow_region");
        }

        public BinaryResult text_reflowReflowDocument(String requestJson) {
            ensureOpen();
            return Native.text_reflowRequestOutput(handle, Native.TEXT_REFLOW_REFLOW_DOCUMENT, requestJson, "text_reflow_reflow_document");
        }

        /**
         * Replays the specified text reflow operation against this immutable
         * preimage, verifies {@code outputPdf}, and executes its canonical
         * transaction undo. A stale output buffer is rejected.
         */
        public BinaryResult text_reflowUndoReflow(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.text_reflowUndoReflow(handle, outputPdf, requestJson, "text_reflow_undo_reflow");
        }

        public String text_reflowReflowApproveStructureJson(String correctionJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_APPROVE_STRUCTURE, correctionJson, "text_reflow_reflow_approve_structure");
        }

        public String text_reflowReflowOperationReportJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(
                handle, Native.TEXT_REFLOW_OPERATION_REPORT, requestJson, "text_reflow_reflow_operation_report");
        }

        public String document_subsystemsReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.DOCUMENT_SUBSYSTEMS_REPORT, "document_subsystems_report");
        }

        public String document_subsystemsAnalyzeJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.DOCUMENT_SUBSYSTEMS_ANALYZE, "document_subsystems_analyze");
        }

        public String document_subsystemsPlanJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.DOCUMENT_SUBSYSTEMS_PLAN, requestJson, "document_subsystems_plan");
        }

        public BinaryResult document_subsystemsApply(String requestJson) {
            ensureOpen();
            return Native.text_reflowRequestOutput(handle, Native.DOCUMENT_SUBSYSTEMS_APPLY, requestJson, "document_subsystems_apply");
        }

        public BinaryResult document_subsystemsUndo(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.document_subsystemsUndo(handle, outputPdf, requestJson, "document_subsystems_undo");
        }

        public String document_securityReportJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.DOCUMENT_SECURITY_REPORT, "document_security_report");
        }

        public String document_securityAnalyzeJson() {
            ensureOpen();
            return Native.documentReport(handle, Native.DOCUMENT_SECURITY_ANALYZE, "document_security_analyze");
        }

        public String document_securityPlanJson(String requestJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.DOCUMENT_SECURITY_PLAN, requestJson, "document_security_plan");
        }

        public BinaryResult document_securityApply(String requestJson) {
            ensureOpen();
            return Native.text_reflowRequestOutput(handle, Native.DOCUMENT_SECURITY_APPLY, requestJson, "document_security_apply");
        }

        public BinaryResult document_securityUndo(byte[] outputPdf, String requestJson) {
            ensureOpen();
            return Native.document_securityUndo(handle, outputPdf, requestJson, "document_security_undo");
        }

        public String document_securityVerifyResidualJson(String termsJson) {
            ensureOpen();
            return Native.documentStringReport(handle, Native.DOCUMENT_SECURITY_VERIFY_RESIDUAL, termsJson, "document_security_verify_residual");
        }

        public String advanced_editingVectorListJson(long page) {
            ensureOpen();
            if (page < 1) throw new IllegalArgumentException("page must be one-based");
            return Native.advanced_editingVectorList(handle, page);
        }

        public BinaryResult advanced_editingTextEdit(
            long page, String oldText, String newText, String mode, String optionsJson
        ) {
            ensureOpen();
            return Native.advanced_editingTextEdit(handle, page, oldText, newText, mode, optionsJson);
        }

        public BinaryResult advanced_editingVectorEdit(
            long page, String stableId, String operationJson, String optionsJson
        ) {
            ensureOpen();
            return Native.advanced_editingVectorEdit(handle, page, stableId, operationJson, optionsJson);
        }

        public BinaryResult advanced_editingInkFit(
            long page, long annotationIndex, String optionsJson, boolean signaturePolicyOverride
        ) {
            ensureOpen();
            return Native.advanced_editingInkFit(
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
            requireOwnerThread(ownerThread, "Document");
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
            requireOwnerThread(ownerThread, "Document");
            if (closed) {
                throw new IllegalStateException("Document is closed");
            }
        }
    }

    public record AdjacentPagePrefetchExecution(
        String reportJson,
        ProgressiveRenderSession session
    ) {}

    public static final class ProgressiveRenderSession implements AutoCloseable {
        private final Thread ownerThread = Thread.currentThread();
        private MemorySegment handle;
        private boolean closed;

        private ProgressiveRenderSession(MemorySegment handle) {
            this.handle = handle;
        }

        public String stepJson(long maxTiles) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_STEP.invokeExact(handle, maxTiles, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive step failed", ex);
            }
        }

        public String stepJson(long maxTiles, BooleanSupplier cancellationRequested) {
            ensureOpen();
            Objects.requireNonNull(cancellationRequested, "cancellationRequested");
            if (cancellationRequested.getAsBoolean()) {
                requestCancel();
                throw new CancellationException("Wellfriend progressive render step was cancelled");
            }
            String report = stepJson(maxTiles);
            if (cancellationRequested.getAsBoolean()) {
                requestCancel();
                throw new CancellationException("Wellfriend progressive render step was cancelled");
            }
            return report;
        }

        public String pauseJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_PAUSE.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive pause failed", ex);
            }
        }

        public String tokenJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_TOKEN.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive token failed", ex);
            }
        }

        public void resumeJson(String tokenJson) {
            ensureOpen();
            Objects.requireNonNull(tokenJson, "tokenJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment token = arena.allocateFrom(tokenJson);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_RESUME.invokeExact(handle, token, err);
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive resume failed", ex);
            }
        }

        public void cancel() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_CANCEL.invokeExact(handle, err);
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive cancel failed", ex);
            }
        }

        public void requestCancel() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REQUEST_CANCEL.invokeExact(handle, err);
                Native.throwError(status, err);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive request-cancel failed", ex);
            }
        }

        public String reviseViewportHintJson(int x, int y, int width, int height) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_VIEWPORT_HINT.invokeExact(
                    handle, 1, x, y, width, height, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive viewport revision failed", ex);
            }
        }

        public String clearViewportHintJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_VIEWPORT_HINT.invokeExact(
                    handle, 0, 0, 0, 0, 0, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive viewport revision failed", ex);
            }
        }

        public String reviseDirtyRegionJson(int x, int y, int width, int height) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_DIRTY_REGION.invokeExact(
                    handle, 1, x, y, width, height, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive dirty-region revision failed", ex);
            }
        }

        public String clearDirtyRegionJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_DIRTY_REGION.invokeExact(
                    handle, 0, 0, 0, 0, 0, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive dirty-region revision failed", ex);
            }
        }

        public String reviseRenderContextJson(
                String renderContractFingerprint,
                String visibilityFingerprint) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = renderContractFingerprint == null
                    ? MemorySegment.NULL
                    : arena.allocateFrom(renderContractFingerprint);
                MemorySegment visibility = visibilityFingerprint == null
                    ? MemorySegment.NULL
                    : arena.allocateFrom(visibilityFingerprint);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_RENDER_CONTEXT.invokeExact(
                    handle, contract, visibility, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive render-context revision failed", ex);
            }
        }

        public String reviseRenderContractJson(String contractJson) {
            ensureOpen();
            Objects.requireNonNull(contractJson, "contractJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment contract = arena.allocateFrom(contractJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_REVISE_RENDER_CONTRACT.invokeExact(
                    handle, contract, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive render-contract revision failed", ex);
            }
        }

        public String reviseRenderContract(RenderContract contract) {
            Objects.requireNonNull(contract, "contract");
            return reviseRenderContractJson(contract.toJson());
        }

        public String evaluateTilePublicationJson(String publicationJson) {
            ensureOpen();
            Objects.requireNonNull(publicationJson, "publicationJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment publication = arena.allocateFrom(publicationJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_EVALUATE_TILE_PUBLICATION.invokeExact(
                    handle, publication, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive tile-publication evaluation failed", ex);
            }
        }

        public String applyRenderInvalidationPlanJson(String planJson) {
            ensureOpen();
            Objects.requireNonNull(planJson, "planJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment plan = arena.allocateFrom(planJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_APPLY_RENDER_INVALIDATION_PLAN.invokeExact(
                    handle, plan, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive render invalidation failed", ex);
            }
        }

        public String viewerQueueJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_VIEWER_QUEUE.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive viewer queue failed", ex);
            }
        }

        public String executeViewerQueueJson() {
            return executeViewerQueueJson(1);
        }

        public String executeViewerQueueJson(long maxItems) {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_EXECUTE_VIEWER_QUEUE.invokeExact(
                    handle, maxItems, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive viewer queue execution failed", ex);
            }
        }

        public String executeViewerQueueJson(RenderCancellation cancellation) {
            return executeViewerQueueJson(1, cancellation);
        }

        public String executeViewerQueueJson(long maxItems, RenderCancellation cancellation) {
            ensureOpen();
            Objects.requireNonNull(cancellation, "cancellation");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_EXECUTE_VIEWER_QUEUE_AND_CANCELLATION.invokeExact(
                    handle, maxItems, cancellation.nativeHandle(), jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable progressive viewer queue execution failed", ex);
            }
        }

        public AdjacentPagePrefetchExecution executeAdjacentPagePrefetch(String prefetchIdentity) {
            return executeAdjacentPagePrefetch(prefetchIdentity, 1);
        }

        public AdjacentPagePrefetchExecution executeAdjacentPagePrefetch(
                String prefetchIdentity,
                long maxTiles) {
            ensureOpen();
            Objects.requireNonNull(prefetchIdentity, "prefetchIdentity");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment identity = arena.allocateFrom(prefetchIdentity);
                MemorySegment childOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_EXECUTE_ADJACENT_PAGE_PREFETCH.invokeExact(
                    handle, identity, maxTiles, childOut, jsonOut, err);
                Native.throwError(status, err);
                String report = Native.takeString(jsonOut);
                MemorySegment child = childOut.get(ValueLayout.ADDRESS, 0);
                ProgressiveRenderSession session = Native.isNull(child)
                    ? null
                    : new ProgressiveRenderSession(child);
                return new AdjacentPagePrefetchExecution(report, session);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive adjacent-page prefetch failed", ex);
            }
        }

        public AdjacentPagePrefetchExecution executeAdjacentPagePrefetch(
                String prefetchIdentity,
                RenderCancellation cancellation) {
            return executeAdjacentPagePrefetch(prefetchIdentity, 1, cancellation);
        }

        public AdjacentPagePrefetchExecution executeAdjacentPagePrefetch(
                String prefetchIdentity,
                long maxTiles,
                RenderCancellation cancellation) {
            ensureOpen();
            Objects.requireNonNull(prefetchIdentity, "prefetchIdentity");
            Objects.requireNonNull(cancellation, "cancellation");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment identity = arena.allocateFrom(prefetchIdentity);
                MemorySegment childOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_EXECUTE_ADJACENT_PAGE_PREFETCH_AND_CANCELLATION.invokeExact(
                    handle, identity, maxTiles, cancellation.nativeHandle(), childOut, jsonOut, err);
                Native.throwError(status, err);
                String report = Native.takeString(jsonOut);
                MemorySegment child = childOut.get(ValueLayout.ADDRESS, 0);
                ProgressiveRenderSession session = Native.isNull(child)
                    ? null
                    : new ProgressiveRenderSession(child);
                return new AdjacentPagePrefetchExecution(report, session);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend cancellable progressive adjacent-page prefetch failed", ex);
            }
        }

        public String viewerCallbackDispatchJson() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_VIEWER_CALLBACK_DISPATCH.invokeExact(handle, jsonOut, err);
                Native.throwError(status, err);
                return Native.takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive viewer callback dispatch failed", ex);
            }
        }

        public String dispatchViewerCallbacks(Consumer<String> callback) {
            ensureOpen();
            Objects.requireNonNull(callback, "callback");
            String report = viewerCallbackDispatchJson();
            Object parsed = Json.parse(report);
            if (parsed instanceof Map<?, ?> map) {
                Object events = map.get("events");
                if (events instanceof List<?> list) {
                    for (Object event : list) {
                        callback.accept(Json.write(event));
                    }
                }
            }
            return report;
        }

        public byte[] finishPng() {
            ensureOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment buffer = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) Native.PROGRESSIVE_FINISH.invokeExact(handle, buffer, err);
                Native.throwError(status, err);
                return Native.takeBuffer(buffer);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive finish failed", ex);
            }
        }

        public byte[] finishPng(BooleanSupplier cancellationRequested) {
            ensureOpen();
            Objects.requireNonNull(cancellationRequested, "cancellationRequested");
            if (cancellationRequested.getAsBoolean()) {
                requestCancel();
                throw new CancellationException("Wellfriend progressive render finish was cancelled");
            }
            byte[] png = finishPng();
            if (cancellationRequested.getAsBoolean()) {
                requestCancel();
                throw new CancellationException("Wellfriend progressive render finish was cancelled");
            }
            return png;
        }

        @Override
        public void close() {
            requireOwnerThread(ownerThread, "ProgressiveRenderSession");
            if (closed) return;
            try {
                Native.PROGRESSIVE_FREE.invokeExact(handle);
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend progressive session release failed", ex);
            } finally {
                handle = MemorySegment.NULL;
                closed = true;
            }
        }

        private void ensureOpen() {
            requireOwnerThread(ownerThread, "ProgressiveRenderSession");
            if (closed || Native.isNull(handle)) {
                throw new IllegalStateException("ProgressiveRenderSession is closed");
            }
        }
    }

    public record Page(Document document, int number) {
        public String text() {
            return document.extractText(number);
        }

        public byte[] renderPng(int dpi) {
            return document.renderPagePng(number, dpi);
        }

        public byte[] renderPng() {
            return renderPng(72);
        }

        public byte[] renderJpeg(int dpi, byte quality) {
            return document.renderPageJpeg(number, dpi, quality);
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

        public static String compression_officeInspectJson(byte[] bytes, String format) {
            return Native.compression_officeOfficeInspect(bytes, format);
        }

        public static BinaryResult compression_officeToPdf(byte[] bytes, String format) {
            return Native.compression_officeOfficeToPdf(bytes, format);
        }
    }

    // Resolve the new session ABI only when requested, not for old document users.
    private static final class StoryNative {
        static {
            if (ValueLayout.ADDRESS.byteSize() != 8)
                throw new UnsupportedOperationException("Java story binding requires a 64-bit native ABI");
        }
        private static final MethodHandle OPEN = Native.downcall("wellfriendpdf_story_session_open",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
        private static final MethodHandle COMMAND = Native.downcall("wellfriendpdf_story_session_command_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
        private static final MethodHandle BYTES = Native.downcall("wellfriendpdf_story_session_bytes",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
        private static final MethodHandle RENDER = Native.downcall("wellfriendpdf_story_session_render_page_png",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
        private static final MethodHandle FREE = Native.downcall("wellfriendpdf_story_session_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));

        private static void checkJsonLength(String json) {
            Objects.requireNonNull(json, "json");
            if (json.length() > 32 * 1024 * 1024) throw new IllegalArgumentException("Story JSON exceeds 32 MiB");
        }
        private static byte[] utf8(String json) {
            checkJsonLength(json);
            try {
                ByteBuffer encoded = StandardCharsets.UTF_8.newEncoder().encode(java.nio.CharBuffer.wrap(json));
                if (encoded.remaining() > 32 * 1024 * 1024) throw new IllegalArgumentException("Story JSON exceeds 32 MiB UTF-8");
                byte[] bytes = new byte[encoded.remaining()];
                encoded.get(bytes);
                return bytes;
            } catch (java.nio.charset.CharacterCodingException ex) {
                throw new IllegalArgumentException("Story JSON contains an unpaired Unicode surrogate", ex);
            }
        }
        private static MemorySegment copy(Arena arena, byte[] bytes) {
            MemorySegment data = arena.allocate(Math.max(1, bytes.length));
            data.asSlice(0, bytes.length).copyFrom(MemorySegment.ofArray(bytes));
            return data;
        }
        private static MemorySegment open(byte[] bytes, MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = copy(arena, bytes);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment session = (MemorySegment) OPEN.invokeExact(data, (long) bytes.length, cancellation, error);
                Native.throwError(Native.isNull(session) ? 2 : 0, error);
                return session;
            } catch (WellfriendPdfException ex) { throw ex; }
            catch (Throwable ex) { throw new IllegalStateException("Story open failed", ex); }
        }
        private static MemorySegment openWithPassword(
                byte[] bytes,
                byte[] password,
                MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = copy(arena, bytes);
                MemorySegment secret = copy(arena, password);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                try {
                    MemorySegment session = (MemorySegment) EncryptedStoryNative.OPEN.invokeExact(
                        data, (long) bytes.length, secret, (long) password.length, cancellation, error);
                    Native.throwError(Native.isNull(session) ? 2 : 0, error);
                    return session;
                } finally {
                    secret.fill((byte) 0);
                }
            } catch (WellfriendPdfException ex) { throw ex; }
            catch (Throwable ex) { throw new IllegalStateException("Encrypted story open failed", ex); }
        }
        private static byte[] command(MemorySegment session, byte[] json, MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = copy(arena, json);
                MemorySegment output = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) COMMAND.invokeExact(session, data, (long) json.length, cancellation, output, error);
                Native.throwError(status, error);
                return Native.takeBuffer(output);
            } catch (WellfriendPdfException ex) { throw ex; }
            catch (Throwable ex) { throw new IllegalStateException("Story command failed", ex); }
        }
        private static byte[] bytes(MemorySegment session) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) BYTES.invokeExact(session, output, error);
                Native.throwError(status, error);
                return Native.takeBuffer(output);
            } catch (WellfriendPdfException ex) { throw ex; }
            catch (Throwable ex) { throw new IllegalStateException("Story byte export failed", ex); }
        }
        private static byte[] render(MemorySegment session, long page, int dpi, MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment output = arena.allocate(Native.BUFFER_LAYOUT);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) RENDER.invokeExact(session, page, dpi, cancellation, output, error);
                Native.throwError(status, error);
                return Native.takeBuffer(output);
            } catch (WellfriendPdfException ex) { throw ex; }
            catch (Throwable ex) { throw new IllegalStateException("Story rendering failed", ex); }
        }
        private static void free(MemorySegment session) {
            try { FREE.invokeExact(session); }
            catch (Throwable ex) { throw new IllegalStateException("Story close failed", ex); }
        }
    }

    // Resolve the additive credentialed constructor only when requested so an
    // older native library can still serve the pre-existing plaintext session
    // surface with a deterministic missing-symbol failure limited to this API.
    private static final class EncryptedStoryNative {
        private static final MethodHandle OPEN = Native.downcall(
            "wellfriendpdf_story_session_open_with_password",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS));
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
        private static final MethodHandle REGISTER_FONT_BYTES = downcall(
            "wellfriendpdf_document_register_font_bytes",
            FunctionDescriptor.of(
                ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS,
                ValueLayout.ADDRESS,
                ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS
            )
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
        private static final MethodHandle RENDER_CANCELLATION_NEW = downcall(
            "wellfriendpdf_render_cancellation_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CANCELLATION_CANCEL = downcall(
            "wellfriendpdf_render_cancellation_cancel",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CANCELLATION_IS_CANCELLED = downcall(
            "wellfriendpdf_render_cancellation_is_cancelled",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CANCELLATION_FREE = downcall(
            "wellfriendpdf_render_cancellation_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CACHE_NEW = downcall(
            "wellfriendpdf_render_cache_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CACHE_CLEAR = downcall(
            "wellfriendpdf_render_cache_clear",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CACHE_APPLY_RENDER_INVALIDATION_PLAN = downcall(
            "wellfriendpdf_render_cache_apply_render_invalidation_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_CACHE_FREE = downcall(
            "wellfriendpdf_render_cache_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
        );
        private static final MethodHandle PAGE_COUNT = downcall(
            "wellfriendpdf_document_page_count",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EXTRACT_TEXT = downcall(
            "wellfriendpdf_document_extract_text",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG = downcall(
            "wellfriendpdf_document_render_page_png",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_FONT_SUBSTITUTION_REPORT = downcall(
            "wellfriendpdf_document_render_page_png_with_font_substitution_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_JPEG = downcall(
            "wellfriendpdf_document_render_page_jpeg",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_BYTE, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DEFAULT_RENDER_CONTRACT = downcall(
            "wellfriendpdf_document_default_render_contract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle BACKEND_PLAN_ARENA_REPORT = downcall(
            "wellfriendpdf_document_backend_plan_arena_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle BACKEND_PLAN_ARENA_REPORT_FOR_CONTRACT = downcall(
            "wellfriendpdf_document_backend_plan_arena_report_for_contract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PREPRESS_PLATE_REPORT = downcall(
            "wellfriendpdf_document_prepress_plate_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_CACHE = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_render_cache_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_font_substitution_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_font_substitution_report_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_REPORT = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_render_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_CACHE_REPORT = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_render_cache_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_PNG_WITH_CONTRACT_AND_RENDER_REPORT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_page_png_with_contract_and_render_report_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_and_font_substitution_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_FONT_SUBSTITUTION_REPORT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_and_font_substitution_report_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_RENDER_REPORT = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_and_render_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RENDER_PAGE_INTO_BUFFER_WITH_CONTRACT_AND_RENDER_REPORT_AND_CANCELLATION = downcall(
            "wellfriendpdf_document_render_into_buffer_with_contract_and_render_report_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_NEW = downcall(
            "wellfriendpdf_document_progressive_render_new",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_NEW_WITH_CONTRACT_JSON = downcall(
            "wellfriendpdf_document_progressive_render_new_with_contract_json",
            FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_STEP = downcall(
            "wellfriendpdf_progressive_render_step_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_TOKEN = downcall(
            "wellfriendpdf_progressive_render_token_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_PAUSE = downcall(
            "wellfriendpdf_progressive_render_pause_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_RESUME = downcall(
            "wellfriendpdf_progressive_render_resume_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_REQUEST_CANCEL = downcall(
            "wellfriendpdf_progressive_render_request_cancel",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_REVISE_VIEWPORT_HINT = downcall(
            "wellfriendpdf_progressive_render_revise_viewport_hint_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_REVISE_DIRTY_REGION = downcall(
            "wellfriendpdf_progressive_render_revise_dirty_region_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_INT,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_REVISE_RENDER_CONTEXT = downcall(
            "wellfriendpdf_progressive_render_revise_render_context_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_REVISE_RENDER_CONTRACT = downcall(
            "wellfriendpdf_progressive_render_revise_render_contract_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_EVALUATE_TILE_PUBLICATION = downcall(
            "wellfriendpdf_progressive_render_evaluate_tile_publication_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_APPLY_RENDER_INVALIDATION_PLAN = downcall(
            "wellfriendpdf_progressive_render_apply_render_invalidation_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_VIEWER_QUEUE = downcall(
            "wellfriendpdf_progressive_render_viewer_queue_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_EXECUTE_VIEWER_QUEUE = downcall(
            "wellfriendpdf_progressive_render_execute_viewer_queue_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_EXECUTE_VIEWER_QUEUE_AND_CANCELLATION = downcall(
            "wellfriendpdf_progressive_render_execute_viewer_queue_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_EXECUTE_ADJACENT_PAGE_PREFETCH = downcall(
            "wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_EXECUTE_ADJACENT_PAGE_PREFETCH_AND_CANCELLATION = downcall(
            "wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json_and_cancellation",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_VIEWER_CALLBACK_DISPATCH = downcall(
            "wellfriendpdf_progressive_render_viewer_callback_dispatch_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_CANCEL = downcall(
            "wellfriendpdf_progressive_render_cancel",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_FINISH = downcall(
            "wellfriendpdf_progressive_render_finish_png",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle PROGRESSIVE_FREE = downcall(
            "wellfriendpdf_progressive_render_free",
            FunctionDescriptor.ofVoid(ValueLayout.ADDRESS)
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
        private static final MethodHandle DOCUMENT_VIEWS_REPORT =
            documentReport("wellfriendpdf_document_views_report_json");
        private static final MethodHandle IMAGE_DECODE_CAPABILITY_REPORT =
            documentReport("wellfriendpdf_document_image_decode_capability_report_json");
        private static final MethodHandle PROGRESSIVE_IMAGE_DECODE_LIFECYCLE_REPORT =
            documentStringReport("wellfriendpdf_document_progressive_image_decode_lifecycle_report_json");
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
        private static final MethodHandle ANNOTATION_MEDIA_REDACTION_REPORT = documentReport("wellfriendpdf_document_annotation_media_redaction_report_json");
        private static final MethodHandle SECURE_MUTATION_REPORT = documentReport("wellfriendpdf_document_secure_mutation_report_json");
        private static final MethodHandle SECURE_MUTATION_CLOSEOUT_REPORT = documentReport("wellfriendpdf_document_secure_mutation_closeout_report_json");
        private static final MethodHandle FORM_JS_REPORT = documentReport("wellfriendpdf_document_form_js_report_json");
        private static final MethodHandle FORM_ACTION_GRAPH = documentReport("wellfriendpdf_document_form_action_graph_json");
        private static final MethodHandle INTERACTIVE_DATA_REPORT = documentReport("wellfriendpdf_document_interactive_data_report_json");
        private static final MethodHandle WORD_PAGINATION_AUDIT = documentStringReport("wellfriendpdf_document_word_pagination_audit_json");
        private static final MethodHandle FORM_ACTION_POLICY_REPORT = documentReport("wellfriendpdf_document_form_action_policy_report_json");
        private static final MethodHandle ADVANCED_EDITING_REPORT = documentReport("wellfriendpdf_document_advanced_editing_report_json");
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_REPORT = documentReport("wellfriendpdf_document_advanced_editing_closeout_report_json");
        private static final MethodHandle SOURCE_EDITING_REPORT = documentReport("wellfriendpdf_document_source_editing_report_json");
        private static final MethodHandle EDITING_TRANSACTIONS_REPORT = documentReport("wellfriendpdf_document_editing_transactions_report_json");
        private static final MethodHandle UNIVERSAL_EDITING_CAPABILITIES_V2 = downcall(
            "wellfriendpdf_universal_editing_capabilities_v2_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_APPROVAL_V2 = downcall(
            "wellfriendpdf_universal_editing_approval_v2_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_ANALYZE_V2 =
            documentStringReport("wellfriendpdf_document_universal_editing_analyze_v2_json");
        private static final MethodHandle UNIVERSAL_RENDER_QUALIFICATION_V2 =
            documentStringReport("wellfriendpdf_document_universal_render_qualification_v2_json");
        private static final MethodHandle UNIVERSAL_EDITING_INSPECT_OBJECT_V2 = downcall(
            "wellfriendpdf_document_universal_editing_inspect_object_v2_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.JAVA_INT, ValueLayout.JAVA_SHORT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_PLAN_V2 =
            documentStringReport("wellfriendpdf_document_universal_editing_plan_v2_json");
        private static final MethodHandle STORY_FIGURE_TRANSFER_PREVIEW =
            documentStringReport("wellfriendpdf_document_story_figure_transfer_preview_json");
        private static final MethodHandle STORY_FIGURE_TRANSFER_APPLY = downcall(
            "wellfriendpdf_document_story_figure_transfer_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_SCOPED_PREVIEW_V2 = downcall(
            "wellfriendpdf_document_universal_editing_scoped_preview_v2_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_APPLY_V2 = downcall(
            "wellfriendpdf_document_universal_editing_apply_v2_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_APPLY_V2_WITH_OUTPUT_CREDENTIALS = downcall(
            "wellfriendpdf_document_universal_editing_apply_v2_with_output_credentials_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle UNIVERSAL_EDITING_APPLY_V2_WITH_OUTPUT_CREDENTIAL_BYTES = downcall(
            "wellfriendpdf_document_universal_editing_apply_v2_with_output_credential_bytes_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_REPORT = documentReport("wellfriendpdf_document_text_reflow_report_json");
        private static final MethodHandle DOCUMENT_SUBSYSTEMS_REPORT = documentReport("wellfriendpdf_document_document_subsystems_report_json");
        private static final MethodHandle DOCUMENT_SUBSYSTEMS_ANALYZE = documentReport("wellfriendpdf_document_document_subsystems_analyze_json");
        private static final MethodHandle DOCUMENT_SUBSYSTEMS_PLAN =
            documentStringReport("wellfriendpdf_document_document_subsystems_plan_json");
        private static final MethodHandle DOCUMENT_SECURITY_REPORT = documentReport("wellfriendpdf_document_document_security_report_json");
        private static final MethodHandle DOCUMENT_SECURITY_ANALYZE = documentReport("wellfriendpdf_document_document_security_analyze_json");
        private static final MethodHandle DOCUMENT_SECURITY_PLAN =
            documentStringReport("wellfriendpdf_document_document_security_plan_json");
        private static final MethodHandle DOCUMENT_SECURITY_VERIFY_RESIDUAL =
            documentStringReport("wellfriendpdf_document_document_security_verify_residual_json");
        private static final MethodHandle WRITER_HISTORY_REPORT = documentReport("wellfriendpdf_document_writer_history_report_json");
        private static final MethodHandle WRITER_HISTORY_RASTER_VECTOR_REPORT = downcall(
            "wellfriendpdf_document_writer_history_raster_vector_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle WRITER_HISTORY_FONT_RECONSTRUCTION_REPORT =
            documentReport("wellfriendpdf_document_writer_history_font_reconstruction_report_json");
        private static final MethodHandle WRITER_HISTORY_HISTORY_REPORT = downcall(
            "wellfriendpdf_writer_history_history_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle WRITER_HISTORY_OBJECT_STREAM_REPORT =
            documentReport("wellfriendpdf_document_writer_history_object_stream_report_json");
        private static final MethodHandle WRITER_HISTORY_PACK_OBJECT_STREAMS = downcall(
            "wellfriendpdf_document_writer_history_pack_object_streams_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle COMPRESSION_OFFICE_REPORT =
            documentReport("wellfriendpdf_document_compression_office_report_json");
        private static final MethodHandle CRYPTO_WRITER_REPORT =
            documentReport("wellfriendpdf_document_crypto_writer_report_json");
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
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_AUTHENTICATE_RECEIPT =
            downcall(
                "wellfriendpdf_advanced_editing_closeout_paint_partition_authenticate_receipt_json",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                    ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_LONG,
                    ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS,
                    ValueLayout.ADDRESS)
            );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_VERIFY_AUTHENTICATED_RECEIPT =
            downcall(
                "wellfriendpdf_advanced_editing_closeout_paint_partition_verify_authenticated_receipt_json",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                    ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.JAVA_LONG,
                    ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS,
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
        private static final MethodHandle COMPRESSION_OFFICE_OPTIMIZE = downcall(
            "wellfriendpdf_document_compression_office_optimize_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle COMPRESSION_OFFICE_OFFICE_INSPECT = downcall(
            "wellfriendpdf_compression_office_office_inspect_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle COMPRESSION_OFFICE_OFFICE_TO_PDF = downcall(
            "wellfriendpdf_compression_office_office_to_pdf",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_TEXT_RANGE_ANALYZE = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_text_range_analyze_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_PROPOSE = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_paint_partition_propose_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_PREVIEW = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_paint_partition_preview_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_TEXT_RANGE_EDIT = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_text_range_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ECBES_UNIVERSAL_EDIT = downcall(
            "wellfriendpdf_document_ecbes_universal_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ECBES_UNIVERSAL_EDIT_WITH_OUTPUT_CREDENTIAL_BYTES = downcall(
            "wellfriendpdf_document_ecbes_universal_edit_with_output_credential_bytes_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_APPLY = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_paint_partition_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_APPLY_WITH_FONT = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_paint_partition_apply_with_font_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_APPLY_REVIEWED_WITH_FONT = downcall(
            "wellfriendpdf_document_advanced_editing_closeout_paint_partition_apply_reviewed_with_font_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle AUTHORED_TYPED_TABLE_SOURCES =
            documentReport("wellfriendpdf_document_authored_typed_table_sources_json");
        private static final MethodHandle AUTHORED_TYPED_TABLE_MUTATE = downcall(
            "wellfriendpdf_document_authored_typed_table_mutate_with_font_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SOURCE_EDITING_PROVENANCE = downcall(
            "wellfriendpdf_document_source_editing_provenance_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SOURCE_EDITING_EDIT_ELIGIBILITY = downcall(
            "wellfriendpdf_document_source_editing_edit_eligibility_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SOURCE_EDITING_OPERATOR_TEXT_EDIT = downcall(
            "wellfriendpdf_document_source_editing_operator_text_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SOURCE_EDITING_PATH_PROVENANCE = downcall(
            "wellfriendpdf_document_source_editing_path_provenance_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle SOURCE_EDITING_PATH_EDIT = downcall(
            "wellfriendpdf_document_source_editing_path_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_SCENE_REPORT = downcall(
            "wellfriendpdf_document_editing_transactions_scene_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_SCENE_SELECT = downcall(
            "wellfriendpdf_document_editing_transactions_scene_select_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_TRANSACTION_PLAN = downcall(
            "wellfriendpdf_document_editing_transactions_transaction_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_TRANSACTION_APPLY = downcall(
            "wellfriendpdf_document_editing_transactions_transaction_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_TRANSACTION_APPLY_WITH_RENDER_INVALIDATION = downcall(
            "wellfriendpdf_document_editing_transactions_transaction_apply_with_render_invalidation_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_TEXT_MAP = downcall(
            "wellfriendpdf_document_editing_transactions_text_map_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_SHAPE_TEXT = downcall(
            "wellfriendpdf_document_editing_transactions_shape_text_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_FONT_SUBSET_PLAN = downcall(
            "wellfriendpdf_document_editing_transactions_font_subset_plan_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle EDITING_TRANSACTIONS_FONT_SUBSTITUTION_REPORT = downcall(
            "wellfriendpdf_document_editing_transactions_font_substitution_report_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_LAYOUT_ANALYZE =
            documentStringReport("wellfriendpdf_document_text_reflow_layout_analyze_json");
        private static final MethodHandle TEXT_REFLOW_SEMANTIC_LAYOUT =
            documentReport("wellfriendpdf_document_text_reflow_semantic_layout_json");
        private static final MethodHandle TEXT_REFLOW_READING_ORDER =
            documentReport("wellfriendpdf_document_text_reflow_reading_order_report_json");
        private static final MethodHandle TEXT_REFLOW_FLOW_GRAPH =
            documentReport("wellfriendpdf_document_text_reflow_flow_graph_report_json");
        private static final MethodHandle TEXT_REFLOW_REFLOW_PREVIEW =
            documentStringReport("wellfriendpdf_document_text_reflow_reflow_preview_json");
        private static final MethodHandle TEXT_REFLOW_OVERFLOW_REPORT =
            documentStringReport("wellfriendpdf_document_text_reflow_overflow_report_json");
        private static final MethodHandle TEXT_REFLOW_CONSTRAINTS_REPORT =
            documentStringReport("wellfriendpdf_document_text_reflow_constraints_report_json");
        private static final MethodHandle TEXT_REFLOW_CONFIDENCE_REPORT =
            documentStringReport("wellfriendpdf_document_text_reflow_confidence_report_json");
        private static final MethodHandle TEXT_REFLOW_VALIDATE_REFLOW_OUTPUT = downcall(
            "wellfriendpdf_document_text_reflow_validate_reflow_output_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_REFLOW_REGION = downcall(
            "wellfriendpdf_document_text_reflow_reflow_region_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_REFLOW_DOCUMENT = downcall(
            "wellfriendpdf_document_text_reflow_reflow_document_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_UNDO_REFLOW = downcall(
            "wellfriendpdf_document_text_reflow_undo_reflow_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS)
        );
        private static final MethodHandle TEXT_REFLOW_APPROVE_STRUCTURE =
            documentStringReport("wellfriendpdf_document_text_reflow_reflow_approve_structure_json");
        private static final MethodHandle TEXT_REFLOW_OPERATION_REPORT =
            documentStringReport("wellfriendpdf_document_text_reflow_reflow_operation_report_json");
        private static final MethodHandle DOCUMENT_SUBSYSTEMS_APPLY = downcall(
            "wellfriendpdf_document_document_subsystems_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DOCUMENT_SUBSYSTEMS_UNDO = downcall(
            "wellfriendpdf_document_document_subsystems_undo_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DOCUMENT_SECURITY_APPLY = downcall(
            "wellfriendpdf_document_document_security_apply_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle DOCUMENT_SECURITY_UNDO = downcall(
            "wellfriendpdf_document_document_security_undo_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.JAVA_LONG, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_VECTOR_LIST = downcall(
            "wellfriendpdf_document_advanced_editing_vector_list_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_TEXT_EDIT = downcall(
            "wellfriendpdf_document_advanced_editing_text_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_VECTOR_EDIT = downcall(
            "wellfriendpdf_document_advanced_editing_vector_edit_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS,
                ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle ADVANCED_EDITING_INK_FIT = downcall(
            "wellfriendpdf_document_advanced_editing_ink_fit_json",
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
        private static final MethodHandle RUNTIME_EFFECTIVE_CONFIG = downcall(
            "wellfriendpdf_runtime_effective_config_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle RUNTIME_CAPABILITIES = downcall(
            "wellfriendpdf_runtime_capabilities_json",
            FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.ADDRESS)
        );
        private static final MethodHandle OCR_PROVIDER_MATRIX = downcall(
            "wellfriendpdf_ocr_provider_matrix_json",
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
                    "Could not locate wellfriendpdf_capi native library. Set WELLFRIENDPDF_NATIVE_LIBRARY or package it beside the Java artifact or under runtimes/<rid>/native.");
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
            List<Path> roots = new ArrayList<>();
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

        private static String runtimeCapabilitiesJson(String configJson) {
            return runtimeConfigCall(
                RUNTIME_CAPABILITIES, configJson, "Wellfriend runtime_capabilities failed");
        }

        private static String runtimeConfigJson(String configJson) {
            return runtimeConfigCall(
                RUNTIME_EFFECTIVE_CONFIG, configJson, "Wellfriend runtime_config failed");
        }

        private static String ocrProviderMatrixJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) OCR_PROVIDER_MATRIX.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend ocr_provider_matrix failed", ex);
            }
        }

        private static String runtimeConfigCall(MethodHandle handle, String configJson, String context) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment config = configJson == null ? MemorySegment.NULL : arena.allocateFrom(configJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) handle.invokeExact(config, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(context, ex);
            }
        }

        private static String writer_historyHistoryReportJson() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) WRITER_HISTORY_HISTORY_REPORT.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend writer_history_history_report failed", ex);
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

        private static String authenticateTextRangePaintPartitionReceipt(
            String publicationReceiptJson,
            String keyId,
            String audience,
            long issuedAtUnix,
            long expiresAtUnix,
            byte[] hmacKey
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment receipt = arena.allocateFrom(publicationReceiptJson);
                MemorySegment keyIdSegment = arena.allocateFrom(keyId);
                MemorySegment audienceSegment = arena.allocateFrom(audience);
                MemorySegment key = hmacKey.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(hmacKey.length);
                if (hmacKey.length != 0) {
                    key.copyFrom(MemorySegment.ofArray(hmacKey));
                }
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_AUTHENTICATE_RECEIPT.invokeExact(
                    receipt,
                    keyIdSegment,
                    audienceSegment,
                    issuedAtUnix,
                    expiresAtUnix,
                    key,
                    (long) hmacKey.length,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend paint-partition receipt authentication failed", ex);
            }
        }

        private static String verifyAuthenticatedTextRangePaintPartitionReceipt(
            String authenticatedReceiptJson,
            String expectedKeyId,
            String expectedAudience,
            long nowUnix,
            long allowedFutureSkewSecs,
            byte[] hmacKey
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment receipt = arena.allocateFrom(authenticatedReceiptJson);
                MemorySegment keyIdSegment = arena.allocateFrom(expectedKeyId);
                MemorySegment audienceSegment = arena.allocateFrom(expectedAudience);
                MemorySegment key = hmacKey.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(hmacKey.length);
                if (hmacKey.length != 0) {
                    key.copyFrom(MemorySegment.ofArray(hmacKey));
                }
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_VERIFY_AUTHENTICATED_RECEIPT.invokeExact(
                    receipt,
                    keyIdSegment,
                    audienceSegment,
                    nowUnix,
                    allowedFutureSkewSecs,
                    key,
                    (long) hmacKey.length,
                    jsonOut,
                    err
                );
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend authenticated paint-partition receipt verification failed", ex);
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

        private static String universalEditingCapabilitiesV2Json() {
            return globalReport(
                UNIVERSAL_EDITING_CAPABILITIES_V2,
                "Wellfriend universal_editing_capabilities_v2 failed");
        }

        private static String universalEditingApprovalV2Json(
            String planJson,
            String decisionJson
        ) {
            return globalTwoStringReport(
                UNIVERSAL_EDITING_APPROVAL_V2,
                planJson,
                decisionJson,
                "Wellfriend universal_editing_approval_v2 failed");
        }

        private static MemorySegment newRenderCancellation() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment cancellation = (MemorySegment) RENDER_CANCELLATION_NEW.invokeExact(error);
                if (isNull(cancellation)) {
                    throwError(2, error);
                }
                return cancellation;
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cancellation creation failed", ex);
            }
        }

        private static void freeRenderCancellation(MemorySegment cancellation) {
            if (isNull(cancellation)) return;
            try {
                RENDER_CANCELLATION_FREE.invokeExact(cancellation);
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cancellation release failed", ex);
            }
        }

        private static void cancelRender(MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) RENDER_CANCELLATION_CANCEL.invokeExact(cancellation, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cancellation failed", ex);
            }
        }

        private static boolean isRenderCancelled(MemorySegment cancellation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment cancelled = arena.allocate(ValueLayout.JAVA_INT);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) RENDER_CANCELLATION_IS_CANCELLED.invokeExact(
                    cancellation, cancelled, error);
                throwError(status, error);
                return cancelled.get(ValueLayout.JAVA_INT, 0) != 0;
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cancellation status query failed", ex);
            }
        }

        private static MemorySegment newRenderCache() {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment cache = (MemorySegment) RENDER_CACHE_NEW.invokeExact(error);
                if (isNull(cache)) {
                    throwError(2, error);
                }
                return cache;
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cache creation failed", ex);
            }
        }

        private static void freeRenderCache(MemorySegment cache) {
            if (isNull(cache)) return;
            try {
                RENDER_CACHE_FREE.invokeExact(cache);
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cache release failed", ex);
            }
        }

        private static void clearRenderCache(MemorySegment cache) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) RENDER_CACHE_CLEAR.invokeExact(cache, error);
                throwError(status, error);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cache clear failed", ex);
            }
        }

        private static String applyRenderCacheInvalidationPlan(MemorySegment cache, String planJson) {
            Objects.requireNonNull(planJson, "planJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment plan = arena.allocateFrom(planJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment error = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) RENDER_CACHE_APPLY_RENDER_INVALIDATION_PLAN.invokeExact(
                    cache, plan, jsonOut, error);
                throwError(status, error);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend render cache invalidation failed", ex);
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

        private static String documentTwoStringReport(MemorySegment handle, MethodHandle method,
            String first, String second, String operation) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment arg1 = first == null ? MemorySegment.NULL : arena.allocateFrom(first);
                MemorySegment arg2 = second == null ? MemorySegment.NULL : arena.allocateFrom(second);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(handle, arg1, arg2, jsonOut, err);
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

        private static String advanced_editingVectorList(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_VECTOR_LIST.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend advanced_editing_vector_list failed", ex);
            }
        }

        private static String writer_historyRasterVectorReport(
            MemorySegment handle, long page, String optionsJson
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) WRITER_HISTORY_RASTER_VECTOR_REPORT.invokeExact(
                    handle, page, options, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend writer_history_raster_vector_report failed", ex);
            }
        }

        private static String advanced_editing_closeoutTextRangeAnalyze(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_TEXT_RANGE_ANALYZE.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend advanced_editing_closeout_text_range_analyze failed", ex); }
        }

        private static String advanced_editing_closeoutPaintPartitionPropose(
            MemorySegment handle, String requestJson
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_PROPOSE.invokeExact(
                    handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend advanced_editing_closeout_paint_partition_propose failed", ex);
            }
        }

        private static String advanced_editing_closeoutPaintPartitionPreview(
            MemorySegment handle,
            String requestJson,
            String proposalJson,
            String approvalJson,
            byte[] fontBytes,
            String optionsJson
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            Objects.requireNonNull(proposalJson, "proposalJson");
            Objects.requireNonNull(approvalJson, "approvalJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment proposal = arena.allocateFrom(proposalJson);
                MemorySegment approval = arena.allocateFrom(approvalJson);
                MemorySegment font = fontBytes == null || fontBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(fontBytes.length);
                if (fontBytes != null && fontBytes.length != 0) {
                    font.copyFrom(MemorySegment.ofArray(fontBytes));
                }
                MemorySegment options = optionsJson == null
                    ? MemorySegment.NULL
                    : arena.allocateFrom(optionsJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_PREVIEW.invokeExact(
                    handle, request, proposal, approval, font,
                    (long) (fontBytes == null ? 0 : fontBytes.length), options, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend advanced_editing_closeout_paint_partition_preview failed", ex);
            }
        }

        private static BinaryResult advanced_editing_closeoutTextRangeEdit(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_TEXT_RANGE_EDIT.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend advanced_editing_closeout_text_range_edit failed", ex); }
        }

        private static BinaryResult advanced_editing_closeoutPaintPartitionApply(
            MemorySegment handle,
            String requestJson,
            String proposalJson,
            String approvalJson,
            byte[] fontBytes
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            Objects.requireNonNull(proposalJson, "proposalJson");
            Objects.requireNonNull(approvalJson, "approvalJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment proposal = arena.allocateFrom(proposalJson);
                MemorySegment approval = arena.allocateFrom(approvalJson);
                MemorySegment font = fontBytes == null || fontBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(fontBytes.length);
                if (fontBytes != null && fontBytes.length != 0) {
                    font.copyFrom(MemorySegment.ofArray(fontBytes));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_APPLY_WITH_FONT.invokeExact(
                    handle, request, proposal, approval, font,
                    (long) (fontBytes == null ? 0 : fontBytes.length), buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend advanced_editing_closeout_paint_partition_apply failed", ex);
            }
        }

        private static BinaryResult advanced_editing_closeoutPaintPartitionApplyReviewed(
            MemorySegment handle,
            String requestJson,
            String proposalJson,
            String approvalJson,
            String publicationReceiptJson,
            byte[] fontBytes
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            Objects.requireNonNull(proposalJson, "proposalJson");
            Objects.requireNonNull(approvalJson, "approvalJson");
            Objects.requireNonNull(publicationReceiptJson, "publicationReceiptJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment proposal = arena.allocateFrom(proposalJson);
                MemorySegment approval = arena.allocateFrom(approvalJson);
                MemorySegment receipt = arena.allocateFrom(publicationReceiptJson);
                MemorySegment font = fontBytes == null || fontBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(fontBytes.length);
                if (fontBytes != null && fontBytes.length != 0) {
                    font.copyFrom(MemorySegment.ofArray(fontBytes));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_CLOSEOUT_PAINT_PARTITION_APPLY_REVIEWED_WITH_FONT.invokeExact(
                    handle, request, proposal, approval, receipt, font,
                    (long) (fontBytes == null ? 0 : fontBytes.length), buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(
                    "Wellfriend reviewed paint-partition apply failed", ex);
            }
        }

        private static BinaryResult authoredTypedTableMutate(
            MemorySegment handle, String requestJson, byte[] fontBytes
        ) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment font = fontBytes == null || fontBytes.length == 0
                    ? MemorySegment.NULL
                    : arena.allocate(fontBytes.length);
                if (fontBytes != null && fontBytes.length != 0) {
                    font.copyFrom(MemorySegment.ofArray(fontBytes));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) AUTHORED_TYPED_TABLE_MUTATE.invokeExact(
                    handle, request, font, (long) (fontBytes == null ? 0 : fontBytes.length),
                    buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend authored_typed_table_mutate failed", ex);
            }
        }

        private static String source_editingProvenance(
            MemorySegment handle, long page, String sourceText, String replacementText
        ) {
            Objects.requireNonNull(sourceText, "sourceText");
            Objects.requireNonNull(replacementText, "replacementText");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment source = arena.allocateFrom(sourceText);
                MemorySegment replacement = arena.allocateFrom(replacementText);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SOURCE_EDITING_PROVENANCE.invokeExact(
                    handle, page, source, replacement, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend source_editing_provenance failed", ex); }
        }

        private static String source_editingEditEligibility(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SOURCE_EDITING_EDIT_ELIGIBILITY.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend source_editing_edit_eligibility failed", ex); }
        }

        private static BinaryResult source_editingOperatorTextEdit(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SOURCE_EDITING_OPERATOR_TEXT_EDIT.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend source_editing_operator_text_edit failed", ex); }
        }

        private static String source_editingPathProvenance(MemorySegment handle, long page) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) SOURCE_EDITING_PATH_PROVENANCE.invokeExact(handle, page, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend source_editing_path_provenance failed", ex); }
        }

        private static BinaryResult source_editingPathEdit(
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
                int status = (int) SOURCE_EDITING_PATH_EDIT.invokeExact(
                    handle, page, id, operation, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend source_editing_path_edit failed", ex); }
        }

        private static String editing_transactionsSceneReport(MemorySegment handle, String pagesJson) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment pages = pagesJson == null || pagesJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(pagesJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_SCENE_REPORT.invokeExact(handle, pages, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_scene_report failed", ex); }
        }

        private static String editing_transactionsSceneSelect(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_SCENE_SELECT.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_scene_select failed", ex); }
        }

        private static String editing_transactionsTransactionPlan(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_TRANSACTION_PLAN.invokeExact(handle, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_transaction_plan failed", ex); }
        }

        private static BinaryResult editing_transactionsTransactionApply(MemorySegment handle, String requestJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_TRANSACTION_APPLY.invokeExact(handle, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_transaction_apply failed", ex); }
        }

        private static BinaryResult editing_transactionsTransactionApplyWithRenderInvalidation(
                MemorySegment handle,
                String requestJson,
                String renderInvalidationOptionsJson) {
            Objects.requireNonNull(requestJson, "requestJson");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment request = arena.allocateFrom(requestJson);
                MemorySegment options = renderInvalidationOptionsJson == null
                    ? MemorySegment.NULL
                    : arena.allocateFrom(renderInvalidationOptionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_TRANSACTION_APPLY_WITH_RENDER_INVALIDATION
                    .invokeExact(handle, request, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend editing_transactions_transaction_apply_with_render_invalidation failed", ex);
            }
        }

        private static String documentObjectReport(
            MemorySegment handle,
            long objectNumber,
            int generation,
            String operation
        ) {
            if (objectNumber < 0 || objectNumber > 0xffff_ffffL) {
                throw new IllegalArgumentException("objectNumber must fit an unsigned 32-bit integer");
            }
            if (generation < 0 || generation > 0xffff) {
                throw new IllegalArgumentException("generation must fit an unsigned 16-bit integer");
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) UNIVERSAL_EDITING_INSPECT_OBJECT_V2.invokeExact(
                    handle, (int) objectNumber, (short) generation, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String globalReport(MethodHandle method, String context) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(context, ex);
            }
        }

        private static String globalTwoStringReport(
            MethodHandle method,
            String first,
            String second,
            String context
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment firstArg = arena.allocateFrom(first);
                MemorySegment secondArg = arena.allocateFrom(second);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) method.invokeExact(firstArg, secondArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException(context, ex);
            }
        }

        private static BinaryResult text_reflowRequestOutput(
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

        private static BinaryResult text_reflowUndoReflow(
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
                int status = (int) TEXT_REFLOW_UNDO_REFLOW.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult document_subsystemsUndo(
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
                int status = (int) DOCUMENT_SUBSYSTEMS_UNDO.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult document_securityUndo(
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
                int status = (int) DOCUMENT_SECURITY_UNDO.invokeExact(
                    handle, output, (long) outputPdf.length, request, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String text_reflowValidateReflowOutput(
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
                int status = (int) TEXT_REFLOW_VALIDATE_REFLOW_OUTPUT.invokeExact(
                    handle, output, (long) outputPdf.length, request, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static String editing_transactionsTextMap(MemorySegment handle, String text, String direction) {
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment directionArg = direction == null || direction.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(direction);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_TEXT_MAP.invokeExact(handle, textArg, directionArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_text_map failed", ex); }
        }

        private static String editing_transactionsShapeText(MemorySegment handle, String text, String direction) {
            Objects.requireNonNull(text, "text");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment textArg = arena.allocateFrom(text);
                MemorySegment directionArg = direction == null || direction.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(direction);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) EDITING_TRANSACTIONS_SHAPE_TEXT.invokeExact(handle, textArg, directionArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_shape_text failed", ex); }
        }

        private static String editing_transactionsFontSubsetPlan(
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
                int status = (int) EDITING_TRANSACTIONS_FONT_SUBSET_PLAN.invokeExact(
                    handle, textArg, directionArg, policyArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_font_subset_plan failed", ex); }
        }

        private static String editing_transactionsFontSubstitutionReport(
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
                int status = (int) EDITING_TRANSACTIONS_FONT_SUBSTITUTION_REPORT.invokeExact(
                    handle, familyArg, textArg, policyArg, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) { throw ex;
            } catch (Throwable ex) { throw new IllegalStateException("Wellfriend editing_transactions_font_substitution_report failed", ex); }
        }

        private static BinaryResult advanced_editingTextEdit(
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
                int status = (int) ADVANCED_EDITING_TEXT_EDIT.invokeExact(
                    handle, page, oldArg, newArg, modeArg, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend advanced_editing_text_edit failed", ex);
            }
        }

        private static BinaryResult advanced_editingVectorEdit(
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
                int status = (int) ADVANCED_EDITING_VECTOR_EDIT.invokeExact(
                    handle, page, id, operation, options, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend advanced_editing_vector_edit failed", ex);
            }
        }

        private static BinaryResult advanced_editingInkFit(
            MemorySegment handle, long page, long annotationIndex, String optionsJson,
            boolean signaturePolicyOverride
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment options = optionsJson == null || optionsJson.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(optionsJson);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) ADVANCED_EDITING_INK_FIT.invokeExact(
                    handle, page, annotationIndex, options,
                    signaturePolicyOverride ? 1 : 0, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend advanced_editing_ink_fit failed", ex);
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

        private static BinaryResult universalEditingApplyWithOutputCredentials(
            MemorySegment handle,
            String plan,
            String approval,
            String userPassword,
            String ownerPassword,
            String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment planArg = arena.allocateFrom(plan);
                MemorySegment approvalArg = approval == null || approval.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(approval);
                MemorySegment userArg = userPassword.isEmpty()
                    ? MemorySegment.NULL : arena.allocateFrom(userPassword);
                MemorySegment ownerArg = ownerPassword == null || ownerPassword.isEmpty()
                    ? MemorySegment.NULL : arena.allocateFrom(ownerPassword);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) UNIVERSAL_EDITING_APPLY_V2_WITH_OUTPUT_CREDENTIALS.invokeExact(
                    handle, planArg, approvalArg, userArg, ownerArg, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult universalEditingApplyWithOutputCredentialBytes(
            MemorySegment handle,
            String plan,
            String approval,
            byte[] userPassword,
            byte[] ownerPassword,
            String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment planArg = arena.allocateFrom(plan);
                MemorySegment approvalArg = approval == null || approval.isBlank()
                    ? MemorySegment.NULL : arena.allocateFrom(approval);
                MemorySegment userArg = arena.allocate(Math.max(1, userPassword.length));
                MemorySegment ownerArg = ownerPassword == null
                    ? MemorySegment.NULL
                    : arena.allocate(Math.max(1, ownerPassword.length));
                if (userPassword.length > 0) {
                    userArg.asSlice(0, userPassword.length).copyFrom(MemorySegment.ofArray(userPassword));
                }
                if (ownerPassword != null && ownerPassword.length > 0) {
                    ownerArg.asSlice(0, ownerPassword.length).copyFrom(MemorySegment.ofArray(ownerPassword));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                try {
                    int status = (int) UNIVERSAL_EDITING_APPLY_V2_WITH_OUTPUT_CREDENTIAL_BYTES.invokeExact(
                        handle, planArg, approvalArg, userArg, (long) userPassword.length,
                        ownerArg, (long) (ownerPassword == null ? 0 : ownerPassword.length),
                        buffer, jsonOut, err);
                    throwError(status, err);
                    return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
                } finally {
                    userArg.fill((byte) 0);
                    if (ownerPassword != null) ownerArg.fill((byte) 0);
                }
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend " + operation + " failed", ex);
            }
        }

        private static BinaryResult ecbesUniversalEditWithOutputCredentialBytes(
            MemorySegment handle,
            String request,
            byte[] userPassword,
            byte[] ownerPassword,
            String operation
        ) {
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment requestArg = arena.allocateFrom(request);
                MemorySegment userArg = arena.allocate(Math.max(1, userPassword.length));
                MemorySegment ownerArg = ownerPassword == null
                    ? MemorySegment.NULL
                    : arena.allocate(Math.max(1, ownerPassword.length));
                if (userPassword.length > 0) {
                    userArg.asSlice(0, userPassword.length).copyFrom(MemorySegment.ofArray(userPassword));
                }
                if (ownerPassword != null && ownerPassword.length > 0) {
                    ownerArg.asSlice(0, ownerPassword.length).copyFrom(MemorySegment.ofArray(ownerPassword));
                }
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                try {
                    int status = (int) ECBES_UNIVERSAL_EDIT_WITH_OUTPUT_CREDENTIAL_BYTES.invokeExact(
                        handle, requestArg, userArg, (long) userPassword.length,
                        ownerArg, (long) (ownerPassword == null ? 0 : ownerPassword.length),
                        buffer, jsonOut, err);
                    throwError(status, err);
                    return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
                } finally {
                    userArg.fill((byte) 0);
                    if (ownerPassword != null) ownerArg.fill((byte) 0);
                }
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

        private static String compression_officeOfficeInspect(byte[] bytes, String format) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(format, "format");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment formatPtr = arena.allocateFrom(format);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) COMPRESSION_OFFICE_OFFICE_INSPECT.invokeExact(
                    data, (long) bytes.length, formatPtr, jsonOut, err);
                throwError(status, err);
                return takeString(jsonOut);
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend compression_office_office_inspect failed", ex);
            }
        }

        private static BinaryResult compression_officeOfficeToPdf(byte[] bytes, String format) {
            Objects.requireNonNull(bytes, "bytes");
            Objects.requireNonNull(format, "format");
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment data = arena.allocate(bytes.length);
                data.copyFrom(MemorySegment.ofArray(bytes));
                MemorySegment formatPtr = arena.allocateFrom(format);
                MemorySegment buffer = arena.allocate(BUFFER_LAYOUT);
                MemorySegment jsonOut = arena.allocate(ValueLayout.ADDRESS);
                MemorySegment err = arena.allocate(ValueLayout.ADDRESS);
                int status = (int) COMPRESSION_OFFICE_OFFICE_TO_PDF.invokeExact(
                    data, (long) bytes.length, formatPtr, buffer, jsonOut, err);
                throwError(status, err);
                return new BinaryResult(takeBuffer(buffer), takeString(jsonOut));
            } catch (WellfriendPdfException ex) {
                throw ex;
            } catch (Throwable ex) {
                throw new IllegalStateException("Wellfriend compression_office_office_to_pdf failed", ex);
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
