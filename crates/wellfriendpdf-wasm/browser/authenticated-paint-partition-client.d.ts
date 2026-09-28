export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type JsonInput = string | { [key: string]: unknown };
export type AdvancedEditingSupportStatus = "implemented" | "implemented_with_limits" | "unsupported_reported_exact" | "unsupported_reported_security_policy" | "not_in_advanced_editing_scope" | "blocked";
export type TextRangeMode = "safe_patch" | "paragraph_reflow_horizontal" | "paragraph_reflow_rtl" | "paragraph_reflow_vertical" | "overlay_fallback" | "unsupported";
export type TextRangeStylePolicy = "inherit_leading" | "inherit_trailing" | "preserve_per_segment" | "explicit_supplied";
export type TextOverflowPolicy = "error" | "clip" | "expand_region";
export type GeneratedTextAlignment = "left" | "right" | "center" | "start" | "end" | "justify";
export type GeneratedPaintOrderPolicy = "require_single_source_text_object" | "anchor_after_first_source_text_object" | "anchor_after_last_source_text_object";

export interface ExplicitLayoutLine {
  logical_text: string;
  visual_text: string;
  bidi?: { levels: number[]; rtl: boolean; context?: { [key: string]: unknown } };
  inserted_visual_hyphen?: boolean;
}

export interface GeneratedPaintPartition {
  source_text_object: number;
  replacement_scalar_range: [number, number];
  region: [number, number, number, number];
  final_lines?: ExplicitLayoutLine[] | null;
}

export interface AdvancedTextEditOptions {
  region: [number, number, number, number];
  font_size: number;
  line_spacing: number;
  max_lines_or_columns: number;
  overflow_policy: TextOverflowPolicy;
  signature_policy_override: boolean;
  deterministic: boolean;
  alignment?: GeneratedTextAlignment;
  justify_last_line?: boolean;
  max_word_spacing?: number;
  max_character_spacing?: number;
  target_stream_object?: number | null;
  target_stream_generation?: number | null;
  target_decoded_byte_range?: [number, number] | null;
  paint_order_policy?: GeneratedPaintOrderPolicy;
  paint_partitions?: GeneratedPaintPartition[];
}

export interface MultiRunTextRangeRequest {
  page: number;
  logical_start: number;
  logical_end: number;
  replacement_text: string;
  mode: TextRangeMode;
  style_policy?: TextRangeStylePolicy;
  /** Omit the complete object to select engine defaults. If supplied, every
   * non-defaulted Rust field below is required so the server cannot accept a
   * shape which the type system described as valid but serde rejects. */
  options?: AdvancedTextEditOptions;
  final_lines?: ExplicitLayoutLine[] | null;
}

export interface PaintPartitionProposalCandidate {
  source_text_object: number;
  selected_source_scalar_count: number;
  replacement_scalar_range: [number, number];
  selected_span_ids: string[];
  suggested_region?: [number, number, number, number];
  start_boundary_class: string;
  end_boundary_class: string;
}

export interface PaintPartitionProposal {
  schema_version: string;
  status: AdvancedEditingSupportStatus;
  input_sha256: string;
  request_sha256: string;
  proposal_id: string;
  page: number;
  logical_range: [number, number];
  replacement_sha256: string;
  candidates: PaintPartitionProposalCandidate[];
  deterministic: boolean;
  exact_limits: string[];
}

export interface PaintPartitionApprovalEntry {
  source_text_object: number;
  region: [number, number, number, number];
  final_lines?: ExplicitLayoutLine[] | null;
}

export interface PaintPartitionApproval {
  proposal_id: string;
  font_sha256?: string;
  partitions: PaintPartitionApprovalEntry[];
}

export interface PaintPartitionPublicationReceipt {
  schema_version: string;
  proposal_id: string;
  input_sha256: string;
  request_sha256: string;
  approval_sha256: string;
  font_sha256: string | null;
  candidate_output_sha256: string;
  preview_evidence_sha256: string;
  receipt_id: string;
}

export interface AuthenticatedPaintPartitionPublicationReceipt {
  schema_version: string;
  key_id: string;
  audience: string;
  issued_at_unix: number;
  expires_at_unix: number;
  publication_receipt: PaintPartitionPublicationReceipt;
  hmac_sha256: string;
}

export interface ReportEnvelope<T> {
  schema_version: number;
  kind: string;
  report: T;
}

export interface ScopedPreviewOptions {
  pages?: number[];
  dpi?: number;
  require_exact?: boolean;
  max_total_pixels?: number;
  channel_tolerance?: number;
}

export interface AuthenticatedPaintPartitionPreviewReport {
  publication_receipt: PaintPartitionPublicationReceipt;
  authenticated_publication_receipt: AuthenticatedPaintPartitionPublicationReceipt;
  [key: string]: unknown;
}

export interface AuthenticatedPaintPartitionClientOptions {
  baseUrl: string | URL;
  apiKey?: string;
  fetchImpl?: typeof fetch;
  credentials?: RequestCredentials;
  maxDocumentBytes?: number;
  maxJsonBytes?: number;
  maxResponseBytes?: number;
}

export interface PaintPartitionRequestBase {
  pdf: Uint8Array;
  request: MultiRunTextRangeRequest | string;
  password?: Uint8Array;
  signal?: AbortSignal;
}

export interface PaintPartitionPreviewRequest extends PaintPartitionRequestBase {
  proposal: PaintPartitionProposal | ReportEnvelope<PaintPartitionProposal> | string;
  approval: PaintPartitionApproval | string;
  fontBytes?: Uint8Array;
  options?: ScopedPreviewOptions | string;
}

export interface PaintPartitionApplyRequest extends PaintPartitionRequestBase {
  proposal: PaintPartitionProposal | ReportEnvelope<PaintPartitionProposal> | string;
  approval: PaintPartitionApproval | string;
  authenticatedReceipt: AuthenticatedPaintPartitionPublicationReceipt | string;
  fontBytes?: Uint8Array;
}

export interface PaintPartitionApplyResult {
  report: ReportEnvelope<{ [key: string]: JsonValue }>;
  document: Uint8Array;
}

export declare function parsePaintPartitionApplyMultipart(
  bytes: Uint8Array,
  contentType: string | null,
  limits?: { maxResponseBytes?: number; maxJsonBytes?: number },
): PaintPartitionApplyResult;

/** Calls only server-side receipt endpoints. The server HMAC key is never a
 * browser input. The host must still display preview evidence and obtain an
 * authorization decision before applyAuthenticated. */
export declare class AuthenticatedPaintPartitionHttpClient {
  constructor(options: AuthenticatedPaintPartitionClientOptions);
  propose(request: PaintPartitionRequestBase): Promise<ReportEnvelope<PaintPartitionProposal>>;
  previewAuthenticated(request: PaintPartitionPreviewRequest): Promise<ReportEnvelope<AuthenticatedPaintPartitionPreviewReport>>;
  applyAuthenticated(request: PaintPartitionApplyRequest): Promise<PaintPartitionApplyResult>;
}
