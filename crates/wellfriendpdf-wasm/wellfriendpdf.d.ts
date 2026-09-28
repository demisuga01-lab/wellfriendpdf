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

export class WellfriendOutput {
  bytes(): Uint8Array;
  byteLength(): number;
  reportJson(): ReportJson;
}

export type RenderContractPixelFormat = "Rgba8" | "Bgra8" | "Rgb8" | "Bgr8" | "Gray8" | "rgba8" | "bgra8" | "rgb8" | "bgr8" | "gray8" | "gray" | "grey8" | "grey";
export type RenderContractAlphaMode = "Premultiplied" | "Straight" | "Opaque" | "premultiplied" | "straight" | "opaque";
export type RenderContractBudgetValue = bigint | number;
export type RenderContractPageBox = "Media" | "Crop" | "Bleed" | "Trim" | "Art" | "media" | "crop" | "bleed" | "trim" | "art";
export type RenderContractExecutionMode = "Standard" | "Research" | "standard" | "research";
export type RenderContractBackendSelection = "ScalarReference" | "StandardCpu" | "ResearchHybrid" | "scalar-reference" | "standard-cpu" | "research-hybrid";
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
export type RenderContractDeterminismPolicy = "Required" | "BestEffortResearch" | "required" | "best-effort-research";

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
}

export class RenderCancellation {
  constructor();
  cancel(): void;
  isCancelled(): boolean;
}

export class RenderCache {
  constructor();
  clear(): void;
  applyRenderInvalidationPlanJson(planJson: string): ReportJson;
}

export class AdjacentPagePrefetchExecution {
  reportJson(): ReportJson;
  hasJob(): boolean;
  takeJob(): ProgressiveRenderJob | undefined;
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
}
