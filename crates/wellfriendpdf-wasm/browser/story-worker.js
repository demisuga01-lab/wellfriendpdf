// Dedicated worker only. Never evaluate document data, fetch a PDF URL or run
// active PDF content. The application supplies trusted WASM asset URLs explicitly.
import { assertStorySessionCapabilities, assertFontInstanceCapabilities } from "./story-capabilities.js";
let sdk, session;
let scopedReview, paintPartitionReview, paintPartitionPreview;
let chain = Promise.resolve();
const parse = (value) => JSON.parse(value);
const sameBytes = (left, right) => left === undefined ? right === undefined
  : right instanceof Uint8Array && left.byteLength === right.byteLength && left.every((value,index)=>value===right[index]);
function state(current = session) {
  assertStorySessionCapabilities(current);
  return { revision: current.revisionSha256(), line_break_policy_version:2, line_shaping_context_version:3, story_pagination_policy_version:1, history_paragraph_style_protocol_version:1, history_paragraph_structure_protocol_version:1, history_inline_style_protocol_version:1, history_compaction_protocol_version:1, stories: parse(current.savedStoriesJson()), pages: parse(current.pagesJson()) };
}
async function dispatch(message) {
  const { id, command, payload = {} } = message;
  let preimage;
  try {
    let result;
    if (command === "open") {
      if (!sdk) {
        sdk = await import(payload.wasmModuleUrl);
        await sdk.default(payload.wasmBinaryUrl);
      }
      const next = new sdk.StoryEditSession(payload.bytes);
      try { result = state(next); } catch (error) { next.close(); next.free(); throw error; }
      session?.close(); session?.free(); session = next; scopedReview = undefined; paintPartitionReview = undefined; paintPartitionPreview = undefined;
    } else {
      if (!session) throw new Error("Open a PDF before editing");
      switch (command) {
        case "state": result = state(); break;
        case "source": result = parse(session.sourceModelJson(payload.page)); break;
        case "inspectFont":
        case "prepareFont":
        case "prepareFontInstance": {
          if (!(payload.bytes instanceof Uint8Array) || !payload.bytes.length || payload.bytes.length > 4 * 1024 * 1024)
            throw new Error("Session font budget is 1..=4 MiB");
          if (command === "prepareFontInstance") assertFontInstanceCapabilities(session);
          const request = command === "inspectFont"
            ? { op: "inspect_font", bytes: Array.from(payload.bytes) }
            : command === "prepareFontInstance"
            ? { op: "prepare_font_instance", lookup_name: payload.lookupName, bytes: Array.from(payload.bytes), request: payload.request }
            : { op: "prepare_font", lookup_name: payload.lookupName, bytes: Array.from(payload.bytes), selection: payload.selection };
          result = parse(session.commandJson(JSON.stringify(request)));
          break;
        }
        case "scopedSources": {
          const doc = new sdk.WellfriendPdf(session.bytes());
          try { result = parse(doc.universalEditingAnalyzeV2Json(JSON.stringify({pages:[payload.page],include_scoped_text_sources:true}))).report.scoped_text_sources; }
          finally { doc.free(); }
          break;
        }
        case "scopedPlan": {
          scopedReview = undefined;
          const doc = new sdk.WellfriendPdf(session.bytes());
          try { result = parse(doc.universalEditingPlanV2Json(JSON.stringify(payload.request))).report; }
          finally { doc.free(); }
          break;
        }
        case "scopedPreview": {
          scopedReview = undefined;
          const plan = JSON.stringify(payload.plan), revision = session.revisionSha256();
          const doc = new sdk.WellfriendPdf(session.bytes());
          try { result = parse(doc.universalEditingScopedPreviewV2Json(plan, JSON.stringify(payload.options ?? {}))).report; }
          finally { doc.free(); }
          scopedReview = { plan, revision };
          // Use transferable byte arrays instead of cloning PNG JSON arrays.
          for (const page of result.pages) for (const side of [page.before,page.candidate]) side.png = Uint8Array.from(side.png);
          break;
        }
        case "scopedApply": {
          const plan = JSON.stringify(payload.plan);
          if (!scopedReview || scopedReview.plan !== plan || scopedReview.revision !== session.revisionSha256()) throw new Error("Render this exact scoped plan before applying it");
          scopedReview = undefined;
          preimage = session.bytes();
          const approval = parse(sdk.WellfriendPdf.universalEditingApprovalV2Json(plan, JSON.stringify(payload.decision))).report;
          const doc = new sdk.WellfriendPdf(preimage);
          let bytes, report;
          try {
            const output = doc.universalEditingApplyV2(plan, JSON.stringify(approval));
            try { bytes = output.bytes(); report = parse(output.reportJson()).report; } finally { output.free(); }
          } finally { doc.free(); }
          if (bytes.byteLength > 256 * 1024 * 1024) throw new Error("Output exceeds the browser session's 256 MiB budget");
          const next = new sdk.StoryEditSession(bytes);
          try { result = { ...state(next), report, bytes }; } catch (error) { next.close(); next.free(); throw error; }
          session.close(); session.free(); session = next;
          break;
        }
        case "paintPartitionPropose": {
          paintPartitionReview = undefined; paintPartitionPreview = undefined;
          const request = JSON.stringify(payload.request), revision = session.revisionSha256();
          const doc = new sdk.WellfriendPdf(session.bytes());
          try { result = parse(doc.proposeTextRangePaintPartitions(request)).report; }
          finally { doc.free(); }
          if (result.input_sha256 !== revision) throw new Error("Paint-partition proposal revision differs from the retained session");
          paintPartitionReview = { request, proposal: JSON.stringify(result), revision };
          break;
        }
        case "paintPartitionPreview": {
          paintPartitionPreview = undefined;
          const request = JSON.stringify(payload.request), proposal = JSON.stringify(payload.proposal);
          const approval = JSON.stringify(payload.approval), revision = session.revisionSha256();
          if (!paintPartitionReview || paintPartitionReview.request !== request || paintPartitionReview.proposal !== proposal || paintPartitionReview.revision !== revision)
            throw new Error("Propose this exact paint partition in the current worker revision before previewing it");
          const font = payload.fontBytes;
          if (font !== undefined && (!(font instanceof Uint8Array) || !font.byteLength || font.byteLength > 4 * 1024 * 1024))
            throw new Error("Approved paint-partition font budget is 1..=4 MiB");
          const dpi = payload.dpi ?? 96;
          if (!Number.isInteger(dpi) || dpi < 24 || dpi > 600) throw new Error("Paint-partition preview DPI must be 24..=600");
          const source = new sdk.WellfriendPdf(session.bytes());
          let preview;
          try {
            preview = parse(source.previewTextRangePaintPartitions(
              request, proposal, approval, font,
              JSON.stringify({ pages:[payload.request.page], dpi })
            )).report;
          } finally { source.free(); }
          const page = preview?.pages?.find((item) => item.page === payload.request.page);
          if (!page) throw new Error("Paint-partition preview omitted the requested page");
          if (!/^[0-9a-f]{64}$/.test(preview?.candidate_output_sha256 ?? "")) throw new Error("Paint-partition preview lacks an exact candidate output digest");
          if (preview.revision_id !== revision) throw new Error("Paint-partition preview revision differs from the retained session");
          const publicationReceipt = JSON.stringify(preview.publication_receipt);
          const previewSummary = {...preview}; delete previewSummary.publication_receipt;
          paintPartitionPreview = { request, proposal, approval, revision,
            font:font?.slice(), candidateOutputSha256:preview.candidate_output_sha256,
            publicationReceipt };
          result = { input_sha256:revision, proposal_id:payload.proposal.proposal_id,
            candidate_output_sha256:preview.candidate_output_sha256, page:payload.request.page, dpi,
            before_png:Uint8Array.from(page.before.png), candidate_png:Uint8Array.from(page.candidate.png),
            difference:page.difference, report:preview.edit_report,
            preview_report:{...previewSummary,pages:preview.pages.map((item) => ({...item,before:{...item.before,png:undefined},candidate:{...item.candidate,png:undefined}}))} };
          break;
        }
        case "paintPartitionApply": {
          const request = JSON.stringify(payload.request), proposal = JSON.stringify(payload.proposal), approval = JSON.stringify(payload.approval);
          const revision = session.revisionSha256();
          if (!paintPartitionReview || paintPartitionReview.request !== request || paintPartitionReview.proposal !== proposal || paintPartitionReview.revision !== revision)
            throw new Error("Propose and review this exact paint partition in the current worker revision before applying it");
          const font = payload.fontBytes;
          if (font !== undefined && (!(font instanceof Uint8Array) || !font.byteLength || font.byteLength > 4 * 1024 * 1024))
            throw new Error("Approved paint-partition font budget is 1..=4 MiB");
          if (!paintPartitionPreview || paintPartitionPreview.request !== request || paintPartitionPreview.proposal !== proposal || paintPartitionPreview.approval !== approval || paintPartitionPreview.revision !== revision || !sameBytes(paintPartitionPreview.font,font))
            throw new Error("Render and review this exact paint-partition approval and font before applying it");
          const expectedOutputSha256 = paintPartitionPreview.candidateOutputSha256;
          const publicationReceipt = paintPartitionPreview.publicationReceipt;
          paintPartitionReview = undefined; paintPartitionPreview = undefined; scopedReview = undefined; preimage = session.bytes();
          const doc = new sdk.WellfriendPdf(preimage);
          let bytes, report;
          try {
            const output = doc.applyReviewedTextRangePaintPartitions(
              request, proposal, JSON.stringify(payload.approval),
              publicationReceipt, font);
            try { bytes = output.bytes(); report = parse(output.reportJson()).report; }
            finally { output.free(); }
          } finally { doc.free(); }
          if (report.output_sha256 !== expectedOutputSha256) throw new Error("Paint-partition apply output differs from the reviewed candidate");
          if (bytes.byteLength > 256 * 1024 * 1024) throw new Error("Output exceeds the browser session's 256 MiB budget");
          const next = new sdk.StoryEditSession(bytes);
          try { result = { ...state(next), report, bytes }; }
          catch (error) { next.close(); next.free(); throw error; }
          session.close(); session.free(); session = next;
          break;
        }
        case "annotations": result = parse(session.annotationSourcesJson()); break;
        case "images": result = parse(session.imageSourcesJson(payload.page)); break;
        case "imageOcrSources": result = parse(sdk.imageOcrSourcesJson(session.bytes(), payload.page)); break;
        case "formTextSources": {
          const doc = new sdk.WellfriendPdf(session.bytes());
          try { result = parse(doc.formTextSourcesJson(payload.page)).report; }
          finally { doc.free(); }
          break;
        }
        case "imageMovePreview": result = parse(sdk.previewImageFragmentMoveJson(session.bytes(), JSON.stringify(payload.request))); break;
        case "imageMove": {
          scopedReview = undefined;
          preimage = session.bytes();
          const output = sdk.applyImageFragmentMove(preimage, JSON.stringify(payload.request), payload.approvedPlanSha256);
          let bytes, report;
          try { bytes = output.bytes(); report = parse(output.reportJson()); } finally { output.free(); }
          if (bytes.byteLength > 256 * 1024 * 1024) throw new Error("Output exceeds the browser session's 256 MiB budget");
          const next = new sdk.StoryEditSession(bytes);
          try { result = { ...state(next), report, bytes }; } catch (error) { next.close(); next.free(); throw error; }
          session.close(); session.free(); session = next;
          break;
        }
        case "tags": result = parse(session.tagSourcesJson()); break;
        case "tableValues": result = parse(session.synchronizeTableValuesJson(JSON.stringify(payload.request))); break;
        case "preview": result = parse(session.previewJson(JSON.stringify(payload.request))); break;
        case "mergeText": result = parse(session.mergeTextJson(JSON.stringify(payload.request))); break;
        case "mergeStructure": result = parse(session.mergeStructureJson(JSON.stringify(payload.request))); break;
        case "reviewStructure": result = parse(session.commandJson(JSON.stringify({op:"review_structure",request:payload.request}))); break;
        case "resolveStructure": result = parse(session.commandJson(JSON.stringify({op:"resolve_structure",request:payload.request,resolution:payload.resolution}))); break;
        case "textHistory": {
          if (!["text_history_new","text_history_merge","text_history_edit","text_history_style","text_history_structure","text_history_inline_style","text_history_resolve_inline_style","text_history_delta","text_history_set_active","text_history_set_many_active"].includes(payload.request?.op)) throw new Error("Invalid text-history command");
          result = parse(session.commandJson(JSON.stringify(payload.request))); break;
        }
        case "historyCommand": {
          if (!["history_resume","history_prepare","history_join","history_edit","history_style","history_structure","history_inline_style","history_resolve_inline_style","history_delta","history_set_active","history_set_many_active","history_preview","history_compaction_plan"].includes(payload.request?.op)) throw new Error("Invalid history preparation command");
          result = parse(session.commandJson(JSON.stringify(payload.request))); break;
        }
        case "historyCompaction": {
          scopedReview=undefined;preimage=session.bytes();
          const report=parse(session.commandJson(JSON.stringify({op:"history_compaction_apply",request:payload.request,approved_plan_sha256:payload.approvedPlanSha256})));
          const bytes=session.bytes();if(bytes.byteLength>256*1024*1024)throw new Error("Output exceeds the browser session's 256 MiB budget");
          result={...state(),report,bytes};break;
        }
        case "historyCheckpoint": {
          scopedReview=undefined;preimage=session.bytes();
          const report=parse(session.commandJson(JSON.stringify({op:"history_checkpoint",source:payload.source,receipt:payload.receipt})));
          const bytes=session.bytes();if(bytes.byteLength>256*1024*1024)throw new Error("Output exceeds the browser session's 256 MiB budget");
          result={...state(),report,bytes};break;
        }
        case "checkpoint": {
          scopedReview = undefined;
          preimage = session.bytes();
          const report = parse(session.checkpointJson(JSON.stringify(payload.request), JSON.stringify(payload.receipt)));
          const bytes = session.bytes();
          if (bytes.byteLength > 256 * 1024 * 1024) throw new Error("Output exceeds the browser session's 256 MiB budget");
          result = { ...state(), report, bytes };
          break;
        }
        case "render": {
          const dpi = payload.dpi ?? 96;
          const geometry = parse(session.pageGeometryJson(payload.page, dpi));
          result = { png: session.renderPagePng(payload.page, dpi), geometry, revision: session.revisionSha256() };
          break;
        }
        case "close": session.close(); session.free(); session = undefined; scopedReview = undefined; paintPartitionReview = undefined; paintPartitionPreview = undefined; result = null; break;
        default: throw new Error("Unknown story worker command");
      }
    }
    const transfers = [];
    if (result?.bytes) transfers.push(result.bytes.buffer);
    if (result?.png) transfers.push(result.png.buffer);
    if (command === "paintPartitionPreview") transfers.push(result.before_png.buffer,result.candidate_png.buffer);
    if (command === "scopedPreview") for (const page of result.pages) for (const side of [page.before,page.candidate]) transfers.push(side.png.buffer);
    postMessage({ id, ok: true, result }, transfers);
  } catch (error) {
    // A failed publication (including saved metadata verification) is atomic
    // from the host's perspective. Reopen the exact preimage, not a partial PDF.
    if (preimage) {
      session?.close(); session?.free();
      session = new sdk.StoryEditSession(preimage);
    }
    postMessage({ id, ok: false, error: String(error?.message ?? error).slice(0, 8192) });
  }
}
self.onmessage = ({ data }) => {
  chain = chain.then(() => dispatch(data)).catch((error) => {
    postMessage({ id: data.id, ok: false, error: `Worker recovery failed: ${String(error)}` });
  });
};
