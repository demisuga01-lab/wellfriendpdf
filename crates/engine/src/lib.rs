//! Wellfriend Engine — Pure Rust PDF processing.
//!
//! This crate has no external PDF processing system dependencies:
//! - No Poppler
//! - No Ghostscript
//! - No ImageMagick
//! - No Java or Python
//! - No system-installed font rendering libraries
//!
//! PDF parsing, text extraction, image decoding, and page rendering are
//! implemented in Rust. Compression and image support come from crate
//! dependencies and do not shell out to external PDF tools.
#![forbid(unsafe_code)]
#![recursion_limit = "256"]
//!
//! # Getting started
//!
//! [`ContentEngine`] is the main entry point. Open a PDF, then call the
//! operation you need:
//!
//! ```no_run
//! use wellfriendpdf_engine::ContentEngine;
//!
//! # fn main() -> wellfriendpdf_engine::Result<()> {
//! let engine = ContentEngine::open_path("input.pdf")?;
//!
//! // Document facts.
//! let pages = engine.page_count()?;
//! let info = engine.document_info()?;          // pdfinfo-equivalent
//! let fonts = engine.list_fonts()?;            // pdffonts-equivalent
//!
//! // Text & rendering.
//! let text = engine.get_page_text(1)?;         // pdftotext-equivalent
//! let png = engine.render_page_png_fast(1, 150)?; // pdftoppm-equivalent
//! let svg = engine.render_page_svg(1, 96)?;    // pdftocairo -svg
//!
//! // Conversion & reporting.
//! let attachments = engine.list_attachments()?;       // pdfdetach-equivalent
//! let sigs = engine.verify_signatures()?;             // pdfsig-equivalent
//! # let _ = (pages, info, fonts, text, png, svg, attachments, sigs);
//! # Ok(())
//! # }
//! ```
//!
//! Document manipulation produces new PDF bytes via the pure-Rust writer:
//!
//! ```no_run
//! use wellfriendpdf_engine::{build_merged, PdfDocument};
//!
//! # fn main() -> wellfriendpdf_engine::Result<()> {
//! let a = PdfDocument::open_path("a.pdf")?;
//! let b = PdfDocument::open_path("b.pdf")?;
//! // Merge all pages of both documents (pdfunite-equivalent).
//! let merged: Vec<u8> = build_merged(&[
//!     (&a, vec![1]),
//!     (&b, vec![1]),
//! ])?;
//! std::fs::write("merged.pdf", merged)?;
//! # Ok(())
//! # }
//! ```
//!
//! Runnable examples live in the crate's `examples/` directory
//! (`cargo run --example getting_started -- input.pdf`).
//!
//! # Public API Stability
//!
//! `wellfriendpdf_engine::prelude` is the curated integration surface. The crate root
//! also exposes lower-level PDF internals for advanced users; those are useful
//! but less stable while the crate remains pre-1.0. See `docs/api_overview.md`
//! and `docs/stability.md` in the repository for the full policy.

pub mod advanced_editing;
pub mod advanced_rag;
pub mod analysis;
pub mod analyzer;
pub mod annotation_media_redaction;
pub mod arlington;
pub mod attachments;
pub mod authoring;
pub mod cancel;
pub mod chunk;
pub mod classify;
pub mod codec_isolation;
pub mod color_report;
pub mod compliance;
pub mod compression_office;
pub mod content;
pub mod crypto;
pub mod crypto_writer;
pub mod decode_cache;
pub mod decode_scanner;
pub mod decode_scheduler;
pub mod docmodel;
pub mod document;
pub mod document_security;
pub mod document_subsystems;
pub mod editable;
pub mod editing;
pub mod editing_transactions;
pub mod engine;
pub mod error;
pub mod eval;
pub mod extract;
pub mod filters;
pub mod fonts;
pub mod fonts_report;
pub mod form_action_policy;
pub mod form_exchange;
#[cfg(feature = "fuzzing")]
pub mod fuzz;
pub mod html;
pub mod images;
pub mod info;
pub mod interactive;
pub mod object;
pub mod ocr;
pub mod office;
pub mod optional_content;
pub mod parse;
pub mod parser;
pub mod parser_report;
pub mod pdf_mac;
pub mod prepress;
pub mod pubsec;
pub mod reader;
pub mod render;
pub mod runtime;
pub mod sdk;
pub mod secure_mutation;
pub mod security;
pub mod semantic;
pub mod semantic_binding;
pub mod semantic_intelligence;
pub mod signature;
pub mod signature_evidence;
pub mod source_editing;
pub mod standards;
pub mod standards_engine;
pub mod structural;
pub mod table_intelligence;
pub mod text;
pub mod text_reflow;
pub mod utilities;
pub mod versioning;
pub mod writer;
pub mod writer_history;
pub mod xfa;

/// Semantic version of the wellfriendpdf-engine crate.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub use advanced_editing::{
    advanced_editing_report, analyze_advanced_text_reflow, analyze_multi_run_text_range,
    analyze_same_width_patch, apply_same_width_patch, edit_advanced_text_pdf,
    edit_advanced_text_pdf_with_layout, edit_advanced_text_pdf_with_positioned_visual_layout,
    edit_advanced_text_pdf_with_visual_layout, edit_multi_run_text_range, edit_vector_object,
    fit_annotation_ink_pdf, fit_ink_stroke, fit_ink_strokes, list_vector_objects,
    move_link_annotation_rect_pdf, AdvancedEditingMutationCheckpoint, AdvancedEditingMutationPatch,
    AdvancedEditingMutationSession, AdvancedEditingSupportStatus, AdvancedTextEditOptions,
    AdvancedTextEditReport, AdvancedTextMode, AnnotationInkFitReport, BidiRunProvenance,
    CacheInvalidationReport, CubicBezier, EditableVectorObject, ExplicitLayoutLine,
    GeneratedLineAdjustment, GeneratedTextAlignment, InkFitOptions, InkFitPolicy, InkFitReport,
    InkFitResult, InkPoint, InkStrokeSetResult, LinkAnnotationMoveReport, MultiRunRangeModel,
    MultiRunSourceSpan, MultiRunStylePolicy, MultiRunTextEditReport, MultiRunTextRangeRequest,
    PatchStringRepresentation, PositionedExplicitLayoutLine, SameWidthMode,
    SameWidthPatchApplyReport, SameWidthPatchEligibility, SameWidthPatchEligibilityReport,
    SameWidthPatchOptions, SharedFormEditPolicy, TextGlyphProvenance, TextOverflowPolicy,
    TextReflowAnalysis, TextReflowLimits, VectorColor, VectorEditOperation, VectorEditOptions,
    VectorEditReport, VectorFillRule, VectorFormInvocation, VectorGroupProvenance, VectorMatrix,
    VectorObjectInventory, VectorPaintMode, VectorPathSegment, VectorProvenance, VectorStrokeStyle,
    VerticalGlyphOrientation, ADVANCED_EDITING_SCHEMA_VERSION,
};
pub use advanced_rag::{
    advanced_chunk_document, AdvancedChunkContext, AdvancedChunkMode, AdvancedChunkOptions,
    AdvancedRagChunk, AdvancedRagChunkSet, ChunkSecurityPosture, RagCitation, RagCjkToken,
    RagSourceSpan, RagTableFragment, TableChunkSerialization, ADVANCED_RAG_CHUNK_SCHEMA_VERSION,
};
pub use analysis::graphics::{
    collect_graphics, collect_graphics_with_images, DrawnGraphics, ImagePlacement, Rect, Segment,
};
pub use analyzer::{PdfAnalyzer, TextLayerAnalysis, TextLayerRecommendation};
pub use annotation_media_redaction::{
    apply_nonaxis_image_redaction_pdf, apply_rich_media_policy_pdf, export_annotation_xfdf,
    generate_annotation_appearances_pdf, import_annotation_xfdf_pdf, parse_annotation_xfdf,
    plan_nonaxis_image_redaction, rich_media_inventory, AnnotationAppearanceMetadata,
    AnnotationAppearanceOptions, AnnotationAppearancePolicy, AnnotationAppearanceReport,
    AnnotationConflictPolicy, AnnotationDeletePolicy, AnnotationXfdfDocument,
    AnnotationXfdfExportReport, AnnotationXfdfImportOptions, AnnotationXfdfImportReport,
    AnnotationXfdfRecord, NonAxisRedactionApplyReport, NonAxisRedactionFallbackPolicy,
    NonAxisRedactionOptions, NonAxisRedactionPlan, NonAxisRedactionRequest,
    RedactionCoordinateSpace, RichMediaCounts, RichMediaCustomPolicy, RichMediaInventoryReport,
    RichMediaLimits, RichMediaPolicyMode, RichMediaPolicyReport,
    ANNOTATION_MEDIA_REDACTION_SCHEMA_VERSION,
};
pub use arlington::{
    arlington_coverage, validate_arlington_dictionary, validate_arlington_dictionary_at_path,
    ArlingtonCoverage, ArlingtonValidationMode,
};
pub use attachments::{
    extract_attachment, extract_attachment_with_limits, list_attachments, sanitize_filename,
    Attachment, AttachmentSource,
};
pub use authoring::{
    CustomFontId, FlowDocument, FontFace, GraphicsStyle, ImageHandle, Margins,
    PageSize as AuthorPageSize, ParagraphStyle, PathBuilder, PdfBuilder, PdfMetadata,
    PdfPageBuilder, StandardFont, TableBuilder, TableCell, TableColumn, TableRow, TableStyle,
    TextAlign, TextStyle,
};
pub use cancel::CancelToken;
pub use chunk::{chunk, estimate_tokens, Chunk, ChunkOptions, ChunkSet, CHUNK_SCHEMA_VERSION};
pub use classify::{
    classify_document, classify_page, ClassifyConfig, PageClassification, PageSource,
};
pub use codec_isolation::{
    codec_backend_registry, codec_dimension_report, codec_isolation_availability_report,
    codec_native_boundary_report, decode_filter_with_isolation, default_worker_path,
    native_codec_dependency_allowlist, native_codecs_compiled, platform_supports_process_isolation,
    select_codec_backend, supported_worker_codecs, validate_codec_registry_policy,
    CodecBackendPreference, CodecBackendRegistryEntry, CodecBackendSelectionReport,
    CodecDimensions, CodecIsolationConfig, CodecIsolationDecode, CodecIsolationLimits,
    CodecIsolationPolicy, CodecIsolationReport, CodecNativeBoundaryReport, CodecWorkerRequest,
    CodecWorkerResponse, NativeCodecDependencyAllowlistEntry, CODEC_WORKER_PROTOCOL_VERSION,
    CODEC_WORKER_VERSION,
};
pub use color_report::{
    color_report, color_report_bytes, ColorBackendDecision, ColorDiagnostic, ColorLimits,
    ColorReport, ColorSeverity, ColorSpaceUsage, ColorValidationProfile, OutputIntentInfo,
    OverprintReport,
};
pub use compliance::{
    convert_to_pdfa, convert_to_pdfa_checked, improve_pdfua_best_effort, validate_pdfa,
    validate_pdfua, ComplianceSeverity, ComplianceViolation, PdfAConversionReport, PdfAProfile,
    PdfAValidationReport, PdfUaValidationReport,
};
pub use compression_office::{
    compression_office_report, inspect_office_package_for_compression_office,
    office_to_pdf_with_report, optimize_pdf as compression_office_optimize_pdf,
    CompressionOfficeCompressionMode, CompressionOfficeCompressionOptions,
    CompressionOfficeDedupFamilyReport, CompressionOfficeDedupReport,
    CompressionOfficeOptimizeOptions, CompressionOfficeOptimizeReport, CompressionOfficeReport,
    CompressionOfficeStatus, CompressionOfficeWriterMode,
    COMPRESSION_OFFICE_CLOSEOUT_SCHEMA_VERSION,
};
pub use content::{
    concat_matrix, BlendMode, Color, ColorSpace, ContentOperation, ContentParser, GraphicsState,
    Matrix, Operand, TextState, IDENTITY_MATRIX,
};
pub use crypto::{
    aes128_cbc_decrypt, aes256_cbc_decrypt, aes256_gcm_decrypt_pdf_object,
    aes256_gcm_encrypt_pdf_object, aes256_gcm_encrypt_pdf_object_tracked,
    aes256_gcm_encrypt_pdf_object_with_nonce, build_encryption, compute_encryption_key,
    decrypt_stream, decrypt_string, derive_v5_file_key_from_owner, derive_v5_file_key_from_user,
    encrypt_bytes, encrypt_bytes_by_method, md5, object_key, r6_hash, verify_user_password,
    verify_v5_owner_password, verify_v5_perms, verify_v5_user_password, CryptMethod,
    EncryptAlgorithm, EncryptParams, EncryptState, EncryptionInfo, Rc4, SecretBytes, V5Fields,
    PADDING,
};
pub use crypto_writer::{
    aes_gcm_report_bytes, crypto_tamper_test_report, crypto_writer_report,
    deterministic_writer_audit, public_key_handler_report_bytes, writer_closeout_report,
    writer_external_diff_report, CryptoWriterFeatureMatrixRow, CryptoWriterReport,
    CryptoWriterStatus, CRYPTO_WRITER_ARTIFACT_ROOT, CRYPTO_WRITER_SCHEMA_VERSION,
};
pub use decode_cache::{DecodeCache, DecodeCacheKey, DecodeCacheMetrics};
pub use decode_scanner::{
    find_marker_accelerated, find_marker_scalar, rfind_marker_accelerated, rfind_marker_scalar,
    scan_pdf_markers_accelerated, scan_pdf_markers_scalar, scanner_availability_report,
    MarkerCandidate, MarkerScanResult, ScannerImplementation, PDF_DELIMITER_MARKERS,
};
pub use decode_scheduler::{
    estimate_image_decode_bytes, estimate_raw_stream_decode_bytes, estimate_stream_decode_bytes,
    estimate_stream_parts_decode_bytes, non_render_decode_scheduler_adoption_report,
    renderer_decode_scheduler_adoption_report, run_scheduled_decode_jobs, DecodeMemoryBudget,
    DecodeSchedulerContext, DecodeSchedulerMetrics, NonRenderDecodeSchedulerAdoptionReport,
    RendererDecodeSchedulerAdoptionReport, ScheduledDecodeJob,
};
pub use docmodel::{
    render_markdown as render_document_markdown, ClassifiedType, DocBlock, DocumentModel, ListItem,
    ModelSource, RegionKind,
};
pub use document::{PdfDocument, PdfPage};
pub use document_security::{
    analyze_document_security, apply_document_security, document_security_feature_matrix,
    plan_document_security, undo_document_security, verify_residual_data,
    AccessibilityMutationKind, DocumentSecurityAction, DocumentSecurityAnalysisReport,
    DocumentSecurityOperationReport, DocumentSecurityRequest, DocumentSecurityStatus,
    DocumentSecuritySubsystem, DocumentSecurityTypedResult, ResidualFinding,
    ResidualVerificationReport, SanitizationPreset, StructureSelector,
    DOCUMENT_SECURITY_SCHEMA_VERSION,
};
pub use document_subsystems::{
    analyze_document_subsystems, apply_document_subsystems, document_subsystems_feature_matrix,
    plan_document_subsystems, undo_document_subsystems, DocumentSubsystemsAction,
    DocumentSubsystemsAnalysisReport, DocumentSubsystemsOperationReport, DocumentSubsystemsRequest,
    DocumentSubsystemsSubsystem, EditableTableGraph, OcrSearchableWord,
    DOCUMENT_SUBSYSTEMS_SCHEMA_VERSION,
};
pub use editable::{
    build_editable_document, build_editable_document_with_parse_options, EditCheckpoint,
    EditOperation, EditPatch, EditSafety, EditTransaction, EditTransactionLog, EditableBlock,
    EditableBuildOptions, EditableDiagnostic, EditableDiagnosticSeverity, EditableDocument,
    EditableImage, EditableListInfo, EditablePage, EditableParagraph, EditableProvenance,
    EditableRole, EditableRun, EditableSection, EditableTable, EditableTableCell,
    EditableTextStyle, EDITABLE_SCHEMA_VERSION,
};
pub use editing::{
    edit_paragraph_reflow_pdf, replace_text_pdf, AnnotationOptions, AttachmentRedactionPolicy,
    DeterministicSaveOptions, DeterministicSaveReport, EditMode, EditRectStyle, EditTextStyle,
    HeaderFooterOptions, ImageRect, ImageRedactionPolicy, ImageStampOptions, OverlayLayer,
    ParagraphEditOperation, ParagraphEditSerializationMode, ParagraphReflowOptions,
    ParagraphReflowReport, PdfEditor, RedactionOptions, TextReplacementOptions,
    TextReplacementReport, WatermarkOptions,
};
pub use editing_transactions::{
    apply_scene_text_transaction, apply_transaction_with_invalidation, build_document_snapshot,
    build_scene_graph, clone_on_write_report, dirty_region_report,
    editing_transactions_feature_matrix, editing_transactions_report, embedding_permission_report,
    font_subset_plan, plan_scene_text_transaction, scene_select, substitution_report,
    text_identity_report, undo_restoration_report, DocumentSnapshot, EditTransactionReport,
    EditableSceneGraph, EditingTransactionsEvidenceKind, EditingTransactionsStatus,
    FontIdentityReport, GraphemeClusterRecord, SceneNode, SceneNodeKind, SceneSelectionReport,
    SceneSelectionRequest, SceneTextEditRequest, ShapingGlyphRecord, TransactionState,
    EDITING_TRANSACTIONS_SCHEMA_VERSION,
};
pub use engine::{
    max_decode_pixels, max_render_pixels, ContentEngine, ExtractionProfile, PageRegion,
    PageResources, PlacedImageReference, RegionImage, RegionWord, DEFAULT_MAX_DECODE_PIXELS,
    DEFAULT_MAX_RENDER_PIXELS,
};
pub use error::{ErrorKind, Result, WellfriendError};
pub use eval::{score, score_json, ScoreInput, ScoreOutput};
pub use extract::{
    extract_fields, DocType, ExtractOptions, ExtractedFields, Field, FieldSource, FieldValue,
    LineItem, ValueHint, FIELDS_SCHEMA_VERSION,
};
pub use filters::{
    decode_image_budget_report, decode_stream, decode_stream_lossless,
    decode_stream_lossless_with_limits, decode_stream_report_from_dict_scheduled,
    decode_stream_with_limits, flate_encode, DecodeDiagnostic, DecodeDiagnosticSource,
    DecodeImageParams, DecodeLimits, DecodeMetrics, DecodePredictorParams, DecodeReport,
    DecodeSeverity, DecodedStream, StreamDecodeStatus, MAX_FLATE_DECOMPRESSED_BYTES,
};
pub use fonts::variations::{AxisValue, VariationRequest};
pub use fonts::{
    BundledFontProvider, FontDecodeSource, FontMatch, FontMatchRequest, FontProvider,
    FontProviderSource, FontResolver, FontType, ShapeOptions, ShapedGlyph, ShapedRun,
    TextDirection, TextShaper,
};
pub use fonts_report::{list_fonts, FontInfo};
pub use form_action_policy::{
    flatten_calculated_values_pdf, form_action_graph, form_action_policy_policy_matrix,
    form_action_policy_report, form_javascript_inventory, form_js_sanitize_pdf,
    interactive_data_closeout_report, word_pagination_audit, ActionInventoryEntry, CalculationEdge,
    CalculationFlattenReport, CalculationResult, CustomActionPolicy, DocxLayoutAuditReport,
    FormActionGraphReport, FormActionPolicySupportStatus, FormJsInventoryReport, FormJsLimits,
    FormJsPolicyMode, FormJsSanitizerOptions, FormJsSanitizerReport,
    FORM_ACTION_POLICY_SCHEMA_VERSION,
};
pub use form_exchange::{
    apply_form_data_pdf, export_form_data, parse_form_data, FormDataApplyReport, FormDataField,
    FormDataFormat, FormDataSet,
};
pub use html::{HtmlExporter, HtmlMode, HtmlOptions};
pub use images::decoder::{ColorSpaceConverter, ImageDecoder, RawImage};
pub use images::encoder::{ImageEncoder, ImageOutputFormat};
pub use images::locator::{ImageLocateOptions, ImageLocator, ImageReference, InlineImageData};
pub use images::smask::SmaskLoader;
pub use info::{
    decode_pdf_text_string, format_pdf_date, DocumentInfo, EncryptionReport, PageSize, Permissions,
};
pub use interactive::{
    annotation_report, forms_report, interactive_report, page_operations_report,
    redaction_verification_report, AnnotationActionInfo, AnnotationInfo, AnnotationReport,
    FieldAttributeSource, FormFieldReport, FormReport, FormWidgetReport, InteractiveDiagnostic,
    InteractiveReport, PageBoxReport, PageOperationsReport, RedactionVerificationReport, XfaReport,
};
pub use object::{PdfDictionary, PdfObject};
pub use ocr::preprocess::{
    binarize_otsu, binarize_sauvola, detect_skew, preprocess, Binarization, PreprocessConfig,
};
pub use ocr::{OcrEngine, OcrImage, OcrOptions, OcrPage, OcrPolicy, OcrWord};
pub use office::{
    docx_to_pdf, inspect_office_package, pdf_to_docx, pdf_to_pptx, pdf_to_xlsx, pptx_to_pdf,
    xlsx_to_pdf, DocxLayout, DocxOptions, OfficeFormat, OfficePackageSecurityLimits,
    OfficePackageSecurityReport, OfficeToPdfOptions, PptxOptions, XlsxLayout, XlsxOptions,
};
pub use optional_content::{
    OptionalContentContext, OptionalContentLayerReport, OptionalContentMembershipReport,
    OptionalContentReport,
};
pub use parse::{
    parse, Block, BlockKind, Document, DocumentMetadata, ImageHandling, ImageRef, InlineSpan,
    InlineText, ListEntry, Page, ParseOptions, SerializeOptions, SourceInfo, SCHEMA_VERSION,
};
pub use parser_report::{
    arlington_status, parser_report_bytes, parser_report_bytes_with_options,
    parser_report_bytes_with_password, ArlingtonIntegrationStatus, DiagnosticCounts,
    LinearizationInfo, ParserCategory, ParserDiagnostic, ParserMode, ParserReport,
    ParserReportOptions, ParserSeverity, ParserSourceMetrics, RepairSummary, RevisionHistory,
    RevisionSection,
};
pub use pdf_mac::{
    pdf_mac_report, pdf_mac_report_bytes, pdf_mac_verify_report_bytes, write_standalone_pdf_mac,
    PdfMacReport, PdfMacState, PdfMacWriteReport,
};
pub use prepress::{
    classify_icc_profile, IccProfileClass, IccProfileInfo, NChannelPixelFormatReport,
    NChannelSample, NchannelPlatePrepressPrepressReport, PlateContribution, PlateKind,
    PlatePreviewHash, PlatePreviewReport, PlateSummary, PrepressCMMPrepressReport,
    RenderingIntentBpcReport, SeparationFramebuffer, SeparationFramebufferReport,
    DEFAULT_SEPARATION_FRAMEBUFFER_BUDGET_BYTES, MAX_NCHANNEL_OUTPUT_CHANNELS, MAX_PREPRESS_PLATES,
};
pub use pubsec::{
    encrypt_pdf_pubsec, parse_pubsec_encryption_info, recover_pubsec_file_key,
    reencrypt_pdf_pubsec, PubSecEncryptOptions, PubSecEncryptReport, PubSecEncryptionInfo,
    PubSecIdentity, PubSecKeyProvider, PubSecRecipientCertificate, PubSecRecipientIdMode,
    PubSecRecoveredKey,
};
pub use reader::{EncryptionContext, PdfReader, XrefEntry};
pub use render::{
    flatten_cubic, flatten_path, get_fallback_font, map_refs_to_canonical_ids, path_raster_stats,
    pixel_buffer_allocation_stats, pixel_compositor_backend,
    pixel_compositor_detected_hardware_backend, pixel_compositor_operation_backend,
    pixel_compositor_stats, rgb, rgba, AlphaMask, CachedGlyph, CanonicalDocument, ClipMask,
    ColorManagementPolicy, ColorSpaceHandler, CompositingPolicy, ContractColor, CpuRenderDevice,
    DashState, DeviceClip, DeviceMatrix, DisplayItemId, DisplayList, DisplayListStats, DisplayOp,
    DrawState, EditDocumentView, ExactnessPolicy, FillRule, FlatPath, FontRasterizer,
    FontSubstitutionEvent, FontSubstitutionLog, FontSubstitutionMetricPosture,
    FontSubstitutionReason, FormRenderPolicy, GlyphCache, GlyphCacheKey, GlyphCacheStats,
    ImagePainter, InvalidationResult, LinePainter, ObjectIdentityId, PageBox, PageIdentity,
    PageRenderer, ParsedPageProgram, Path, PathPainter, PathRasterStats, PathSegment, PixelBuffer,
    PixelBufferAllocationStats, PixelColor, PixelCompositorBackend, PixelCompositorOperation,
    PixelCompositorStats, PrintProfile, PrintProfileRefusal, PrintProfileRefusalCategory,
    ProgressiveRenderJob, ProgressiveRenderState, ProgressiveRenderStepReport,
    ProgressiveRenderToken, RenderArtifactCacheStats, RenderCache, RenderCacheKey,
    RenderCacheMetrics, RenderColor, RenderContract, RenderDependencyGraph, RenderDevice,
    RenderDocumentCache, RenderDocumentView, RenderMode, RenderPlan, RenderQuality,
    RenderResourceBudget, RenderTile, ResourceId, RevisionId, SemanticDocumentView, SourceLinkId,
    SvgPage, TransactionInvalidationResult, TransactionWriteSet, Transform2D, UnsupportedRenderOp,
    ValidationDocumentView, ViewMaterializationStats, Viewport, WuLineRenderer, BLACK, BLUE, GREEN,
    RED, TRANSPARENT, WHITE,
};
pub use render::{render_page_svg, svg, text_decode};
pub use runtime::*;
pub use sdk::REPORT_ENVELOPE_VERSION;
pub use secure_mutation::{
    analyze_edit_policy, analyze_edit_policy_for_target, apply_signature_preserving_form_fill,
    associated_file_extract, associated_files_add_pdf, associated_files_inventory,
    associated_files_remove_owner_pdf, associated_files_sanitize_pdf,
    associated_files_update_owner_pdf, incremental_annotation_update_pdf,
    incremental_form_value_update_pdf, incremental_metadata_update_pdf,
    incremental_page_property_update_pdf, mask_redaction_inventory, redact_masked_images_pdf,
    secure_mutation_closeout_report, secure_mutation_report, AfRelationship,
    AssociatedFileAddRequest, AssociatedFileOwnerRemoveRequest, AssociatedFileOwnerType,
    AssociatedFileOwnerUpdateRequest, AssociatedFileRecord, AssociatedFileSanitizerOptions,
    AssociatedFileSanitizerPolicy, AssociatedFilesInventoryReport, AssociatedFilesMutationReport,
    EditOperation as SignatureEditOperation, EditPolicyDecision, EditPolicyReport,
    IncrementalAnnotationEdit, IncrementalMutationReport, IncrementalPagePropertyEdit,
    MaskInventoryReport, MaskInventoryRow, MaskRedactionStrategy, PostEditSignatureReport,
    SecureMutationSupportStatus, SignatureImpactSummary, SignaturePreservingEditPlan,
    SignaturePreservingEditResult, StructuralSignaturePolicy,
    SECURE_MUTATION_CLOSEOUT_SCHEMA_VERSION, SECURE_MUTATION_SCHEMA_VERSION,
};
pub use security::{
    canonicalize_pdf, sanitize_pdf, scan_risky_content, security_report, CanonicalizeOptions,
    CanonicalizeReport, RiskyContentReport, SanitizerOptions, SanitizerPolicy, SanitizerReport,
    SecurityFinding, SecurityReport, SecuritySeverity,
};
pub use semantic::{SemanticDocument, SemanticElement, SemanticMcid, SemanticSource};
pub use semantic_binding::{
    build_semantic_binding_report, semantic_search_report, SemanticBindingOptions,
    SemanticBindingReport, SemanticBindingSummary, SemanticCjkTokenPage, SemanticPageTables,
    SemanticPrivacyStatus, SemanticSearchReport, SEMANTIC_BINDING_SCHEMA_VERSION,
};
pub use semantic_intelligence::{
    merge_layout_proposals_deterministic, recover_parenttree_semantics,
    semantic_elements_from_parenttree_recovery, validate_layout_proposal_set,
    CloudLayoutBackendConfig, CloudLayoutContractBackend, HeuristicLocalLayoutBackend,
    LayoutAvailabilityReport, LayoutBackendDescriptor, LayoutBackendInput, LayoutBackendKind,
    LayoutBackendStatus, LayoutCloudPayloadPolicy, LayoutDiagnostic, LayoutInputPayloadKind,
    LayoutLocalBackendConfig, LayoutMergeOutcome, LayoutMergePolicy, LayoutMergeReport,
    LayoutPrivacyMode, LayoutProposalRegion, LayoutProposalSet, LayoutRegionGeometry,
    LayoutRegionLabel, ParentTreeDiagnostic, ParentTreePageSummary, ParentTreeRecoveredNode,
    ParentTreeRecoveryReport, ParentTreeRecoveryStatus, SemanticEvidenceKind,
    SemanticIntelligenceSemanticIntelligenceReport,
};
pub use signature::{
    add_ltv_material, plan_signature_placeholder, sign_document, sign_incremental,
    verify_options_from_json, verify_signature_timestamp_token_der, verify_signatures,
    verify_signatures_with_options, verify_signatures_with_options_and_evidence, CertInfo,
    CertificatePathValidationReport, CertificateRevocationDecision, CmsSigningRequest,
    CmsSigningResult, ConfiguredTrustAnchor, Coverage, DocumentTimestampStatus, ExternalSigner,
    IncrementalSignResult, IncrementalSigner, IncrementalSigningOptions, IntermediateStore,
    LtvMaterial, LtvReport, NetworkEvidenceReport, NetworkValidationReport,
    PadesLTVSignatureLtvEditReport, PadesLevel, PadesValidationReport, PdfSigner,
    PostSignValidationReport, RevocationStatus, RevocationValidationReport,
    SignatureAlgorithmPolicy, SignatureCheckDetails, SignatureOptions, SignaturePlaceholderPlan,
    SignatureReport, SignatureRevocationMode, SignatureStatus, SignatureTrust,
    SignatureValidationIndication, SignatureValidationOutcome, SignatureValidationPolicyProfile,
    SignatureValidationPolicyReport, SignatureValidationSignatureValidationReport,
    SignatureValidationState, SignatureValidationSubindication, SignatureValidity, SigningIntent,
    TimestampTokenType, TimestampValidationReport, TrustStore, VerifyOptions,
    PADES_LTV_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
    SIGNATURE_VALIDATION_SIGNATURE_VALIDATION_SCHEMA_VERSION,
};
pub use signature_evidence::{
    EvidenceBundle, EvidenceKind, EvidenceRecord, EvidenceStore, NetworkBudget, OcspNoncePolicy,
    RetrievalKind, RetrievalMethod, RetrievalPolicy, RetrievalTrace,
};
pub use source_editing::{
    edit_path_operator, edit_text_operator, operator_image_eligibility, operator_path_provenance,
    operator_text_eligibility, operator_text_provenance, source_editing_report,
    OperatorEditOperationReport, OperatorEditRefusal, OperatorTextEditRequest,
    OperatorTextEligibilityReport, ProvenanceSelectionReport, ProvenanceStrength,
    SourceInstructionIdentity, TrueEditingMode, SOURCE_EDITING_SCHEMA_VERSION,
};
pub use standards::{
    validate_standards_profile, StandardsProfile, StandardsValidationReport, ValidationRuleResult,
    ValidationSeverity, ValidationStatus,
};
pub use standards_engine::{
    validate_all_standards, validate_pdfa_profile, validate_pdfua_profile, validate_pdfx_profile,
    validate_standards_family, ConformanceStatus, CrossProfileConflict, CrossProfileConflictReport,
    ProfileDetection, RuleImplementation, RuleStatus, StandardsClauseRef, StandardsFamily,
    StandardsRuleCounts, StandardsRuleResult, StandardsValidationOptions, ValidationEvidence,
    STANDARDS_ENGINE_SCHEMA_VERSION,
};
pub use structural::{
    crop_pages, encrypt, linearize::linearize, optimize, repair, rotate_pages, OptimizeReport,
    Rotation,
};
pub use table_intelligence::{
    merge_table_proposals_deterministic, mock_tableformer_proposal_set,
    table_model_backend_status_report, validate_table_proposal_set, DeterministicTableEvidence,
    MergedTableOverlay, TableBoundaryKind, TableBoundaryProposal, TableCellProposal,
    TableCoordinateTransform, TableModelBackendStatusReport, TableModelMetadata,
    TablePreprocessingMetadata, TableProposalMergeOutcome, TableProposalMergeOutcomeKind,
    TableProposalMergePolicy, TableProposalMergeReport, TableProposalProvenance, TableProposalSet,
    TableProposalValidationReport, TableSectionRole, TableStructureProposal,
    TABLE_PROPOSAL_MERGE_SCHEMA_VERSION, TABLE_PROPOSAL_SCHEMA_VERSION,
};
pub use text::{
    bounded_text_parallel_window, builtin_cjk_dictionary_metadata, cjk_dictionary_entries_sha256,
    cjk_dictionary_rag_token_chunks, cjk_dictionary_token_search, segment_cjk_dictionary_text,
    segment_cjk_dictionary_text_with_provider, CjkDictionaryLoadDiagnostic,
    CjkDictionaryLoadReport, CjkDictionaryMetadata, CjkDictionaryPackManifest,
    CjkDictionaryPackStatus, CjkDictionaryProvider, CjkDictionaryProviderLimits,
    CjkDictionaryToken, CjkRagTokenChunk, CjkSegmentationMode, CjkTokenSearchMatch, LineEnding,
    MarkedTextChunk, ReadingOrderReconstructor, SemanticTextDirection, TextChunk, TextCollector,
    TextDiagnostic, TextExtractOptions, TextExtractionCounters, TextExtractionMode, TextExtractor,
    TextFormatOptions, TextFormatter, TextLayoutStrategy, TextLine, TextMappingSource,
    TextProvenanceFlag, TextProvenanceSummary, TextQuad, TextRole, TextRoleSource, TextSearchMatch,
    TextSearchOptions, TextSemanticBlock, TextSemanticChar, TextSemanticDocument, TextSemanticLine,
    TextSemanticOptions, TextSemanticPage, TextSemanticParagraph, TextSemanticSpan,
    TextSemanticWord, TextStructureContext, TextStructureEntry, TextStructurePageSummary,
};
pub use text_reflow::{
    analyze_geometric_region, analyze_semantic_layout, apply_reflow_document, apply_reflow_region,
    approve_structure_correction, evaluate_reflow_confidence, flow_graph_report, line_break_text,
    no_overlay_no_clipping_report, paragraph_style_model, preview_reflow, query_confidence,
    query_constraints, query_overflow, reading_order_report, score_reading_order_fixture,
    text_reflow_feature_matrix, text_reflow_report, transaction_undo_report,
    undo_reflow_from_replay, validate_reflow_output, ConfidenceDecision, ConstraintSolverReport,
    GeometricReflowRequest, GeometricTextRegion, LayoutConstraint, LayoutLine, LineBreakRecord,
    LineBreakingResult, NeighborPolicy, OverflowStatus, ParagraphStyleModel,
    ReflowConfidencePolicy, ReflowMutationSession, ReflowTransactionReport, ReflowUndoReport,
    SemanticLayoutReport, SemanticRegionEdge, SemanticRegionNode, TextReflowEvidenceKind,
    TextReflowStatus, TEXT_REFLOW_SCHEMA_VERSION,
};
pub use utilities::{
    add_page_numbers_pdf, attachments_json, crop_pdf, crop_pdf_pages, decrypt_pdf, encrypt_pdf,
    encrypt_pdf_with_pdf_mac, export_pdf_pages_to_images, fonts_json, html_string,
    images_to_pdf_from_bytes, images_to_pdf_from_paths, linearize_pdf, n_up_pdf, optimize_pdf,
    organize_pdf, organize_pdf_with_insert, render_page_image, repair_pdf, rotate_pdf,
    scale_pdf_pages, signatures_json, watermark_image_pdf, watermark_text_pdf, ImagePdfPageSize,
    ImageToPdfOptions, ImageWatermarkOptions, NUpOptions, PageNumberOptions, RasterImageFormat,
    RasterPageResult, RgbColor, ScalePagesOptions, StampPosition, TextWatermarkOptions,
};
pub use versioning::{
    content_defined_chunks, hamming_distance, resource_dedup_report, resource_digest, simhash_text,
    ContentChunk, ResourceDedupGroup, ResourceDedupReport,
};
pub use writer::{
    build_merged, build_subset, rewrite_document, rewrite_document_objects,
    rewrite_document_with_mode, rewrite_references, serialize_object, write_document_linearized,
    write_document_roundtrip, OutputObject, PdfWriter, WriterMode,
};
pub use writer_history::{
    font_reconstruction_report, object_stream_packing_report, pack_object_streams_pdf,
    persistent_store_report, raster_vectorization_report, vectorize_raw_image,
    writer_history_report, FontReconstructionReport, ObjectStreamPackingReport,
    PersistentStoreReport, RasterVectorOutputMode, RasterVectorizationOptions,
    RasterVectorizationReport, WriterHistoryReport, WRITER_HISTORY_ARTIFACT_ROOT,
    WRITER_HISTORY_SCHEMA_VERSION,
};
pub use xfa::{
    extract_xfa, sanitize_xfa_pdf, xfa_flatten_pdf, xfa_inventory, xfa_inventory_cancellable,
    xfa_render_preview_pdf, xfa_runtime_report, xfa_runtime_report_cancellable,
    xfa_security_report, XfaBindingRecord, XfaClassification, XfaDataNode, XfaDiagnostic,
    XfaDrawRecord, XfaEventRecord, XfaExtractionReport, XfaFieldRecord, XfaFlattenMode,
    XfaFlattenOptions, XfaFlattenReport, XfaGeometry, XfaInventoryReport, XfaLayoutItem,
    XfaLayoutRect, XfaLimits, XfaOccur, XfaPacketRecord, XfaProvenance, XfaRagChunk,
    XfaRedactionPosture, XfaReopenVerification, XfaRuntimeMetrics, XfaRuntimeOptions,
    XfaRuntimeReport, XfaSandboxAuditEntry, XfaSandboxReport, XfaSanitizerMode,
    XfaSanitizerOptions, XfaSanitizerReport, XfaScriptPolicy, XfaScriptRecord, XfaSecurityReport,
    XfaSemanticIntegrationReport, XfaSignatureImpact, XfaSubformRecord, XfaSupportStatus,
    XfaXmlMetrics, XfaXmlSafetyReport, XFA_SCHEMA_VERSION,
};

/// The curated high-level embedding surface.
///
/// The crate root re-exports a large, flat surface that mixes the high-level
/// embedder API with low-level building blocks (renderer internals, font and
/// crypto primitives, the raw object/reader types). That breadth is useful for
/// advanced consumers but obscures the path most embedders want. This module
/// gathers exactly the types and functions needed to **open a document, parse
/// it to the canonical [`Document`] model, serialize it (Markdown / JSON /
/// HTML), chunk it for RAG, extract key-value fields, run the structural ops,
/// export Office formats, and inject an OCR backend** — nothing more.
///
/// ```no_run
/// use wellfriendpdf_engine::prelude::*;
///
/// # fn main() -> wellfriendpdf_engine::Result<()> {
/// let engine = ContentEngine::open_path("input.pdf")?;
///
/// // Parse → canonical model → Markdown / JSON for RAG and automation.
/// let doc = engine.parse_document(&ParseOptions::default())?;
/// let markdown = doc.to_markdown_default();
/// let json = doc.to_json();
///
/// // RAG-ready semantic chunks.
/// let chunks = doc.chunk(&ChunkOptions::default());
///
/// // Structured key-value fields (invoice/receipt/form).
/// let fields = engine.extract_fields(&ExtractOptions::default())?;
/// # let _ = (markdown, json, chunks, fields);
/// # Ok(())
/// # }
/// ```
///
/// To inject OCR for scanned pages, supply a concrete [`OcrEngine`] (e.g. the
/// `wellfriendpdf-ocr-tesseract` crate) via [`ParseOptions::ocr`] /
/// [`ExtractOptions::ocr`]. Everything here works **without** the CLI, the
/// server, or any non-Rust binding.
pub mod prelude {
    pub use crate::annotation_media_redaction::{
        apply_nonaxis_image_redaction_pdf, apply_rich_media_policy_pdf, export_annotation_xfdf,
        generate_annotation_appearances_pdf, import_annotation_xfdf_pdf,
        plan_nonaxis_image_redaction, rich_media_inventory, AnnotationAppearanceOptions,
        AnnotationXfdfImportOptions, NonAxisRedactionOptions, NonAxisRedactionRequest,
        RichMediaPolicyMode,
    };
    pub use crate::authoring::{
        CustomFontId, FlowDocument, FontFace, GraphicsStyle, ImageHandle, Margins,
        PageSize as AuthorPageSize, ParagraphStyle, PathBuilder, PdfBuilder, PdfMetadata,
        PdfPageBuilder, StandardFont, TableBuilder, TableCell, TableColumn, TableRow, TableStyle,
        TextAlign, TextStyle,
    };
    pub use crate::chunk::{chunk, Chunk, ChunkOptions, ChunkSet, CHUNK_SCHEMA_VERSION};
    pub use crate::compliance::{
        convert_to_pdfa, convert_to_pdfa_checked, improve_pdfua_best_effort, validate_pdfa,
        validate_pdfua, ComplianceSeverity, ComplianceViolation, PdfAConversionReport, PdfAProfile,
        PdfAValidationReport, PdfUaValidationReport,
    };
    pub use crate::compression_office::{
        compression_office_report, inspect_office_package_for_compression_office,
        office_to_pdf_with_report, optimize_pdf as compression_office_optimize_pdf,
        CompressionOfficeCompressionMode, CompressionOfficeCompressionOptions,
        CompressionOfficeDedupFamilyReport, CompressionOfficeDedupReport,
        CompressionOfficeOptimizeOptions, CompressionOfficeOptimizeReport, CompressionOfficeReport,
        CompressionOfficeStatus, CompressionOfficeWriterMode,
        COMPRESSION_OFFICE_CLOSEOUT_SCHEMA_VERSION,
    };
    pub use crate::crypto_writer::{
        aes_gcm_report_bytes, crypto_tamper_test_report, crypto_writer_report,
        deterministic_writer_audit, public_key_handler_report_bytes, writer_closeout_report,
        writer_external_diff_report, CryptoWriterFeatureMatrixRow, CryptoWriterReport,
        CryptoWriterStatus, CRYPTO_WRITER_ARTIFACT_ROOT, CRYPTO_WRITER_SCHEMA_VERSION,
    };
    pub use crate::editable::{
        build_editable_document, EditableBuildOptions, EditableDocument, EditableRole,
        EDITABLE_SCHEMA_VERSION,
    };
    pub use crate::editing::{
        edit_paragraph_reflow_pdf, replace_text_pdf, AnnotationOptions, AttachmentRedactionPolicy,
        DeterministicSaveOptions, DeterministicSaveReport, EditMode, EditRectStyle, EditTextStyle,
        HeaderFooterOptions, ImageRect, ImageRedactionPolicy, ImageStampOptions, OverlayLayer,
        ParagraphEditOperation, ParagraphEditSerializationMode, ParagraphReflowOptions,
        ParagraphReflowReport, PdfEditor, RedactionOptions, TextReplacementOptions,
        TextReplacementReport, WatermarkOptions,
    };
    pub use crate::engine::ContentEngine;
    pub use crate::error::{ErrorKind, Result, WellfriendError};
    pub use crate::eval::{score, score_json, ScoreInput, ScoreOutput};
    pub use crate::extract::{
        extract_fields, DocType, ExtractOptions, ExtractedFields, Field, FieldValue, LineItem,
    };
    pub use crate::ocr::{OcrEngine, OcrOptions};
    pub use crate::office::{
        docx_to_pdf, inspect_office_package, pdf_to_docx, pdf_to_pptx, pdf_to_xlsx, pptx_to_pdf,
        xlsx_to_pdf, DocxLayout, DocxOptions, OfficeFormat, OfficePackageSecurityLimits,
        OfficePackageSecurityReport, OfficeToPdfOptions, PptxOptions, XlsxLayout, XlsxOptions,
    };
    pub use crate::parse::{
        parse, Block, BlockKind, Document, DocumentMetadata, Page, ParseOptions, SerializeOptions,
        SourceInfo, SCHEMA_VERSION,
    };
    pub use crate::secure_mutation::{
        analyze_edit_policy, apply_signature_preserving_form_fill, associated_file_extract,
        associated_files_add_pdf, associated_files_inventory, associated_files_remove_owner_pdf,
        associated_files_sanitize_pdf, associated_files_update_owner_pdf,
        incremental_annotation_update_pdf, incremental_form_value_update_pdf,
        incremental_metadata_update_pdf, incremental_page_property_update_pdf,
        mask_redaction_inventory, redact_masked_images_pdf, secure_mutation_closeout_report,
        secure_mutation_report, AssociatedFileAddRequest, AssociatedFileOwnerRemoveRequest,
        AssociatedFileOwnerUpdateRequest, AssociatedFileSanitizerOptions,
        EditOperation as SignatureEditOperation, IncrementalAnnotationEdit,
        IncrementalPagePropertyEdit, PostEditSignatureReport, SignaturePreservingEditPlan,
        SignaturePreservingEditResult,
    };
    pub use crate::signature::{
        add_ltv_material, sign_document, verify_options_from_json,
        verify_signature_timestamp_token_der, verify_signatures, verify_signatures_with_options,
        verify_signatures_with_options_and_evidence, CertInfo, CertificatePathValidationReport,
        CertificateRevocationDecision, ConfiguredTrustAnchor, Coverage, IntermediateStore,
        LtvMaterial, LtvReport, NetworkValidationReport, PadesLTVSignatureLtvEditReport,
        PadesLevel, PadesValidationReport, PdfSigner, RevocationStatus, RevocationValidationReport,
        SignatureAlgorithmPolicy, SignatureOptions, SignatureReport, SignatureRevocationMode,
        SignatureStatus, SignatureTrust, SignatureValidationIndication, SignatureValidationOutcome,
        SignatureValidationPolicyProfile, SignatureValidationPolicyReport,
        SignatureValidationSignatureValidationReport, SignatureValidationState,
        SignatureValidationSubindication, SignatureValidity, TimestampTokenType,
        TimestampValidationReport, TrustStore, VerifyOptions,
        PADES_LTV_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
        SIGNATURE_VALIDATION_SIGNATURE_VALIDATION_SCHEMA_VERSION,
    };
    pub use crate::writer::{build_merged, build_subset};
    pub use crate::ENGINE_VERSION;
}

/// Compile-time guarantee that the parsed engine is thread-safe.
///
/// `ContentEngine` (and the `PdfDocument`/`PdfReader` it owns) must stay
/// `Send + Sync` so a single parsed document can be wrapped in an `Arc` and
/// shared across rayon worker threads for parallel text extraction and page
/// rendering — instead of cloning and re-parsing the whole PDF per thread.
/// The only interior mutability in the reader (the object-stream cache) is an
/// `RwLock` precisely to preserve this. If a future change reintroduces a
/// `RefCell`/`Rc`/raw pointer into the parse tree, this assertion fails to
/// compile, flagging the regression immediately.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ContentEngine>();
    assert_send_sync::<PdfDocument>();
    assert_send_sync::<PdfReader>();
};
