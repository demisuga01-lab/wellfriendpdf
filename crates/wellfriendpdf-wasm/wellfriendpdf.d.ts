export default function init(input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module): Promise<unknown>;

export type ReportJson = string;

export interface FormTextInvocation {
  resource_name: string;
  owner_stream_object: number;
  owner_stream_generation: number;
  owner_operation_byte_start: number;
  owner_operation_byte_end: number;
  form_object: number;
  form_generation: number;
  depth: number;
}
/** Exact input-revision binding from formTextSourcesJson. Rediscover after
 * unrelated mutations; editFormText reports the saved target for a second edit.
 */
export interface FormTextTarget {
  input_sha256: string;
  page: number;
  content_stream_index: number;
  invocation_path: FormTextInvocation[];
}

export interface OcrCarrierSelection {
  span_ids: string[];
  expected_text: string;
  /** Required for an invisible carrier inside a nested Form occurrence. */
  form_target?: FormTextTarget | null;
}

/** Exact selected normal appearance and nested Form source occurrence.
 * Geometry is source-local, before Matrix/Rect placement. */
export interface AppearanceTextTarget {
  input_sha256: string;
  page: number;
  annotation_index: number;
  annotation: [number, number];
  appearance_stream: [number, number];
  normal_state: string | null;
  invocation_path: FormTextInvocation[];
}
export type AppearanceMetadataPolicy = "preserve_annotation_metadata" | "synchronize_free_text_plain_text";
/** Default reject preserves the previous behavior. Splitting shared namespaces
 * explicitly approves placing the new carrier after the old under the same
 * logical owner; it does not infer reading order or certify accessibility. */
export type TaggedAppearanceClonePolicy = "reject" | "move_exclusive_namespaces" | "split_shared_namespaces_after_source";
export interface TaggedAppearanceCloneOptions {
  policy?: TaggedAppearanceClonePolicy;
  actual_text_updates?: Array<{
    element: [number, number];
    expected_text: string;
    replacement_text: string;
  }>;
}
/** Add `operation: {kind: "scoped_text", request: ...}` to a universal v2
 * request. `source.request.edit` is the existing native MultiRunTextRangeRequest.
 * Planner-owned planned_output_sha256 must be omitted from new requests. */
export interface UniversalScopedTextRequest {
  source:
    | { scope: "form"; request: { target: FormTextTarget; edit: Record<string, unknown>; shared_form_policy: "clone_edit_one_instance" | "edit_all_uses" } }
    | { scope: "appearance"; request: { target: AppearanceTextTarget; edit: Record<string, unknown>; metadata_policy: AppearanceMetadataPolicy; tagged_clone?: TaggedAppearanceCloneOptions } }
    | { scope: "widget_field"; request: WidgetTextEditRequest };
  approved_font_asset?: { lookup_name: string; bytes: number[] } | null;
}

export interface WidgetTextTarget { input_sha256:string; page:number; field:[number,number] }
export type WidgetDefaultAppearance = {kind:"preserve_source_defaults"}
  | {kind:"from_edited_appearance";widget:[number,number];font_resource:string;font_size:number;rgb:[number,number,number]};
export interface WidgetTextEditRequest {
  target:WidgetTextTarget; expected_value:string; replacement_value:string;
  widgets:{appearance:{target:AppearanceTextTarget;edit:Record<string,unknown>;metadata_policy:"preserve_annotation_metadata";
    tagged_clone?:TaggedAppearanceCloneOptions};expected_display:string;replacement_display:string}[];
  default_appearance:WidgetDefaultAppearance;update_default_value?:boolean;discard_rich_text?:boolean;
  allow_read_only?:boolean;preserve_actions_without_execution?:boolean;
}

export interface ImageFragmentBinding {
  key: string;
  page: number;
  rect: [number, number, number, number];
  content_sha256: string;
}
export interface ImageFragmentMove {
  input_sha256: string;
  source: { kind: "occurrence"; page: number; content_stream_index: number; occurrence_id: string }
    | { kind: "owned"; binding: ImageFragmentBinding };
  target_page: number;
  target_rect: [number, number, number, number];
  stack: "background" | "foreground";
  /** Initial occurrence only; exact IDs/text from imageOcrSourcesJson.
   * Owned groups retain their existing OCR automatically. Not OCR recognition.
   * expected_text concatenates font-decoded selected spans in stream order;
   * complete direct ActualText scopes are preserved separately.
   */
  ocr?: OcrCarrierSelection | null;
  signature_policy_override?: boolean;
}
/** Entry in a linked-story request's figures array. Initial OCR decisions are
 * consumed on save; reloaded owned groups carry their search layer intact.
 */
export interface StoryFigure {
  id: string;
  caption_paragraph: string;
  source: ImageFragmentMove["source"];
  ocr?: OcrCarrierSelection | null;
  ocr_unrelated?: boolean;
  width: number;
  height: number;
  gap?: number;
  alignment?: "left" | "center" | "right";
  stack: "background" | "foreground";
}
/** Standalone source-preserving image move, not automatic story/caption reflow.
 * Call in a worker. Preview is geometry/source evidence, not a raster preview.
 * It does not mutate an existing StoryEditSession. Reopen that session explicitly
 * after accepting output; prior session receipts then belong to the old revision.
 */
export function imageFragmentBindingsJson(input: Uint8Array): ReportJson;
/** Page-logical source spans, including render mode and exact span IDs.
 * Nested Form carriers come from formTextSourcesJson and include form_target.
 */
export function imageOcrSourcesJson(input: Uint8Array, page: number): ReportJson;
export function previewImageFragmentMoveJson(input: Uint8Array, requestJson: string): ReportJson;
export function applyImageFragmentMove(input: Uint8Array, requestJson: string, approvedPlanSha256: string): ImageFragmentOutput;
export class ImageFragmentOutput {
  bytes(): Uint8Array;
  reportJson(): ReportJson;
  free(): void;
  private constructor();
}

/** Retained native PDF editor. Run synchronous WASM methods in a dedicated worker.
 * previewJson yields geometry and a revision/request-bound receipt, not raster proof.
 * Merge methods return candidates/conflicts; they do not mutate or grant approval.
 * Encrypted input requires its permissions/owner password and becomes an
 * explicitly reported unencrypted working revision. Encrypted output uses the
 * universal API.
 */
export class StoryEditSession {
  constructor(bytes: Uint8Array);
  static openWithPassword(bytes: Uint8Array, password: Uint8Array): StoryEditSession;
  /** Shared v1 native/browser command envelope, limited to 32 MiB UTF-8 JSON. */
  commandJson(commandJson: string): ReportJson;
  bytes(): Uint8Array;
  revisionSha256(): string;
  savedStoriesJson(): ReportJson;
  pagesJson(): ReportJson;
  sourceModelJson(page: number): ReportJson;
  annotationSourcesJson(): ReportJson;
  imageSourcesJson(page: number): ReportJson;
  tagSourcesJson(): ReportJson;
  /** Pure draft evaluation. Table rowspan/tag ownership travels in table_layout;
   * no PDF bytes are published and no preview approval is granted here. */
  synchronizeTableValuesJson(requestJson: string): ReportJson;
  pageGeometryJson(page: number, dpi: number): ReportJson;
  previewJson(requestJson: string): ReportJson;
  checkpointJson(requestJson: string, receiptJson: string): ReportJson;
  mergeTextJson(requestJson: string): ReportJson;
  mergeStructureJson(requestJson: string): ReportJson;
  renderPagePng(page: number, dpi: number): Uint8Array;
  undo(): boolean;
  redo(): boolean;
  close(): void;
  free(): void;
}

/** Interactive XFA form session (set values, clicks, events, recalculation,
 * validation, rendering and saving the datasets packet). Commands are JSON
 * objects with an "op": fields, value, set_value, click, dispatch,
 * validations, script_log, export_data_xml, page_count, form_type.
 */
export class XfaSession {
  /** `optionsJson`: optional `{"run_scripts": boolean}`. */
  constructor(bytes: Uint8Array, optionsJson?: string | null);
  commandJson(commandJson: string): ReportJson;
  renderPagePng(page: number, dpi: number): Uint8Array;
  /** The PDF with the session's data in its datasets packet (incremental update). */
  savePdf(): Uint8Array;
  layoutPdf(): Uint8Array;
  close(): void;
  free(): void;
}

export class WellfriendOutput {
  bytes(): Uint8Array;
  byteLength(): number;
  reportJson(): ReportJson;
  private constructor();
  free(): void;
}

export type RenderContractPixelFormat = "Rgba8" | "Bgra8" | "Rgb8" | "Bgr8" | "Gray8" | "rgba8" | "bgra8" | "rgb8" | "bgr8" | "gray8" | "gray" | "grey8" | "grey";
export type RenderContractAlphaMode = "Premultiplied" | "Straight" | "Opaque" | "premultiplied" | "straight" | "opaque";
export type RenderContractBudgetValue = bigint | number;
export type RenderContractPageBox = "Media" | "Crop" | "Bleed" | "Trim" | "Art" | "media" | "crop" | "bleed" | "trim" | "art";
/** Standard is the only execution mode; "Research"/"research" are deprecated aliases mapped to Standard. */
export type RenderContractExecutionMode = "Standard" | "standard" | "Research" | "research";
/** "ResearchHybrid"/"research-hybrid" are deprecated aliases of "Hybrid". */
export type RenderContractBackendSelection = "ScalarReference" | "StandardCpu" | "Hybrid" | "scalar-reference" | "standard-cpu" | "hybrid" | "ResearchHybrid" | "research-hybrid";
export type RenderContractCompositingPolicy = "Compatibility" | "HighQuality" | "compatibility" | "high-quality";
export type RenderContractIncludePolicy = "Include" | "Exclude" | "include" | "exclude";
export type RenderContractSmoothingPolicy = "Disabled" | "Antialiased" | "Subpixel" | "disabled" | "antialiased" | "subpixel";
export type RenderContractColorScheme = "Light" | "Dark" | "ForcedMonochrome" | "light" | "dark" | "forced-monochrome";
export type RenderContractPrintProfile = "Display" | "Print" | "Proof" | "display" | "print" | "proof";
export type RenderContractHalftonePolicy = "Disabled" | "Screen" | "disabled" | "screen";
export type RenderContractOverprintPolicy = "Disabled" | "Preview" | "PreserveSeparations" | "disabled" | "preview" | "preserve-separations";
export type RenderContractRenderingIntent = "RelativeColorimetric" | "AbsoluteColorimetric" | "Perceptual" | "Saturation" | "relative-colorimetric" | "absolute-colorimetric" | "perceptual" | "saturation";
export type RenderContractColorManagementPolicy = "PortableQcms" | "NativeLittleCms" | "DeterministicFallback" | "portable-qcms" | "native-little-cms" | "deterministic-fallback";
export type RenderContractExactnessPolicy = "Compatibility" | "HighQualityExact" | "compatibility" | "high-quality-exact";
/** "BestEffortResearch"/"best-effort-research" are deprecated aliases of "BestEffort". */
export type RenderContractDeterminismPolicy = "Required" | "BestEffort" | "required" | "best-effort" | "BestEffortResearch" | "best-effort-research";
export type RenderContractThinLineMode = "None" | "Solid" | "Shape" | "none" | "solid" | "shape";
export type RenderContractVectorAntialiasGamma = "Linear" | "Gamma15" | "linear" | "gamma15";
/** Packed straight-alpha colour, 0xRRGGBBAA. */
export type RenderContractPackedRgba = number;

export class WellfriendRenderContract {
  static fromJson(json: string): WellfriendRenderContract;
  toJson(): ReportJson;
  surfaceByteLength(): number;
  withSurface(width: number, height: number, pixelFormat?: RenderContractPixelFormat, alphaMode?: RenderContractAlphaMode, stride?: number, grayscale?: boolean, reverseByteOrder?: boolean): WellfriendRenderContract;
  withClip(x: number, y: number, width: number, height: number): WellfriendRenderContract;
  withoutClip(): WellfriendRenderContract;
  withDeviceTransform(a: number, b: number, c: number, d: number, e: number, f: number): WellfriendRenderContract;
  withBackground(r: number, g: number, b: number, a?: number): WellfriendRenderContract;
  withPageBox(pageBox: RenderContractPageBox): WellfriendRenderContract;
  withExecutionMode(executionMode: RenderContractExecutionMode): WellfriendRenderContract;
  withBackend(backend: RenderContractBackendSelection): WellfriendRenderContract;
  withCompositing(compositing: RenderContractCompositingPolicy): WellfriendRenderContract;
  withAnnotations(annotations: RenderContractIncludePolicy): WellfriendRenderContract;
  withForms(forms: RenderContractIncludePolicy): WellfriendRenderContract;
  withOptionalContent(optionalContent: string): WellfriendRenderContract;
  withSmoothing(smoothing: RenderContractSmoothingPolicy): WellfriendRenderContract;
  withTextSmoothing(textSmoothing: RenderContractSmoothingPolicy): WellfriendRenderContract;
  withImageSmoothing(imageSmoothing: RenderContractSmoothingPolicy): WellfriendRenderContract;
  withPathSmoothing(pathSmoothing: RenderContractSmoothingPolicy): WellfriendRenderContract;
  withSubpixelText(subpixelText: RenderContractSmoothingPolicy): WellfriendRenderContract;
  withColorScheme(colorScheme: RenderContractColorScheme): WellfriendRenderContract;
  withPrintProfile(printProfile: RenderContractPrintProfile): WellfriendRenderContract;
  withHalftone(halftone: RenderContractHalftonePolicy): WellfriendRenderContract;
  withOverprint(overprint: RenderContractOverprintPolicy): WellfriendRenderContract;
  withRenderingIntent(renderingIntent: RenderContractRenderingIntent): WellfriendRenderContract;
  withColorManagement(colorManagement: RenderContractColorManagementPolicy): WellfriendRenderContract;
  withExactness(exactness: RenderContractExactnessPolicy): WellfriendRenderContract;
  withDeterminism(determinism: RenderContractDeterminismPolicy): WellfriendRenderContract;
  withResourceBudget(maxPixels?: RenderContractBudgetValue, maxDecodedBytes?: RenderContractBudgetValue, maxTemporaryBytes?: RenderContractBudgetValue, maxCacheBytes?: RenderContractBudgetValue): WellfriendRenderContract;
  withThinLineMode(thinLineMode: RenderContractThinLineMode): WellfriendRenderContract;
  withVectorAntialiasGamma(vectorAntialiasGamma: RenderContractVectorAntialiasGamma): WellfriendRenderContract;
  withForcedColors(pathFill: RenderContractPackedRgba, pathStroke: RenderContractPackedRgba, textFill: RenderContractPackedRgba, textStroke: RenderContractPackedRgba, convertFillToStroke?: boolean): WellfriendRenderContract;
  withoutForcedColors(): WellfriendRenderContract;
  withFormHighlight(highlight: RenderContractPackedRgba, focusField?: string): WellfriendRenderContract;
  /** FormHighlightPolicy JSON: text/push_button/check_box/radio_button/combo_box/list_box/signature colours and focus_field. */
  withFormHighlightJson(policyJson: string): WellfriendRenderContract;
  withoutFormHighlight(): WellfriendRenderContract;
  private constructor();
  free(): void;
  /**
   * Render profile: default, pdfium (render like PDFium) or
   * pdfium-test (pdfium plus pdfium_test's form highlight). Sets
   * every field the profile bundles.
   */
  withProfile(profile: string): WellfriendRenderContract;
  /**
   * Filled-rectangle pixel snapping: default or pdfium (PDFium's
   * whole-pixel rectangle fast path; abutting fills show no seams).
   */
  withRectSnapping(rect_snapping: string): WellfriendRenderContract;
  /**
   * Stroke adjustment: default or off (never snap strokes to whole
   * pixels, like PDFium).
   */
  withStrokeAdjustment(stroke_adjustment: string): WellfriendRenderContract;
}

export class ProgressiveRenderJob {
  stateJson(): ReportJson;
  step(maxTiles: number): ReportJson;
  stepWithCancellation(maxTiles: number, cancellation: boolean | AbortSignal): ReportJson;
  tokenJson(): ReportJson;
  pauseJson(): ReportJson;
  resumeJson(tokenJson: string): void;
  cancel(): void;
  requestCancel(): void;
  reviseViewportHintJson(hintPresent: boolean, x: number, y: number, width: number, height: number): ReportJson;
  reviseDirtyRegionJson(dirtyPresent: boolean, x: number, y: number, width: number, height: number): ReportJson;
  reviseRenderContextJson(renderContractFingerprint?: string, visibilityFingerprint?: string): ReportJson;
  reviseRenderContractJson(contractJson: string): ReportJson;
  applyRenderInvalidationPlanJson(planJson: string): ReportJson;
  evaluateTilePublicationJson(publicationJson: string): ReportJson;
  viewerQueueJson(): ReportJson;
  executeViewerQueueJson(maxItems: number): ReportJson;
  executeViewerQueueJsonWithCancellation(maxItems: number, cancellation: RenderCancellation): ReportJson;
  executeAdjacentPagePrefetch(prefetchIdentity: string, maxTiles: number): AdjacentPagePrefetchExecution;
  executeAdjacentPagePrefetchWithCancellation(prefetchIdentity: string, maxTiles: number, cancellation: RenderCancellation): AdjacentPagePrefetchExecution;
  viewerCallbackDispatchJson(): ReportJson;
  dispatchViewerCallbacks(callback: (eventJson: string) => unknown): ReportJson;
  finishPng(): Uint8Array;
  finishPngWithCancellation(cancellation: boolean | AbortSignal): Uint8Array;
  close(): void;
  private constructor();
  free(): void;
}

export class RenderCancellation {
  constructor();
  cancel(): void;
  isCancelled(): boolean;
  free(): void;
}

export class RenderCache {
  constructor();
  clear(): void;
  applyRenderInvalidationPlanJson(planJson: string): ReportJson;
  free(): void;
}

export class AdjacentPagePrefetchExecution {
  reportJson(): ReportJson;
  hasJob(): boolean;
  takeJob(): ProgressiveRenderJob | undefined;
  private constructor();
  free(): void;
}

export class WellfriendPdf {
  static universalEditingCapabilitiesV2Json(): ReportJson;
  static universalEditingApprovalV2Json(planJson: string, decisionJson: string): ReportJson;
  /** Authenticate a canonical publication receipt with a caller-held key.
   * Never place a server-held key in browser-delivered code. All timestamps
   * must be non-negative JavaScript safe integers.
   */
  static authenticateTextRangePaintPartitionReceipt(
    publicationReceiptJson: string,
    keyId: string,
    audience: string,
    issuedAtUnix: number,
    expiresAtUnix: number,
    hmacKey: Uint8Array,
  ): ReportJson;
  /** Verify a host-authenticated receipt and return the nested content receipt. */
  static verifyAuthenticatedTextRangePaintPartitionReceipt(
    authenticatedReceiptJson: string,
    expectedKeyId: string,
    expectedAudience: string,
    nowUnix: number,
    allowedFutureSkewSecs: number,
    hmacKey: Uint8Array,
  ): ReportJson;
  /** Set include_scoped_text_sources for source-local Form/AP inventories. */
  universalEditingAnalyzeV2Json(optionsJson?: string): ReportJson;
  universalEditingPlanV2Json(requestJson: string): ReportJson;
  /** Preview a revision-bound transfer of one saved native Figure between saved stories. */
  storyFigureTransferPreviewJson(requestJson: string): ReportJson;
  /** Apply the exact planSha256 returned by storyFigureTransferPreviewJson. */
  storyFigureTransferApply(requestJson: string, approvedPlanSha256: string): WellfriendOutput;
  /** Bounded before/candidate PNG byte arrays, without publishing candidate PDF bytes. */
  universalEditingScopedPreviewV2Json(planJson: string, optionsJson?: string): ReportJson;
  universalEditingApplyV2(planJson: string, approvalJson?: string): WellfriendOutput;
  universalEditingApplyV2WithOutputCredentials(
    planJson: string,
    approvalJson: string | undefined,
    inputPassword: Uint8Array | undefined,
    outputUserPassword: Uint8Array,
    outputOwnerPassword?: Uint8Array,
  ): WellfriendOutput;
  /** Materialize canonical candidates from one immutable revision and publish
   * only the evidence-qualified ECBES selection (or the exact input bytes).
   */
  ecbesUniversalEdit(requestJson: string): WellfriendOutput;
  /** ECBES variant for Standard-security output candidates. Credentials are
   * apply-only and are excluded from the JSON report.
   */
  ecbesUniversalEditWithOutputCredentials(
    requestJson: string,
    inputPassword: Uint8Array | undefined,
    outputUserPassword: Uint8Array,
    outputOwnerPassword?: Uint8Array,
  ): WellfriendOutput;
  /** Inspect the page-local logical/source range model used by native text edits. */
  advanced_editing_closeoutTextRangeAnalyzeJson(page: number): ReportJson;
  /** Build a non-mutating, exact-revision paint-slot partition proposal. */
  proposeTextRangePaintPartitions(requestJson: string): ReportJson;
  /** Render bounded same-engine before/candidate PNGs without returning the candidate PDF. */
  previewTextRangePaintPartitions(
    requestJson: string,
    proposalJson: string,
    approvalJson: string,
    fontBytes?: Uint8Array,
    optionsJson?: string,
  ): ReportJson;
  /** Apply a direct multi-run text edit without a separate partition proposal. */
  editTextRange(requestJson: string): WellfriendOutput;
  /** Apply reviewed regions/final lines to the exact revision-bound proposal. */
  applyTextRangePaintPartitions(
    requestJson: string,
    proposalJson: string,
    approvalJson: string,
    fontBytes?: Uint8Array,
  ): WellfriendOutput;
  /** Apply only the candidate bound by the canonical preview publication receipt. */
  applyReviewedTextRangePaintPartitions(
    requestJson: string,
    proposalJson: string,
    approvalJson: string,
    publicationReceiptJson: string,
    fontBytes?: Uint8Array,
  ): WellfriendOutput;
  /** Reopen-validated exact typed-cell owners, ranges and content rectangles. */
  authoredTypedTableSourcesJson(): ReportJson;
  /** Revision-bound typed value/formula mutation. Optional bytes are the
   * approved shaping font used when the retained source font cannot cover the
   * replacement. The current document object is not mutated.
   */
  mutateAuthoredTypedTable(requestJson: string, fontBytes?: Uint8Array): WellfriendOutput;
  /** Direct text operands per page Form occurrence, not automatic reading order.
   * Geometry and edit regions use Form-local coordinates, before its Matrix.
   */
  formTextSourcesJson(page: number): ReportJson;
  /** JSON: {target: FormTextTarget, edit: MultiRunTextRangeRequest,
   * shared_form_policy: "clone_edit_one_instance" | "edit_all_uses"}.
   * Optional font bytes supply the shaping font; output does not mutate this
   * document/session. Run in a worker and reopen accepted output explicitly.
   */
  editFormText(requestJson: string, fontBytes?: Uint8Array): WellfriendOutput;
  /** Discover source-local direct text in existing annotation appearance/Form occurrences. */
  appearanceTextSourcesJson(page: number): ReportJson;
  /** Native selected-occurrence copy-on-write. requestJson contains target, edit
   * (MultiRunTextRangeRequest), and an explicit metadata_policy. Returns new PDF
   * bytes and a saved-revision target; does not mutate this document/session.
   * Widget field synchronization and tagged stream cloning are not implemented. */
  editAppearanceText(requestJson: string, fontBytes?: Uint8Array): WellfriendOutput;
  constructor(bytes: Uint8Array | ArrayBuffer | ArrayLike<number>);
  static openWithPassword(bytes: Uint8Array | ArrayBuffer | ArrayLike<number>, password: Uint8Array | ArrayLike<number>): WellfriendPdf;
  static sdkVersion(): string;
  static abiVersion(): number;
  static featureReportJson(): ReportJson;
  static tableProposalStatusJson(): ReportJson;
  static decodeBudgetReportJson(filter: string, width: number, height: number, components: number): ReportJson;
  static codecIsolationReportJson(filter: string, data: Uint8Array | ArrayBuffer | ArrayLike<number>, policy?: string): ReportJson;

  registerFontBytes(name: string, fontBytes: Uint8Array | ArrayBuffer | ArrayLike<number>): void;
  close(): void;
  isClosed(): boolean;
  pageCount(): number;
  extractText(page: number): string;
  extractStructuredText(page: number): ReportJson;
  extractSemanticJson(): ReportJson;
  parseMarkdown(): string;
  parseJson(): ReportJson;
  chunk(targetTokens: number, overlap: number): ReportJson;
  extractFieldsJson(docType: string): ReportJson;
  imageDecodeCapabilityReportJson(): ReportJson;
  backendPlanArenaReportJson(page: number, dpi: number, mode?: string): ReportJson;
  backendPlanArenaReportForContractJson(contractJson: string): ReportJson;
  prepressPlateReportJson(page: number, dpi: number): ReportJson;
  progressiveImageDecodeLifecycleReportJson(requestJson: string): ReportJson;
  infoJson(): ReportJson;
  renderPagePng(page: number, dpi: number): Uint8Array;
  renderPagePngWithFontSubstitutionReport(page: number, dpi: number, mode?: string): WellfriendOutput;
  defaultRenderContractJson(page: number, dpi: number, mode?: string): ReportJson;
  defaultRenderContract(page: number, dpi: number, mode?: string): WellfriendRenderContract;
  renderContractPng(contractJson: string): Uint8Array;
  renderContractPngWithRenderCache(contractJson: string, cache: RenderCache): Uint8Array;
  renderContractPngWithCancellation(contractJson: string, cancellation: RenderCancellation): Uint8Array;
  renderContractObjectPng(contract: WellfriendRenderContract): Uint8Array;
  renderContractObjectPngWithRenderCache(contract: WellfriendRenderContract, cache: RenderCache): Uint8Array;
  renderContractObjectPngWithCancellation(contract: WellfriendRenderContract, cancellation: RenderCancellation): Uint8Array;
  renderContractPngWithFontSubstitutionReport(contractJson: string): WellfriendOutput;
  renderContractPngWithFontSubstitutionReportWithCancellation(contractJson: string, cancellation: RenderCancellation): WellfriendOutput;
  renderContractPngWithRenderReport(contractJson: string): WellfriendOutput;
  renderContractPngWithRenderCacheReport(contractJson: string, cache: RenderCache): WellfriendOutput;
  renderContractPngWithRenderReportWithCancellation(contractJson: string, cancellation: RenderCancellation): WellfriendOutput;
  renderContractObjectPngWithFontSubstitutionReport(contract: WellfriendRenderContract): WellfriendOutput;
  renderContractObjectPngWithFontSubstitutionReportWithCancellation(contract: WellfriendRenderContract, cancellation: RenderCancellation): WellfriendOutput;
  renderContractObjectPngWithRenderReport(contract: WellfriendRenderContract): WellfriendOutput;
  renderContractObjectPngWithRenderCacheReport(contract: WellfriendRenderContract, cache: RenderCache): WellfriendOutput;
  renderContractObjectPngWithRenderReportWithCancellation(contract: WellfriendRenderContract, cancellation: RenderCancellation): WellfriendOutput;
  renderContractInto(contractJson: string, output: Uint8Array): void;
  renderContractIntoWithCancellation(contractJson: string, output: Uint8Array, cancellation: RenderCancellation): void;
  renderContractObjectInto(contract: WellfriendRenderContract, output: Uint8Array): void;
  renderContractObjectIntoWithCancellation(contract: WellfriendRenderContract, output: Uint8Array, cancellation: RenderCancellation): void;
  renderContractIntoWithFontSubstitutionReport(contractJson: string, output: Uint8Array): ReportJson;
  renderContractIntoWithFontSubstitutionReportWithCancellation(contractJson: string, output: Uint8Array, cancellation: RenderCancellation): ReportJson;
  renderContractIntoWithRenderReport(contractJson: string, output: Uint8Array): ReportJson;
  renderContractIntoWithRenderReportWithCancellation(contractJson: string, output: Uint8Array, cancellation: RenderCancellation): ReportJson;
  renderContractObjectIntoWithFontSubstitutionReport(contract: WellfriendRenderContract, output: Uint8Array): ReportJson;
  renderContractObjectIntoWithFontSubstitutionReportWithCancellation(contract: WellfriendRenderContract, output: Uint8Array, cancellation: RenderCancellation): ReportJson;
  renderContractObjectIntoWithRenderReport(contract: WellfriendRenderContract, output: Uint8Array): ReportJson;
  renderContractObjectIntoWithRenderReportWithCancellation(contract: WellfriendRenderContract, output: Uint8Array, cancellation: RenderCancellation): ReportJson;
  progressiveRenderJob(page: number, dpi: number, tileWidth: number, tileHeight: number, mode?: string): ProgressiveRenderJob;
  progressiveRenderJobWithContractJson(contractJson: string, tileWidth: number, tileHeight: number): ProgressiveRenderJob;

  documentInfoJson(): ReportJson;
  documentViewsReportJson(): ReportJson;
  securityReportJson(): ReportJson;
  riskyContentReportJson(): ReportJson;
  parserReportJson(mode?: string): ReportJson;
  colorReportJson(profile?: string): ReportJson;
  validateJson(profile?: string): ReportJson;
  validatePdfaJson(profile?: string): ReportJson;
  validatePdfuaJson(): ReportJson;
  formsReportJson(): ReportJson;
  xfaReportJson(): ReportJson;
  xfaExtractJson(): ReportJson;
  xfaScriptReportJson(): ReportJson;
  xfaSecurityReportJson(): ReportJson;
  xfaRuntimeReportJson(scriptPolicy?: string, executeEvents?: boolean): ReportJson;
  annotationsReportJson(): ReportJson;
  pagesReportJson(): ReportJson;
  interactiveReportJson(): ReportJson;
  signatureReportJson(): ReportJson;
  fontReportJson(): ReportJson;
  textSemanticJson(): ReportJson;
  semanticDocumentReportJson(): ReportJson;
  chunksJson(): ReportJson;
  advancedChunksJson(): ReportJson;
  semanticBundleJson(): ReportJson;
  semanticSearchJson(query: string): ReportJson;
  /** Options JSON: pages, case_sensitive, normalize_unicode, ignore_accents, whole_word, include_hidden, max_matches, ... */
  searchTextJson(query: string, optionsJson?: string): ReportJson;
  /** RGBA8 thumbnail; reportJson() has width/height/channels. undefined when the page has no /Thumb. */
  pageThumbnailRgba(page: number): WellfriendOutput | undefined;
  decodePageThumbnail(page: number): WellfriendOutput | undefined;
  pageThumbnailPng(page: number): Uint8Array | undefined;
  pageImagesJson(page: number): ReportJson;
  /** pdfimages -all style original bytes; reportJson() has extension, filter and sidecars. */
  extractImageOriginal(page: number, index: number): WellfriendOutput;
  extractImageOriginalSidecar(page: number, index: number, sidecarIndex: number): Uint8Array;
  renderImageObjectPng(page: number, index?: number, name?: string): Uint8Array;
  renderImageObjectRgba(page: number, index?: number, name?: string): WellfriendOutput;
  editing_transactionsReportJson(): ReportJson;
  editing_transactionsSceneReportJson(pagesJson?: string): ReportJson;
  editing_transactionsSceneSelectJson(requestJson: string): ReportJson;
  editing_transactionsTransactionPlanJson(requestJson: string): ReportJson;
  editing_transactionsTransactionApply(requestJson: string): WellfriendOutput;
  editing_transactionsTransactionApplyWithRenderInvalidation(requestJson: string, renderInvalidationOptionsJson?: string): WellfriendOutput;
  editing_transactionsSceneEditText(requestJson: string): WellfriendOutput;

  xfaRender(scriptPolicy?: string, executeEvents?: boolean, dpi?: number): WellfriendOutput;
  xfaFlatten(mode?: string): WellfriendOutput;
  xfaSanitize(mode?: string): WellfriendOutput;
  sanitize(policy?: string): WellfriendOutput;
  canonicalize(dateEpoch?: bigint | number): WellfriendOutput;
  redactTermsJson(termsJson: string, strict: boolean): WellfriendOutput;
  free(): void;
  advanced_editingInkFit(page: number, annotation_index: number, options_json: string | null | undefined, signature_policy_override: boolean): WellfriendOutput;
  advanced_editingReportJson(): string;
  advanced_editingTextEdit(page: number, old_text: string, new_text: string, mode: string, options_json?: string | null): WellfriendOutput;
  advanced_editingVectorEdit(page: number, stable_id: string, operation_json: string, options_json?: string | null): WellfriendOutput;
  advanced_editingVectorListJson(page: number): string;
  advanced_editing_closeoutReportJson(): string;
  aesGcmReportJson(): string;
  annotationAppearanceGenerate(options_json?: string | null): WellfriendOutput;
  annotationAppearanceReportJson(options_json?: string | null): string;
  annotationXfdfExport(): WellfriendOutput;
  annotationXfdfImport(xfdf: Uint8Array, options_json?: string | null): WellfriendOutput;
  annotation_media_redactionReportJson(): string;
  associatedFileAdd(payload: Uint8Array, options_json: string): WellfriendOutput;
  associatedFileRemoveOwner(options_json: string): WellfriendOutput;
  associatedFileUpdateOwner(payload: Uint8Array, options_json: string): WellfriendOutput;
  associatedFilesRemove(stable_ids_json: string): WellfriendOutput;
  associatedFilesReportJson(): string;
  associatedFilesSanitize(options_json?: string | null): WellfriendOutput;
  static compression_officeOfficeInspectJson(bytes: Uint8Array, format: string): string;
  static compression_officeOfficeToPdf(bytes: Uint8Array, format: string): WellfriendOutput;
  compression_officeOptimize(options_json?: string | null): WellfriendOutput;
  compression_officeReportJson(): string;
  static cryptoTamperTestJson(): string;
  crypto_writerReportJson(): string;
  document_securityAnalyzeJson(): string;
  document_securityApply(request_json: string): WellfriendOutput;
  document_securityPlanJson(request_json: string): string;
  document_securityReportJson(): string;
  document_securityUndo(output_pdf: Uint8Array, request_json: string): WellfriendOutput;
  document_securityVerifyResidualJson(terms_json: string): string;
  document_subsystemsAnalyzeJson(): string;
  document_subsystemsApply(request_json: string): WellfriendOutput;
  document_subsystemsPlanJson(request_json: string): string;
  document_subsystemsReportJson(): string;
  document_subsystemsUndo(output_pdf: Uint8Array, request_json: string): WellfriendOutput;
  editPolicyReportJson(operation: string): string;
  editing_transactionsFontSubsetPlanJson(text: string, direction?: string | null, policy?: string | null): string;
  editing_transactionsFontSubstitutionReportJson(requested_family: string, text: string, policy?: string | null): string;
  editing_transactionsShapeTextJson(text: string, direction?: string | null): string;
  editing_transactionsTextMapJson(text: string, direction?: string | null): string;
  /**
   * Text in reading order (columns, blocks and lines in the order a
   * reader follows them), the `wellfriendpdf extract-text` default;
   * `extractText` returns the faster content order. `page` is 1-based;
   * 0 returns every page, each followed by a newline, as the CLI
   * prints the document.
   */
  extractTextReadingOrder(page: number): string;
  formActionGraphJson(): string;
  formJsFlattenValues(options_json?: string | null): WellfriendOutput;
  formJsReportJson(): string;
  formJsSanitize(options_json?: string | null): WellfriendOutput;
  form_action_policyReportJson(): string;
  incrementalAnnotationEdit(options_json: string, signature_policy_override: boolean): WellfriendOutput;
  incrementalFormEdit(field_name: string, value: string, signature_policy_override: boolean): WellfriendOutput;
  incrementalPagePropertyEdit(options_json: string, signature_policy_override: boolean): WellfriendOutput;
  interactiveDataReportJson(): string;
  nonaxisRedactionPlanJson(options_json: string): string;
  static ocrProviderMatrixJson(): string;
  pubsecReportJson(): string;
  redactImageMask(options_json: string): WellfriendOutput;
  redactImageNonaxis(options_json: string): WellfriendOutput;
  redactInlineImage(options_json: string): WellfriendOutput;
  richMediaFlattenPoster(): WellfriendOutput;
  richMediaReportJson(): string;
  richMediaSanitize(mode?: string | null, custom_json?: string | null): WellfriendOutput;
  static runtimeCapabilitiesJson(config_json?: string | null): string;
  static runtimeConfigJson(config_json?: string | null): string;
  secure_mutationReportJson(): string;
  secure_mutation_closeoutReportJson(): string;
  /**
   * Incremental Signing Standards append-only incremental signing (in-memory). Produces a
   * signed PDF whose original bytes are preserved as a prefix, reopened
   * and validated before it is returned. `key_pem`/`cert_pem` are the
   * caller-supplied in-memory signer material (never logged/persisted).
   */
  signPdf(key_pem: string, cert_pem: string, placeholder_size: number, certify: number, field_name?: string | null, reason?: string | null): WellfriendOutput;
  /**
   * Incremental Signing Standards append-only incremental signing plan (in-memory). `certify`
   * in 1..=3 plans a certification (DocMDP) signature; else approval.
   */
  signPlanJson(key_pem: string, cert_pem: string, placeholder_size: number, certify: number): string;
  signaturePreservingFormEdit(field_name: string, value: string, options_json: string | null | undefined, explicit_invalidation_override: boolean): WellfriendOutput;
  signaturePreservingFormPlanJson(field_name: string, value: string, options_json?: string | null): string;
  signatureReportWithOptionsJson(options_json: string): string;
  /**
   * Offline Signature Validation validation with owned caller-supplied trust and
   * evidence.  WASM never performs implicit AIA, OCSP, or CRL retrieval.
   */
  signatureValidation(options: SignatureValidationOptions): string;
  /**
   * Offline Signature Validation validation plus a portable, hash-checked evidence
   * bundle that can be replayed by a later WASM or native invocation.
   */
  signatureValidationWithEvidence(options: SignatureValidationOptions): string;
  signatureValidationWithEvidenceJson(options_json: string): string;
  /**
   * Exact WASM signing capability posture. In-memory local signing with
   * caller-provided PEM key material is supported (pure compute); host
   * filesystem key loading, network TSA acquisition, and JS external
   * signer callbacks are reported unsupported rather than faked.
   */
  static signingCapabilities(): string;
  source_editingEditEligibilityJson(request_json: string): string;
  source_editingImageEligibilityJson(page: number, occurrence?: string | null): string;
  source_editingOperatorTextEdit(request_json: string): WellfriendOutput;
  source_editingPathEdit(page: number, stable_id: string, operation_json: string, options_json?: string | null): WellfriendOutput;
  source_editingPathProvenanceJson(page: number): string;
  source_editingProvenanceJson(page: number, source_text: string, replacement_text: string): string;
  source_editingReportJson(): string;
  text_reflowConfidenceReportJson(request_json: string): string;
  text_reflowConstraintsReportJson(request_json: string): string;
  text_reflowFlowGraphReportJson(): string;
  text_reflowLayoutAnalyzeJson(request_json: string): string;
  text_reflowOverflowReportJson(request_json: string): string;
  text_reflowReadingOrderReportJson(): string;
  text_reflowReflowDocument(request_json: string): WellfriendOutput;
  text_reflowReflowOperationReportJson(request_json: string): string;
  text_reflowReflowPreviewJson(request_json: string): string;
  text_reflowReflowRegion(request_json: string): WellfriendOutput;
  text_reflowReportJson(): string;
  text_reflowSemanticLayoutJson(): string;
  text_reflowUndoReflow(output_pdf: Uint8Array, request_json: string): WellfriendOutput;
  text_reflowValidateReflowOutputJson(output_pdf: Uint8Array, request_json: string): string;
  static timestampTokenValidationJson(token: Uint8Array, signature_value: Uint8Array, options_json?: string | null): string;
  universalEditingInspectObjectV2Json(number: number, generation: number): string;
  universalRenderQualificationV2Json(options_json?: string | null): string;
  /**
   * Incremental Signing Standards clause-mapped PDF/A validation. `target` e.g. "PDF/A-2B".
   */
  validatePdfaStandardsJson(target?: string | null): string;
  /**
   * Incremental Signing Standards clause-mapped PDF/UA validation. `target` e.g. "PDF/UA-1".
   */
  validatePdfuaStandardsJson(target?: string | null): string;
  /**
   * Incremental Signing Standards clause-mapped PDF/X validation. `target` e.g. "PDF/X-4".
   */
  validatePdfxStandardsJson(target?: string | null): string;
  /**
   * Incremental Signing Standards combined PDF/A + PDF/UA + PDF/X validation with
   * cross-profile conflicts.
   */
  validateStandardsAllJson(target?: string | null): string;
  wordPaginationAuditJson(layout: string): string;
  writerCloseoutReportJson(): string;
  writerDeterminismAuditJson(): string;
  writerExternalDiffJson(): string;
  writer_historyFontReconstructionReportJson(): string;
  static writer_historyHistoryReportJson(): string;
  writer_historyObjectStreamReportJson(): string;
  writer_historyPackObjectStreams(): WellfriendOutput;
  writer_historyRasterVectorReportJson(page: number, options_json?: string | null): string;
  writer_historyReportJson(): string;
}

export class TextEditor {
  free(): void;
  /**
   * The current document bytes.
   */
  bytes(): Uint8Array;
  canRedo(): boolean;
  canUndo(): boolean;
  /**
   * Caret JSON in pixels: `{offset, line, top: [x, y], bottom: [x, y]}`.
   */
  caretJson(page: number, offset: number, dpi: number): string;
  delete(page: number, start: number, end: number): string;
  /**
   * Page-to-pixel transform `[a, b, c, d, e, f]` of a render at `dpi`.
   */
  deviceTransform(page: number, dpi: number): Float64Array;
  /**
   * Caret offset for a click at pixel (`x`, `y`), or -1 on a page
   * without text.
   */
  hitTest(page: number, x: number, y: number, dpi: number): bigint;
  insert(page: number, offset: number, text: string): string;
  /**
   * Page layout JSON (PDF user space): `text`, `glyphs` with quads,
   * `lines`, `paragraphs`, `page_box`, `rotation`.
   */
  layoutJson(page: number): string;
  /**
   * Caret offset after a keyboard movement (`left`, `word_right`,
   * `line_up`, `line_end`, ...).
   */
  moveCaret(page: number, offset: number, movement: string): number;
  constructor(bytes: Uint8Array);
  /**
   * `[start, end]` of the word, line or paragraph (`unit`) around `offset`.
   */
  range(page: number, offset: number, unit: string): Uint32Array;
  redo(): boolean;
  /**
   * Replace `[start, end)` with `text`; returns the outcome JSON
   * (`caret`, `dirty_region` in user space, transaction `report`).
   */
  replace(page: number, start: number, end: number, text: string): string;
  /**
   * Selection highlight quads in pixels, one per line: `[[[x, y] x4], ...]`.
   */
  selectionJson(page: number, start: number, end: number, dpi: number): string;
  /**
   * The text of `[start, end)`.
   */
  textIn(page: number, start: number, end: number): string;
  undo(): boolean;
}

/**
 * Owned offline Signature Validation validation options for the WASM surface.
 *
 * WASM accepts caller-supplied trust anchors, intermediates, and
 * revocation evidence, but has no native network transport.  Enabling a
 * retrieval policy returns an exact unsupported error instead of allowing
 * implicit browser networking or relying on ambient platform trust.
 */
export class SignatureValidationOptions {
  free(): void;
  addCrlDer(der: Uint8Array): void;
  addDistrustedCertificateSha256(fingerprint: string): void;
  addIntermediateDer(der: Uint8Array): void;
  addOcspResponseDer(der: Uint8Array): void;
  addTrustAnchorDer(der: Uint8Array): void;
  applyEvidenceStore(store: SignatureEvidenceStore): void;
  applyIntermediateStore(store: SignatureIntermediateStore): void;
  applyRetrievalPolicy(policy: SignatureRetrievalPolicy): void;
  applyTrustStore(store: SignatureTrustStore): void;
  constructor();
  static onlineRetrievalCapability(): string;
  setAlgorithmPolicyJson(policy_json: string): void;
  setCancellation(cancellation: SignatureValidationCancellation): void;
  setEvidenceBundleJson(bundle_json: string): void;
  setPathLimits(max_chain_depth: number, max_path_candidates: number): void;
  setRetrievalPolicyJson(policy_json: string): void;
  setRevocationMode(mode: string): void;
  setValidationTimeUnix(unix: number): void;
  useSystemValidationTime(): void;
}

export class SignatureValidationCancellation {
  free(): void;
  cancel(): void;
  isCancelled(): boolean;
  constructor();
}

export class SignatureTrustStore {
  free(): void;
  addAnchorDer(der: Uint8Array, origin?: string | null, purpose?: string | null): void;
  addDistrustedCertificateSha256(fingerprint: string): void;
  anchorCount(): number;
  constructor();
}

export class SignatureRetrievalPolicy {
  free(): void;
  constructor();
  setJson(policy_json: string): void;
  toJson(): string;
}

export class SignatureIntermediateStore {
  free(): void;
  addDer(der: Uint8Array): void;
  certificateCount(): number;
  constructor();
}

export class SignatureEvidenceStore {
  free(): void;
  addCrlDer(der: Uint8Array): void;
  addOcspResponseDer(der: Uint8Array): void;
  bundleJson(): string | undefined;
  crlCount(): number;
  importBundleJson(bundle_json: string): void;
  constructor();
  ocspCount(): number;
}
