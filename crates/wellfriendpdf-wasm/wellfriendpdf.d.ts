export default function init(input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module): Promise<unknown>;

export type ReportJson = string;

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
