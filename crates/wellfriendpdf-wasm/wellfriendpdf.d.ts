export default function init(input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module): Promise<unknown>;

export type ReportJson = string;

export class WellfriendOutput {
  bytes(): Uint8Array;
  byteLength(): number;
  reportJson(): ReportJson;
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
  infoJson(): ReportJson;
  renderPagePng(page: number, dpi: number): Uint8Array;

  documentInfoJson(): ReportJson;
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

  xfaRender(scriptPolicy?: string, executeEvents?: boolean, dpi?: number): WellfriendOutput;
  xfaFlatten(mode?: string): WellfriendOutput;
  xfaSanitize(mode?: string): WellfriendOutput;
  sanitize(policy?: string): WellfriendOutput;
  canonicalize(dateEpoch?: bigint | number): WellfriendOutput;
  redactTermsJson(termsJson: string, strict: boolean): WellfriendOutput;
}
