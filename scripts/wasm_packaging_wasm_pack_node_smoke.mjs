import { createRequire } from "node:module";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const [, , packageDirArg, fixtureArg, outJsonArg] = process.argv;
if (!packageDirArg || !fixtureArg || !outJsonArg) {
  console.error("usage: node wasm_packaging_wasm_pack_node_smoke.mjs <package-dir> <fixture.pdf> <out.json>");
  process.exit(2);
}

const packageDir = resolve(packageDirArg);
const fixture = resolve(fixtureArg);
const outJson = resolve(outJsonArg);
const require = createRequire(import.meta.url);

const result = {
  schema_version: 1,
  status: "failed",
  package_dir: packageDir,
  fixture,
  node_version: process.version,
  apis_tested: [],
  errors: [],
};

try {
  const wellfriendpdf = require(packageDir);
  result.apis_tested.push("package import");

  const feature = JSON.parse(wellfriendpdf.WellfriendPdf.featureReportJson());
  result.apis_tested.push("WellfriendPdf.featureReportJson");
  result.feature_has_codec_isolation = JSON.stringify(feature).includes("codec_isolation");
  result.feature_has_semantic_closeout = JSON.stringify(feature).includes(
    "semantic_closeout_semantic_binding_rag_benchmark_closeout",
  );

  const tableStatus = JSON.parse(wellfriendpdf.WellfriendPdf.tableProposalStatusJson());
  result.table_status_kind = tableStatus.kind;
  result.apis_tested.push("WellfriendPdf.tableProposalStatusJson");

  const bytes = new Uint8Array(readFileSync(fixture));
  const pdf = new wellfriendpdf.WellfriendPdf(bytes);
  result.apis_tested.push("new WellfriendPdf(bytes)");
  result.page_count = pdf.pageCount();
  result.apis_tested.push("pageCount");

  const security = JSON.parse(pdf.securityReportJson());
  result.security_kind = security.kind;
  result.apis_tested.push("securityReportJson");

  const xfa = JSON.parse(pdf.xfaReportJson());
  const xfaExtract = JSON.parse(pdf.xfaExtractJson());
  const xfaScripts = JSON.parse(pdf.xfaScriptReportJson());
  const xfaSecurity = JSON.parse(pdf.xfaSecurityReportJson());
  const xfaRuntime = JSON.parse(pdf.xfaRuntimeReportJson("disabled", false));
  result.xfa_kinds = [
    xfa.kind,
    xfaExtract.kind,
    xfaScripts.kind,
    xfaSecurity.kind,
    xfaRuntime.kind,
  ];
  result.xfa_schema = xfa.report?.schema_version;
  result.apis_tested.push(
    "xfaReportJson",
    "xfaExtractJson",
    "xfaScriptReportJson",
    "xfaSecurityReportJson",
    "xfaRuntimeReportJson",
  );

  const advancedChunks = JSON.parse(pdf.advancedChunksJson());
  result.advanced_chunks_kind = advancedChunks.kind;
  result.apis_tested.push("advancedChunksJson");

  const semanticBundle = JSON.parse(pdf.semanticBundleJson());
  result.semantic_bundle_kind = semanticBundle.kind;
  result.apis_tested.push("semanticBundleJson");

  const semanticSearch = JSON.parse(pdf.semanticSearchJson("the"));
  result.semantic_search_kind = semanticSearch.kind;
  result.apis_tested.push("semanticSearchJson");

  const advanced_editing_closeout = JSON.parse(pdf.advanced_editing_closeoutReportJson());
  result.advanced_editing_closeout_schema = advanced_editing_closeout.report?.schema_version;
  result.apis_tested.push("advanced_editing_closeoutReportJson");

  let advanced_editing_closeoutPdf = pdf;
  let rangeModel = JSON.parse(advanced_editing_closeoutPdf.advanced_editing_closeoutTextRangeAnalyzeJson(1));
  if ((rangeModel.report?.source_spans?.length ?? 0) === 0) {
    const textFixture = resolve(dirname(fixture), "multi_stream.pdf");
    if (existsSync(textFixture)) {
      advanced_editing_closeoutPdf = new wellfriendpdf.WellfriendPdf(new Uint8Array(readFileSync(textFixture)));
      result.apis_tested.push("new WellfriendPdf(multi_stream for AdvancedEditingB)");
      rangeModel = JSON.parse(advanced_editing_closeoutPdf.advanced_editing_closeoutTextRangeAnalyzeJson(1));
    }
  }
  result.advanced_editing_closeout_range_kind = rangeModel.kind;
  result.advanced_editing_closeout_range_logical_length = rangeModel.report?.logical_text?.length ?? 0;
  result.apis_tested.push("advanced_editing_closeoutTextRangeAnalyzeJson");
  const firstRange = rangeModel.report?.source_spans?.[0]?.logical_range;
  if (Array.isArray(firstRange) && firstRange.length === 2) {
    const edit = advanced_editing_closeoutPdf.editTextRange(
      JSON.stringify({
        page: 1,
        logical_start: firstRange[0],
        logical_end: firstRange[1],
        replacement_text: "Wasm20B",
        mode: "paragraph_reflow_horizontal",
        style_policy: "inherit_leading",
        options: {
          region: [20.0, 80.0, 180.0, 140.0],
          font_size: 12.0,
          line_spacing: 1.2,
          max_lines_or_columns: 4096,
          overflow_policy: "error",
          signature_policy_override: false,
          deterministic: true,
        },
      }),
    );
    const editReport = JSON.parse(edit.reportJson());
    result.advanced_editing_closeout_edit_kind = editReport.kind;
    result.advanced_editing_closeout_edit_bytes = edit.bytes().length;
    result.apis_tested.push("editTextRange");
  } else {
    result.errors.push("advanced editing closeout range model did not expose a source span");
  }
  if (advanced_editing_closeoutPdf !== pdf) {
    advanced_editing_closeoutPdf.close();
    result.apis_tested.push("close AdvancedEditingB fixture");
  }

  const text_reflowRequest = JSON.stringify({
    requested_mode: "geometric_block",
    page: 1,
    source_text: "Hello",
    replacement_text: "World",
    region: [10.0, 10.0, 260.0, 90.0],
    language: "en",
    hyphenation: true,
  });
  const text_reflowReport = JSON.parse(pdf.text_reflowReportJson());
  result.text_reflow_schema = text_reflowReport.report?.schema_version;
  result.apis_tested.push("text_reflowReportJson");
  const text_reflowApply = pdf.text_reflowReflowRegion(text_reflowRequest);
  const text_reflowApplyReport = JSON.parse(text_reflowApply.reportJson());
  result.text_reflow_apply_kind = text_reflowApplyReport.kind;
  result.apis_tested.push("text_reflowReflowRegion");
  const text_reflowValidation = JSON.parse(
    pdf.text_reflowValidateReflowOutputJson(text_reflowApply.bytes(), text_reflowRequest),
  );
  result.text_reflow_validation_kind = text_reflowValidation.kind;
  result.apis_tested.push("text_reflowValidateReflowOutputJson");
  const text_reflowUndo = pdf.text_reflowUndoReflow(
    text_reflowApply.bytes(),
    text_reflowRequest,
  );
  const text_reflowUndoReport = JSON.parse(text_reflowUndo.reportJson());
  result.text_reflow_undo_kind = text_reflowUndoReport.kind;
  result.apis_tested.push("text_reflowUndoReflow");

  const codec = JSON.parse(
    wellfriendpdf.WellfriendPdf.codecIsolationReportJson(
      "FlateDecode",
      new Uint8Array([0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0xaf, 0xc8, 0x4c, 0x49, 0x05, 0x00, 0x19, 0xdd, 0x04, 0x4e]),
      "in_process",
    ),
  );
  result.codec_isolation_status = codec.report?.status;
  result.codec_isolation_mode = codec.report?.isolation_mode;
  result.apis_tested.push("codecIsolationReportJson");

  try {
    new wellfriendpdf.WellfriendPdf(new Uint8Array([0x25, 0x50, 0x44, 0x46]));
    result.invalid_input_error = null;
    result.errors.push("invalid input unexpectedly opened");
  } catch (error) {
    result.invalid_input_error = String(error?.message ?? error);
    result.apis_tested.push("invalid input error");
  }

  pdf.close();
  result.apis_tested.push("close");

  const checks = [
    result.feature_has_codec_isolation === true,
    result.feature_has_semantic_closeout === true,
    result.table_status_kind === "table_proposal_status",
    result.page_count >= 1,
    result.security_kind === "security_report",
    result.xfa_schema === "xfa_runtime.xfa.v1",
    JSON.stringify(result.xfa_kinds) === JSON.stringify([
      "xfa_report",
      "xfa_extract_report",
      "xfa_script_report",
      "xfa_security_report",
      "xfa_runtime_report",
    ]),
    result.advanced_chunks_kind === "advanced_rag_chunk_set",
    result.semantic_bundle_kind === "semantic_binding_report",
    result.semantic_search_kind === "semantic_search_report",
    result.advanced_editing_closeout_schema === "advanced_editing_closeout.multirun-form-appearance-closure.v1",
    result.advanced_editing_closeout_range_kind === "advanced_editing_closeout_multi_run_range_model",
    result.advanced_editing_closeout_range_logical_length > 0,
    result.advanced_editing_closeout_edit_kind === "advanced_editing_closeout_multi_run_text_edit_report",
    result.advanced_editing_closeout_edit_bytes > 0,
    result.codec_isolation_status === "success",
    result.text_reflow_schema === "text_reflow.geometric-semantic-reflow.v1",
    result.text_reflow_apply_kind === "text_reflow_reflow_region",
    result.text_reflow_validation_kind === "text_reflow_validate_reflow_output",
    result.text_reflow_undo_kind === "text_reflow_undo_reflow",
    typeof result.invalid_input_error === "string" && result.invalid_input_error.length > 0,
  ];
  result.status = checks.every(Boolean) ? "passed" : "failed";
} catch (error) {
  result.errors.push(String(error?.stack ?? error));
}

writeFileSync(outJson, `${JSON.stringify(result, null, 2)}\n`, "utf8");
if (result.status !== "passed") {
  console.error(JSON.stringify(result, null, 2));
  process.exit(1);
}
