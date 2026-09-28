import type { UniversalScopedTextRequest, FormTextTarget, AppearanceTextTarget, WidgetTextTarget, OcrCarrierSelection } from "../wellfriendpdf.js";

export interface ScopedTextOperation { kind: "scoped_text"; request: UniversalScopedTextRequest & { planned_output_sha256?: string | null } }
export interface ScopedTextPlan {
  plan_id: string; revision_id: string; state: "ready"|"approval_required"|"policy_denied"|"target_not_found"|"irrecoverable_input";
  requested_operation: ScopedTextOperation; execution_operation: ScopedTextOperation;
  selected_candidate_ids: string[]; policy: Record<string,unknown>; preview: Record<string,unknown>;
  [key:string]: unknown;
}
export interface ScopedPreviewOptions { pages?:number[]; dpi?:number; require_exact?:boolean; max_total_pixels?:number; channel_tolerance?:number }
export interface ScopedRaster { png:Uint8Array; png_sha256:string; rgba_sha256:string; diagnostics:Record<string,unknown> }
export interface ScopedCandidatePreview {
  schema_version:string; plan_id:string; revision_id:string; input_sha256:string; candidate_output_sha256:string;
  options:ScopedPreviewOptions; total_pixels:number; affected_pages_not_previewed:number[]; limitations:string[];
  pages:{page:number; width:number; height:number; before:ScopedRaster; candidate:ScopedRaster;
    difference:{channel_tolerance:number; changed_pixels:number; maximum_channel_delta:number; bounds:[number,number,number,number]|null}}[];
}
export interface ScopedSourceOccurrence {
  target:FormTextTarget|AppearanceTextTarget; text:{logical_text:string; source_spans:unknown[]; writing_mode:number; exact_limits:string[]};
  form_bbox?:[number,number,number,number]; source_bbox?:[number,number,number,number]; coordinate_space:string; external_actual_text_owner:boolean;annotation_subtype?:string;
}
export interface WidgetFieldSource { target:WidgetTextTarget;value:string;flags:number;widgets:{annotation:[number,number];page:number;annotation_index:number;display_text:string}[];default_appearance:Record<string,unknown>;limits:string[] }
export interface ScopedSourceInventory { input_sha256:string; pages:{page:number; forms:{occurrences:ScopedSourceOccurrence[]}; appearances:{occurrences:ScopedSourceOccurrence[]}}[];widget_fields:WidgetFieldSource[] }

export interface StoryWorkerOptions {
  wasmModuleUrl: string | URL;
  wasmBinaryUrl?: string | URL;
  workerUrl?: string | URL;
  workerFactory?: (url: string | URL) => Worker;
}
export interface StoryFrame {
  id: string; page: number; logical_range: [number, number]; expected_text: string;
  rect: [number, number, number, number]; exclusions: [number, number, number, number][];
  owner?: { key: string; content_sha256: string } | null;
}
export interface StoryParagraph {
  id: string; text: string; preferred_font: string; font_size: number; line_height: number;
  rgb?: [number,number,number]; rtl?: boolean; keep_with_next?: boolean; keep_together?: boolean;
  break_before?: boolean; page_break_before?: "none"|"next_page"|"next_odd_page"|"next_even_page";
  orphans?: number; widows?: number; space_before?: number; space_after?: number;
  shaping?: { language?: string | null; features?: string[] };
  inline_styles?: StoryInlineStyleSpan[];
  line_break?: {
    profile?: "unicode" | "japanese_strict";
    emergency?: "break_word" | "preserve_words";
    composition?: "greedy" | "balanced";
    prohibit_start?: string; prohibit_end?: string;
  };
}
export interface StoryInlineStyleSpan {
  logical_range:[number,number];preferred_font?:string|null;font_size?:number|null;
  rgb?:[number,number,number]|null;shaping?:StoryParagraph["shaping"]|null;
}
export interface StoryRequest {
  writing_mode?: "horizontal_tb" | "vertical_rl" | "vertical_lr";
  story_id: string; input_sha256: string; frames: StoryFrame[]; paragraphs: StoryParagraph[];
  fonts: { lookup_name: string; bytes: number[] }[];
  annotation_anchors?: StoryAnnotationAnchor[];
  figures?: StoryFigure[];
  figure_removals?: StoryFigureRemoval[];
  source_tags?: StoryTagging | null;
  table_layout?: TableLayout | null;
  mode?: "preserve_layout" | "flow_document"; allow_font_substitution?: boolean;
  allow_page_creation?: boolean; max_new_pages?: number; signature_policy_override?: boolean;
  prune_empty_pages?: boolean;
}
export interface StoryAnnotationGroup {
  topology_sha256: string;
  members: { annotation_id: string; geometry_sha256: string; page: number }[];
  paint_order: string[];
}
export interface StoryAnnotationSource {
  annotation_id: string; page: number; subtype: string; rect: [number,number,number,number];
  name?: string | null;
  geometry_sha256: string; group?: StoryAnnotationGroup | null;
}
export interface StoryAnnotationAnchor {
  annotation_id: string; paragraph_id: string; geometry_sha256: string;
  offset: [number,number]; group?: StoryAnnotationGroup | null;
  rename_conflicting_names?: boolean;
}
export interface StoryFigureBinding {
  key:string; page:number; rect:[number,number,number,number]; content_sha256:string;
}
export interface StoryFigureRemoval { figure_id:string; binding:StoryFigureBinding }
export interface StoryFigure {
  id: string; caption_paragraph: string;
  source: {kind:"occurrence"; page:number; content_stream_index:number; occurrence_id:string}
    | {kind:"owned"; binding:StoryFigureBinding};
  ocr?:OcrCarrierSelection|null; ocr_unrelated?:boolean;
  width:number; height:number; gap?:number;
  alignment?:"left"|"center"|"right";
  stack:"background"|"foreground";
}
export interface TagReference { object: number; generation?: number; key?: string | null }
export interface DecimalValue { coefficient: string; scale: number }
export type TableFormula = {kind:"constant"; value:DecimalValue} | {kind:"cell"; id:string}
  | {kind:"add"|"subtract"|"multiply"; left:TableFormula; right:TableFormula} | {kind:"sum"; cells:string[]};
export type TableValue = {kind:"text"; text:string} | {kind:"decimal"; value:DecimalValue}
  | {kind:"formula"; expression:TableFormula; display_scale:number};
export interface TableLayout {
  column_weights: number[];
  rows: {id:string; min_height?:number; allow_split?:boolean; break_before?:boolean; keep_with_next?:boolean}[];
  cells: {id:string; paragraph_ids?:string[]; row:number; column:number; row_span?:number; column_span?:number;
    padding?:[number,number,number,number]; background?:[number,number,number]|null; value?:TableValue|null}[];
  header_rows?:number; border?:{width:number; rgb:[number,number,number]}|null;
  source_paint?:{page:number; stable_id:string; action:"keep"|"remove"}[];
  tagging?: TableTagging | null;
}
export interface TableParagraphFragment {
  paragraph_id:string; logical_byte_range:[number,number]; line_count:number;
}
export interface TableCellFragment {
  cell_id:string; row:number; column:number; row_span:number; column_span:number;
  rect:[number,number,number,number];
  /** For explicit paragraph_ids, includes a virtual separator after every block.
   * Use paragraph_fragments for actual source-text byte ranges. */
  logical_byte_range:[number,number]; paragraph_fragments:TableParagraphFragment[];
  repeated_header:boolean; continued:boolean; line_count:number;
}
export interface TableTagging {
  source: TagReference;
  rows: Record<string, TagReference | null>;
  cells: Record<string, TagReference | null>;
  semantics: Record<string, {role:"TH"|"TD"; scope?:"Row"|"Column"|"Both"|null; headers?:string[]}>;
  content_paths?: Record<string, {source:TagReference; semantic_text?:{alternate?:string|null; expansion?:string|null}|null}[]>;
  blocks?: Record<string, {path?:{source:TagReference; semantic_text?:{alternate?:string|null; expansion?:string|null}|null}[];
    new_role?:"P"|"Span"|"H"|"H1"|"H2"|"H3"|"H4"|"H5"|"H6"|"Quote"|"Code"|null;
    semantic_text?:{alternate?:string|null; expansion?:string|null}|null}>;
  groups?: {id:string; role:"THead"|"TBody"|"TFoot"; rows:string[]; source?:TagReference|null; semantic_text?:{alternate?:string|null; expansion?:string|null}|null}[];
  removed_groups?: TagReference[];
  semantic_text?: Record<string, {alternate?:string|null; expansion?:string|null}>;
  row_text?: Record<string, {alternate?:string|null; expansion?:string|null}>;
  table_text?: {alternate?:string|null; expansion?:string|null}|null;
}
export interface StoryTagging {
  parent: TagReference; selected: TagReference[]; insert_at?: number | null;
  paragraph_sources?: Record<string, TagReference | null>; new_roles?: Record<string, string>;
  semantic_text?: Record<string, { alternate?: string | null; expansion?: string | null }>;
  figures?: Record<string, {
    source?:TagReference|null;
    semantic_text?:{alternate?:string|null; expansion?:string|null}|null;
    /** Preserve residual Figure and approved content-only OCR owners when moving one reused Form occurrence. */
    split_reused_form_semantics?:boolean;
    /** Required when the Figure or a preserved descendant owns an external /Ref. */
    outbound_ref_split?:"move_with_selected"|"retain_with_residual"|"copy_to_both"|null;
    /** Required when an external structure owner points to the Figure/subtree. */
    inbound_ref_split?:"follow_selected"|"retarget_residual"|"reference_both"|null;
    /** Preserve a bounded descendant structure tree only when every descendant
     * is semantic-only and owns no independent content or OBJR item. Explicit
     * descendant page bindings follow the Figure destination page. */
    preserve_semantic_subtree?:boolean;
    /** One-shot approval used only while removing this Figure. Deletes the
     * complete validated contentless subtree; external relationships refuse. */
    delete_semantic_subtree?:boolean;
    /** One-shot approval to copy a contentless subtree onto the residual owner
     * created by a reused-Form occurrence split. Internal Ref links follow the
     * clones; external links use outbound_ref_split/inbound_ref_split. */
    clone_semantic_subtree_for_reused_form?:boolean;
    /** Backward-compatible one-owner spelling. An omitted span_ids array means
     * every exact OCR span selected for this Figure. */
    separate_ocr_owner?:{source:TagReference;policy:"merge_into_figure";span_ids?:string[]}|null;
    /** Explicitly consume selected content-only P/Span OCR owners. Every entry
     * identifies a nonempty, disjoint subset of the Figure's exact span_ids. */
    separate_ocr_owners?:{source:TagReference;policy:"merge_into_figure";span_ids:string[]}[];
  }>;
}
export interface StoryTagSource {
  reference: TagReference; parent: TagReference | null; role: string;
  stable_keys: string[];
  child_position: number | null; page_mcids: [number,number][]; text_leaf: boolean;
  alternate: string | null; expansion: string | null;
}
export interface StoryState {
  line_break_policy_version: 2;
  line_shaping_context_version: 3;
  story_pagination_policy_version: 1;
  history_paragraph_style_protocol_version: 1;
  history_paragraph_structure_protocol_version: 1;
  history_inline_style_protocol_version: 1;
  history_compaction_protocol_version: 1;
  revision: string; stories: { schema_version: number; request: StoryRequest }[];
  pages: { page: number; crop_box: number[]; media_box: number[]; rotate: number; user_unit: number }[];
  report?: unknown;
}
export interface PaintPartitionRequest extends Record<string,unknown> {
  page:number;logical_start:number;logical_end:number;replacement_text:string;
  mode:"paragraph_reflow_horizontal"|"paragraph_reflow_rtl"|"paragraph_reflow_vertical";
  style_policy:string;options:Record<string,unknown>;
}
export interface PaintPartitionCandidate {
  source_text_object:number;selected_source_scalar_count:number;
  replacement_scalar_range:[number,number];selected_span_ids:string[];
  suggested_region?:[number,number,number,number];
  start_boundary_class:string;end_boundary_class:string;
}
export interface PaintPartitionProposal {
  schema_version:string;input_sha256:string;request_sha256:string;proposal_id:string;
  page:number;logical_range:[number,number];replacement_sha256:string;
  candidates:PaintPartitionCandidate[];deterministic:true;exact_limits:string[];
}
export interface PaintPartitionApprovalEntry {
  source_text_object:number;region:[number,number,number,number];final_lines?:unknown[]|null;
}
export interface PaintPartitionApproval {
  proposal_id:string;font_sha256?:string|null;partitions:PaintPartitionApprovalEntry[];
}
export interface PaintPartitionPreview {
  input_sha256:string;proposal_id:string;candidate_output_sha256:string;
  page:number;dpi:number;before_png:Uint8Array;candidate_png:Uint8Array;
  difference?:Record<string,unknown>;report:Record<string,unknown>;
  preview_report?:Record<string,unknown>;
}
export interface StoryPreviewReceipt { revision_sha256: string; request_sha256: string; preview_sha256: string }
export interface StoryPageBreakReceipt {
  paragraph_id:string; byte_offset:number; source:"paragraph_policy"|"form_feed";
  policy:"next_page"|"next_odd_page"|"next_even_page"; from_page:number; to_page:number;
}
export interface StoryLayoutPreview extends Record<string,unknown> { page_breaks:StoryPageBreakReceipt[] }
export interface StoryStructureBranch {branch_id:string;base_story_sha256:string;proposed:StoryRequest}
export interface StoryStructureMergeRequest {base:StoryRequest;branches:StoryStructureBranch[]}
export type StructureConflictTarget = {kind:"frame_field";frame_id:string;field:string}
  | {kind:"paragraph_field";paragraph_id:string;field:string}
  | {kind:"paragraph"|"retained_paragraph";paragraph_id:string}
  | {kind:"paragraph_order"}
  | {kind:"insertions"|"table_projection";paragraph_ids:string[]};
export interface ReviewedStructureConflict {
  conflict_id:string;target:StructureConflictTarget;path:string;reason:string;branch_ids:string[];
  alternatives:{branch_id:string|null;value:unknown}[];
}
export interface StructureResolution {
  expected_review_sha256:string;acknowledged_conflicts:string[];paragraphs:StoryParagraph[];
  frame_geometry:{frame_id:string;rect:[number,number,number,number];exclusions:[number,number,number,number][]}[];
}
export interface StructureReview {schema_version:1;input_sha256:string;base_story_sha256:string;review_sha256:string;candidate:StoryRequest;unplaced_paragraphs:StoryParagraph[];conflicts:ReviewedStructureConflict[];limits:string[]}
export interface StructureResolutionResult {merged:StoryRequest;review_sha256:string;resolution_sha256:string;resolved_conflict_ids:string[];limits:string[]}
export interface StoryOperationId { actor:string; sequence:number }
export interface StoryAtomId { operation:StoryOperationId|null;offset:number }
export interface StoryAtomRange { operation:StoryOperationId|null;start:number;end:number }
export interface StoryTextOperation {
  id:StoryOperationId;lamport:number;context:Record<string,number>;paragraph_id:string;
  after:StoryAtomId|null;removed:StoryAtomRange[];inserted:string;
  visibility?:{target:StoryOperationId;active:boolean}|null;
  paragraph_style?:StoryParagraphStylePatch|null;
  paragraph_structure?:StoryParagraphStructurePatch|null;
  inserted_style?:StoryResolvedInlineStyle|null;
  inline_style?:StoryInlineStyleOperation|null;
}
export type StoryParagraphStylePatch = Partial<Omit<StoryParagraph,"id"|"text">>;
export interface StoryTextHistory {
  schema_version:1|2|3|4|5;base_revision_sha256:string;base_story_sha256:string;operations:StoryTextOperation[];
}
export interface StoryHistoryEdit {
  expected_history_sha256:string;actor:string;paragraph_id:string;
  /** Exact UTF-8 byte offsets, on grapheme boundaries in the current projection. */
  range:[number,number];expected_text:string;replacement:string;
}
export interface StoryHistoryResult {
  history:StoryTextHistory;history_sha256:string;frontier:Record<string,number>;
  missing_dependencies:StoryOperationId[];merged:StoryRequest|null;paragraph_ids:string[];atom_count:number;tombstone_count:number;limits:string[];
  inactive_operations:StoryOperationId[];suppressed_atom_count:number;
  style_conflicts:{paragraph_id:string;field:string;candidates:{operation:StoryOperationId;value:unknown}[]}[];
  style_conflicts_sha256:string;
  structure_conflicts:{paragraph_id:string;field:"present"|"position";candidates:{operation:StoryOperationId|null;value:unknown}[]}[];
  structure_conflicts_sha256:string;
  inline_style_runs:StoryInlineStyleRun[];
  inline_conflicts:StoryInlineStyleConflict[];
  inline_conflicts_sha256:string;
}
export interface StoryHistoryStyleEdit {
  expected_history_sha256:string;expected_style_conflicts_sha256:string;actor:string;paragraph_id:string;
  expected:StoryParagraphStylePatch;replacement:StoryParagraphStylePatch;
}
export interface StoryParagraphPosition {after:string|null}
export interface StoryParagraphStructurePatch {
  present?:boolean|null;position?:StoryParagraphPosition|null;inserted_paragraph?:StoryParagraph|null;
}
export interface StoryHistoryStructureEdit {
  expected_history_sha256:string;expected_structure_conflicts_sha256:string;actor:string;paragraph_id:string;
  expected_absent?:boolean;expected:StoryParagraphStructurePatch;replacement:StoryParagraphStructurePatch;
}
export type StoryInlineStyleField="preferred_font"|"font_size"|"rgb"|"shaping";
export interface StoryInlineStylePatch {
  clear?:StoryInlineStyleField[];
  preferred_font?:string|null;font_size?:number|null;rgb?:[number,number,number]|null;
  shaping?:StoryParagraph["shaping"]|null;
}
export interface StoryResolvedInlineStyle {
  preferred_font?:string|null;font_size?:number|null;rgb?:[number,number,number]|null;
  shaping?:StoryParagraph["shaping"]|null;
}
export interface StoryInlineStyleOperation {targets:StoryAtomRange[];patch:StoryInlineStylePatch}
export interface StoryHistoryInlineStyleEdit {
  expected_history_sha256:string;expected_inline_conflicts_sha256:string;actor:string;paragraph_id:string;
  range:[number,number];expected_text:string;replacement:StoryInlineStylePatch;
}
export interface StoryHistoryInlineStyleResolution {
  expected_history_sha256:string;expected_inline_conflicts_sha256:string;actor:string;paragraph_id:string;
  targets:StoryAtomRange[];replacement:StoryInlineStylePatch;
}
export interface StoryInlineStyleRun {paragraph_id:string;logical_range:[number,number];style:StoryResolvedInlineStyle}
export interface StoryInlineStyleConflict {
  paragraph_id:string;target:StoryAtomId;field:StoryInlineStyleField;
  candidates:{operation:StoryOperationId;value:unknown}[];
}
export interface StoryHistorySetActive {
  expected_history_sha256:string;actor:string;target:StoryOperationId;expected_active:boolean;active:boolean;
}
export interface StoryHistorySetManyActive {
  expected_history_sha256:string;actor:string;targets:StoryOperationId[];expected_active:boolean;active:boolean;
}
/** Convert one textarea-normalized typing burst into one grapheme-safe,
 * exact-source history replacement. Returns null when no text changed. */
export function coalescedTextEdit(logicalText:string,normalizedBefore:string,normalizedAfter:string):
  {range:[number,number];expected_text:string;replacement:string}|null;
export type HistorySource = {kind:"start";base:StoryRequest;history:StoryTextHistory;replace_epoch?:boolean}
  | {kind:"resume";input_sha256:string;story_id:string;expected_checkpoint_sha256:string;history:StoryTextHistory};
export interface PreparedHistory {source:HistorySource;result:StoryHistoryResult;checkpoint_before:string|null;generation_before:number|null}
export interface HistoryPreviewReceipt {layout:StoryPreviewReceipt;source_sha256:string;history_sha256:string}
export interface HistoryLayoutPreview {prepared:PreparedHistory;preview:Record<string,unknown>;receipt:HistoryPreviewReceipt}
export interface HistoryCheckpointReport {
  story:Record<string,unknown>;checkpoint_sha256:string;generation:number;
  seed_sha256:string;history_sha256:string;output_sha256:string;same_epoch_preserved:boolean;limits:string[];
}
export interface HistoryCompactionRequest {
  input_sha256:string;story_id:string;expected_checkpoint_sha256:string;expected_history_sha256:string;
  acknowledged_frontier:Record<string,number>;acknowledge_operation_and_undo_loss:boolean;
  acknowledge_prior_epoch_rejected:boolean;
}
export interface HistoryCompactionPlan {
  schema_version:1;input_sha256:string;story_id:string;checkpoint_before:string;history_sha256:string;
  generation_before:number;acknowledged_frontier:Record<string,number>;source_operation_count:number;
  source_atom_count:number;source_tombstone_count:number;source_inactive_operation_count:number;
  plan_sha256:string;limits:string[];
}
export interface HistoryCompactionReport {
  schema_version:1;story_id:string;checkpoint_before:string;checkpoint_after:string;
  generation_before:number;generation_after:number;seed_before:string;seed_after:string;
  history_before:string;history_after:string;retired_operation_count:number;retired_atom_count:number;
  retired_tombstone_count:number;output_sha256:string;prior_epoch_rejected:true;
  exact_session_undo_available:boolean;limits:string[];
}
export interface FontAxis {tag:string; min:number; default:number; max:number}
export type FontOutlineFormat = "true_type" | "cff1" | "cff2" | "other";
export interface FontAssetFace {
  face_index:number; family:string|null; subfamily:string|null; postscript_name:string|null;
  outline_format:FontOutlineFormat; glyph_count:number; units_per_em:number; axes:FontAxis[];
  permission_bits_allow_editing:boolean; subsetting_allowed:boolean; signature_present:boolean;
}
export interface FontAssetCatalog {schema_version:1; source_sha256:string; collection:boolean; faces:FontAssetFace[]}
export interface FontFaceSelection {source_sha256:string; face_index:number; allow_signature_removal?:boolean}
export interface FontPreparationReport {
  schema_version:1; source_sha256:string; prepared_sha256:string; face_index:number; source_face_count:number;
  extracted_collection:boolean; removed_signature:boolean; signature_verified:false;
  outline_format:FontOutlineFormat; subsetting_allowed:boolean; retained_variation_axes:FontAxis[];
}
export interface PreparedFont {asset:{lookup_name:string; bytes:number[]}; report:FontPreparationReport}
export type FontStyleLink = "regular" | "bold" | "italic" | "bold_italic";
export interface FontInstanceNaming {family:string;subfamily:string;legacy_family:string;postscript_name:string;style_link:FontStyleLink}
export interface FontInstanceRequest {
  selection:FontFaceSelection;coordinates:Record<string,number>;naming:FontInstanceNaming;
  accept_redundant_metric_differences?:boolean;
  cff2_contours?:{tolerance_font_units:number;allow_hint_loss?:boolean}|null;
}
export interface Cff2ContourReport {
  algorithm:string;tolerance_font_units:number;solver_epsilon:number;coordinate_quantum:number;
  normalized_glyphs:number[];dehinted_glyphs:number[];emptied_glyphs:number[];private_hint_dictionaries_removed:number;
  /** Absent in older reports; unchanged glyph/private hint owners in current output. */
  preserved_glyphs?:number[];preserved_hint_glyphs?:number[];
  private_hint_dictionaries_bypassed?:number;private_hint_dictionaries_retained?:number;
  /** Exact fixed-point classification only, not independent rendering qualification. */
  exact_linear_preserved_glyphs?:number[];exact_linear_work?:number;
  input_segments:number;output_segments:number;symmetric_difference_area:number;broadphase_pairs:number;potential_intersections:number;independently_verified:false;
}
export interface FontInstanceReport {
  schema_version:1;source_sha256:string;prepared_sha256:string;face_index:number;source_face_count:number;
  coordinates:Record<string,number>;normalized_coordinates:number[];naming:FontInstanceNaming;glyph_count:number;
  output_outline_format:"true_type"|"cff1";
  cff2:{source_font_dicts:number;output_font_dicts:number;glyphs:number;stem_hints:number;masks:number;glyph_blends:number;private_blends:number;reordered_stem_snap_arrays:number;expanded_instructions:number;contour_overlaps_checked:boolean;contour_overlaps_removed:boolean;contour_normalization:Cff2ContourReport|null;preserved_contour_check?:Cff2ContourReport|null}|null;
  removed_tables:string[];changed_tables:string[];preserved_tables:string[];removed_signature:boolean;
  signature_verified:false;subsetting_allowed:boolean;
  metric_differences:{glyph:number;vertical:boolean;field:string;declared:number;derived:number}[];
  normalized_component_offsets:[number,number][];repaired_source_bounds:number[];checked_layout_point_references:number;
  replaced_name_records:number;relocated_stat_name_ids:Record<string,number>;retained_stat_values:number;removed_stat_values:number;
  hint_instruction_capacity:number;hint_stack_capacity:number;structural_postconditions_checked:true;independently_render_verified:false;
}
export interface PreparedFontInstance {asset:{lookup_name:string;bytes:number[]};report:FontInstanceReport}
export interface PageGeometry { page: number; width: number; height: number; pdf_to_device: [number,number,number,number,number,number] }
export class StoryWorkerClient extends EventTarget {
  constructor(options: StoryWorkerOptions);
  open(bytes: Uint8Array): Promise<StoryState>;
  readonly state: StoryState | undefined;
  readonly canUndo: boolean; readonly canRedo: boolean;
  bytes(): Uint8Array;
  preview(request: StoryRequest): Promise<{preview: StoryLayoutPreview; receipt: StoryPreviewReceipt; dirty_regions: unknown[]}>;
  checkpoint(request: StoryRequest, receipt: StoryPreviewReceipt): Promise<StoryState>;
  source(page: number): Promise<{page: number; logical_text: string; source_spans: unknown[]; exact_limits: string[]}>;
  inspectFont(bytes:Uint8Array):Promise<FontAssetCatalog>;
  prepareFont(lookupName:string, bytes:Uint8Array, selection:FontFaceSelection):Promise<PreparedFont>;
  prepareFontInstance(lookupName:string, bytes:Uint8Array, request:FontInstanceRequest):Promise<PreparedFontInstance>;
  scopedSources(page:number):Promise<ScopedSourceInventory>;
  planScopedText(request:{operation:ScopedTextOperation; policy?:Record<string,unknown>}):Promise<ScopedTextPlan>;
  previewScopedText(plan:ScopedTextPlan,options?:ScopedPreviewOptions):Promise<ScopedCandidatePreview>;
  applyScopedText(plan:ScopedTextPlan,decision:{selected_candidate_ids:string[];approved_font?:string|null;
    mutation_mode:"preserve_signatures"|"authorized_rewrite";accept_visual_change:boolean;accept_signature_invalidation:boolean}):Promise<StoryState>;
  proposePaintPartitions(request:PaintPartitionRequest):Promise<PaintPartitionProposal>;
  previewPaintPartitions(request:PaintPartitionRequest,proposal:PaintPartitionProposal,
    approval:PaintPartitionApproval,options?:{dpi?:number;fontBytes?:Uint8Array}):Promise<PaintPartitionPreview>;
  applyPaintPartitions(request:PaintPartitionRequest,proposal:PaintPartitionProposal,
    approval:PaintPartitionApproval,fontBytes?:Uint8Array):Promise<StoryState>;
  annotations(): Promise<StoryAnnotationSource[]>;
  images(page: number): Promise<{page:number; occurrence_id:string; content_stream_index:number; bbox:[number,number,number,number]; resource_name?:string|null; invocation_path:unknown[]}[]>;
  imageOcrSources(page:number):Promise<{source_spans:Array<{span_id:string;text:string;text_render_mode:number;flow_relocatable:boolean}>}>;
  formTextSources(page:number):Promise<{input_sha256:string;occurrences:Array<{target:FormTextTarget;text:{source_spans:Array<{span_id:string;text:string;text_render_mode:number;flow_relocatable:boolean}>};external_actual_text_owner:boolean}>}>;
  tags(): Promise<StoryTagSource[]>;
  synchronizeTableValues(request: StoryRequest): Promise<StoryRequest>;
  render(page: number, dpi?: number): Promise<{png: Uint8Array; geometry: PageGeometry; revision: string}>;
  mergeText(request: unknown): Promise<{merged: StoryRequest | null; conflicts: unknown[]}>;
  mergeStructure(request: unknown): Promise<{merged: StoryRequest | null; conflicts: unknown[]}>;
  reviewStructure(request:StoryStructureMergeRequest):Promise<StructureReview>;
  resolveStructure(request:StoryStructureMergeRequest,resolution:StructureResolution):Promise<StructureResolutionResult>;
  beginTextHistory(base:StoryRequest):Promise<StoryHistoryResult>;
  mergeTextHistories(base:StoryRequest,histories:StoryTextHistory[]):Promise<StoryHistoryResult>;
  editTextHistory(base:StoryRequest,history:StoryTextHistory,edit:StoryHistoryEdit):Promise<StoryHistoryResult>;
  styleTextHistory(base:StoryRequest,history:StoryTextHistory,edit:StoryHistoryStyleEdit):Promise<StoryHistoryResult>;
  structureTextHistory(base:StoryRequest,history:StoryTextHistory,edit:StoryHistoryStructureEdit):Promise<StoryHistoryResult>;
  inlineStyleTextHistory(base:StoryRequest,history:StoryTextHistory,edit:StoryHistoryInlineStyleEdit):Promise<StoryHistoryResult>;
  resolveInlineStyleTextHistory(base:StoryRequest,history:StoryTextHistory,resolution:StoryHistoryInlineStyleResolution):Promise<StoryHistoryResult>;
  textHistoryDelta(base:StoryRequest,history:StoryTextHistory,peer:Record<string,number>):Promise<StoryTextHistory>;
  setTextHistoryOperationActive(base:StoryRequest,history:StoryTextHistory,change:StoryHistorySetActive):Promise<StoryHistoryResult>;
  setTextHistoryOperationsActive(base:StoryRequest,history:StoryTextHistory,change:StoryHistorySetManyActive):Promise<StoryHistoryResult>;
  resumeHistory(storyId:string):Promise<PreparedHistory>;
  prepareHistory(source:HistorySource):Promise<PreparedHistory>;
  joinHistory(source:HistorySource,histories:StoryTextHistory[]):Promise<PreparedHistory>;
  editHistory(source:HistorySource,edit:StoryHistoryEdit):Promise<PreparedHistory>;
  styleHistory(source:HistorySource,edit:StoryHistoryStyleEdit):Promise<PreparedHistory>;
  structureHistory(source:HistorySource,edit:StoryHistoryStructureEdit):Promise<PreparedHistory>;
  inlineStyleHistory(source:HistorySource,edit:StoryHistoryInlineStyleEdit):Promise<PreparedHistory>;
  resolveInlineStyleHistory(source:HistorySource,resolution:StoryHistoryInlineStyleResolution):Promise<PreparedHistory>;
  historyDelta(source:HistorySource,peer:Record<string,number>):Promise<StoryTextHistory>;
  setHistoryOperationActive(source:HistorySource,change:StoryHistorySetActive):Promise<PreparedHistory>;
  setHistoryOperationsActive(source:HistorySource,change:StoryHistorySetManyActive):Promise<PreparedHistory>;
  previewHistory(source:HistorySource):Promise<HistoryLayoutPreview>;
  planHistoryCompaction(request:HistoryCompactionRequest):Promise<HistoryCompactionPlan>;
  compactHistory(request:HistoryCompactionRequest,approvedPlanSha256:string):Promise<StoryState & {report:HistoryCompactionReport}>;
  checkpointHistory(source:HistorySource,receipt:HistoryPreviewReceipt):Promise<StoryState & {report:HistoryCheckpointReport}>;
  undo(): Promise<boolean>; redo(): Promise<boolean>;
  cancel(): Promise<StoryState | undefined>;
  close(): void;
}
export function pdfRectToCss(rect: number[], geometry: PageGeometry): {left: number; top: number; width: number; height: number};
export function devicePointToPdf(x: number, y: number, geometry: PageGeometry): [number,number];
export function sourceSelectionRange(logicalText: string, start: number, end: number): [number,number];
