/** A serialized, revision-aware controller. Worker termination really stops
 * WASM CPU work; it is not a cancellation message queued behind synchronous WASM.
 * The last published PDF and bounded undo preimages live outside the worker. */
const sameBytes=(left,right)=>left===undefined?right===undefined:
  right instanceof Uint8Array&&left.byteLength===right.byteLength&&left.every((value,index)=>value===right[index]);
export class StoryWorkerClient extends EventTarget {
  #worker; #options; #pending = new Map(); #next = 1; #epoch = 0;
  #tail = Promise.resolve(); #ready = Promise.resolve(); #bytes; #state;
  #undo = []; #redo = []; #closed = false;
  #scopedReview; #paintPartitionReview; #paintPartitionPreview;
  constructor(options) {
    super();
    if (!options?.wasmModuleUrl) throw new TypeError("A trusted wasmModuleUrl is required");
    this.#options = { ...options };
    this.#start();
  }
  #start() {
    const factory = this.#options.workerFactory ?? ((url) => new Worker(url, { type: "module" }));
    const worker = factory(this.#options.workerUrl ?? new URL("./story-worker.js", import.meta.url));
    this.#worker = worker;
    worker.onmessage = ({ data }) => {
      if (this.#worker !== worker) return;
      const pending = this.#pending.get(data.id);
      if (!pending) return;
      this.#pending.delete(data.id);
      if (data.ok) pending.resolve(data.result); else pending.reject(new Error(data.error));
    };
    const failed = () => {
      if (this.#worker !== worker) return;
      this.#stop(new Error("Editing worker failed; call cancel() to restore the last published PDF"));
      this.dispatchEvent(new Event("workererror"));
    };
    worker.onerror = failed;
    worker.onmessageerror = failed;
  }
  #stop(reason) {
    this.#scopedReview = undefined; this.#paintPartitionReview = undefined; this.#paintPartitionPreview = undefined;
    this.#epoch++;
    this.#worker?.terminate(); this.#worker = undefined;
    for (const pending of this.#pending.values()) pending.reject(reason);
    this.#pending.clear(); this.#tail = Promise.resolve();
  }
  #send(command, payload = {}) {
    if (this.#closed || !this.#worker) return Promise.reject(new Error("Editing worker is unavailable"));
    const id = this.#next++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject });
      try { this.#worker.postMessage({ id, command, payload }); }
      catch (error) { this.#pending.delete(id); reject(error); }
    });
  }
  #queue(task) {
    const epoch = this.#epoch;
    const guard = () => { if (this.#closed || epoch !== this.#epoch) throw new DOMException("Operation superseded", "AbortError"); };
    const operation = this.#tail.then(async () => {
      await this.#ready;
      guard();
      const result = await task(guard);
      guard();
      return result;
    });
    this.#tail = operation.catch(() => {});
    return operation;
  }
  #openBytes(bytes) {
    return this.#send("open", { bytes, wasmModuleUrl: String(this.#options.wasmModuleUrl),
      wasmBinaryUrl: this.#options.wasmBinaryUrl ? String(this.#options.wasmBinaryUrl) : undefined });
  }
  #publish(state, bytes) {
    this.#scopedReview = undefined; this.#paintPartitionReview = undefined; this.#paintPartitionPreview = undefined;
    this.#state = state; this.#bytes = bytes;
    this.dispatchEvent(new Event("change"));
    return structuredClone(state);
  }
  async open(bytes) {
    if (!(bytes instanceof Uint8Array) || bytes.byteLength > 256 * 1024 * 1024) throw new TypeError("Expected PDF bytes within the 256 MiB budget");
    const owned = bytes.slice();
    return this.#queue(async (guard) => {
      const state = await this.#openBytes(owned);
      guard();
      this.#undo = []; this.#redo = [];
      return this.#publish(state, owned);
    });
  }
  get state() { return this.#state ? structuredClone(this.#state) : undefined; }
  get canUndo() { return this.#undo.length > 0; }
  get canRedo() { return this.#redo.length > 0; }
  bytes() { if (!this.#bytes) throw new Error("No published PDF"); return this.#bytes.slice(); }
  #boundRequest(request) {
    const copy = structuredClone(request);
    if (!this.#state || copy.input_sha256 !== this.#state.revision) throw new Error("Request belongs to another PDF revision; reload source bindings");
    return copy;
  }
  preview(request) {
    const owned = this.#boundRequest(request);
    return this.#queue(() => this.#send("preview", { request: owned }));
  }
  checkpoint(request, receipt) {
    const owned = this.#boundRequest(request), approval = structuredClone(receipt);
    return this.#queue(async (guard) => {
      const result = await this.#send("checkpoint", { request: owned, receipt: approval });
      guard();
      if (this.#bytes.byteLength <= 128 * 1024 * 1024) this.#undo.push(this.#bytes);
      this.#redo = []; this.#trim();
      const { bytes, ...state } = result;
      return this.#publish(state, bytes);
    });
  }
  source(page) { return this.#queue(() => this.#send("source", { page })); }
  #fontBytes(bytes) {
    if (!(bytes instanceof Uint8Array) || !bytes.byteLength || bytes.byteLength > 4 * 1024 * 1024)
      throw new TypeError("Expected font bytes within the 1..=4 MiB session budget");
    return bytes.slice();
  }
  inspectFont(bytes) {
    const owned = this.#fontBytes(bytes);
    return this.#queue(() => this.#send("inspectFont", { bytes: owned }));
  }
  prepareFont(lookupName, bytes, selection) {
    const owned = this.#fontBytes(bytes), decision = structuredClone(selection);
    if (typeof lookupName !== "string" || !lookupName.trim()) throw new TypeError("A font lookup name is required");
    return this.#queue(() => this.#send("prepareFont", { lookupName, bytes: owned, selection: decision }));
  }
  prepareFontInstance(lookupName, bytes, request) {
    const owned = this.#fontBytes(bytes), decision = structuredClone(request);
    if (typeof lookupName !== "string" || !lookupName.trim()) throw new TypeError("A font lookup name is required");
    return this.#queue(() => this.#send("prepareFontInstance", { lookupName, bytes: owned, request: decision }));
  }
  scopedSources(page) { return this.#queue(() => this.#send("scopedSources", { page })); }
  #scopedRevision(operation) {
    if (operation?.kind !== "scoped_text") throw new TypeError("Expected a native scoped_text operation");
    const revision = operation.request?.source?.request?.target?.input_sha256;
    if (!this.#state || revision !== this.#state.revision) throw new Error("Scoped source belongs to another PDF revision; rediscover it");
    return revision;
  }
  planScopedText(request) {
    const owned = structuredClone(request); this.#scopedRevision(owned.operation);
    this.#scopedReview = undefined;
    return this.#queue(() => { this.#scopedReview = undefined; this.#scopedRevision(owned.operation); return this.#send("scopedPlan", { request:owned }); });
  }
  previewScopedText(plan, options = {}) {
    const owned = structuredClone(plan), settings = structuredClone(options);
    const revision = this.#scopedRevision(owned.requested_operation);
    this.#scopedReview = undefined;
    return this.#queue(async (guard) => {
      this.#scopedReview = undefined;
      this.#scopedRevision(owned.requested_operation);
      const result = await this.#send("scopedPreview", { plan:owned, options:settings });
      guard();
      if (result.input_sha256 !== revision || result.plan_id !== owned.plan_id || result.candidate_output_sha256 !== owned.execution_operation?.request?.planned_output_sha256) throw new Error("Scoped preview receipt differs from its plan");
      this.#scopedReview = { plan:JSON.stringify(owned), revision };
      return result;
    });
  }
  /** Requires a completed preview in this worker epoch. The receipt proves
   * binding, not human review or authorization; the host supplies decisions. */
  applyScopedText(plan, decision) {
    const owned = structuredClone(plan), approved = structuredClone(decision);
    this.#scopedRevision(owned.requested_operation);
    return this.#queue(async (guard) => {
      const revision = this.#scopedRevision(owned.requested_operation);
      if (this.#scopedReview?.plan !== JSON.stringify(owned) || this.#scopedReview.revision !== revision) throw new Error("Preview this exact scoped plan before applying it");
      this.#scopedReview = undefined;
      const result = await this.#send("scopedApply", { plan:owned, decision:approved });
      guard();
      if (result.report?.changed === true) {
        if (this.#bytes.byteLength <= 128 * 1024 * 1024) this.#undo.push(this.#bytes);
        this.#redo = []; this.#trim();
      }
      const { bytes, ...state } = result;
      return this.#publish(state, bytes);
    });
  }
  /** Generate a non-mutating exact-revision replacement-to-paint-slot plan. */
  proposePaintPartitions(request) {
    const owned = structuredClone(request);
    this.#paintPartitionReview = undefined; this.#paintPartitionPreview = undefined;
    return this.#queue(async (guard) => {
      this.#paintPartitionReview = undefined;
      const proposal = await this.#send("paintPartitionPropose", { request: owned });
      guard();
      if (!this.#state || proposal.input_sha256 !== this.#state.revision || !/^[0-9a-f]{64}$/.test(proposal.proposal_id ?? ""))
        throw new Error("Paint-partition proposal is not bound to the current PDF revision");
      this.#paintPartitionReview = { request:JSON.stringify(owned), proposal:JSON.stringify(proposal), revision:this.#state.revision };
      return proposal;
    });
  }
  /** Render the exact approval privately. Candidate PDF bytes remain in the
   * worker; only before/candidate PNGs and the exact output digest are returned. */
  previewPaintPartitions(request, proposal, approval, options = {}) {
    const owned = structuredClone(request), planned = structuredClone(proposal), approved = structuredClone(approval);
    const font = options.fontBytes == null ? undefined : this.#fontBytes(options.fontBytes);
    const dpi = options.dpi ?? 96;
    this.#paintPartitionPreview = undefined;
    return this.#queue(async (guard) => {
      const exact = this.#paintPartitionReview;
      if (!exact || exact.request !== JSON.stringify(owned) || exact.proposal !== JSON.stringify(planned) || exact.revision !== this.#state?.revision)
        throw new Error("Propose this exact paint partition before previewing it");
      if (approved?.proposal_id !== planned.proposal_id) throw new Error("Paint-partition approval belongs to another proposal");
      const result = await this.#send("paintPartitionPreview", {request:owned,proposal:planned,approval:approved,fontBytes:font,dpi});
      guard();
      if (result.input_sha256 !== this.#state?.revision || result.proposal_id !== planned.proposal_id || !/^[0-9a-f]{64}$/.test(result.candidate_output_sha256 ?? ""))
        throw new Error("Paint-partition preview receipt differs from its proposal");
      this.#paintPartitionPreview = {request:JSON.stringify(owned),proposal:JSON.stringify(planned),approval:JSON.stringify(approved),revision:this.#state.revision,font:font?.slice(),candidateOutputSha256:result.candidate_output_sha256};
      return result;
    });
  }
  /** Publish only the exact proposal reviewed in this worker epoch. Optional
   * font bytes must be approved by matching approval.font_sha256; the engine
   * performs the authoritative digest check before mutation. */
  applyPaintPartitions(request, proposal, approval, fontBytes) {
    const owned = structuredClone(request), planned = structuredClone(proposal), approved = structuredClone(approval);
    const font = fontBytes == null ? undefined : this.#fontBytes(fontBytes);
    if (!this.#state || planned.input_sha256 !== this.#state.revision || approved?.proposal_id !== planned.proposal_id)
      throw new Error("Paint-partition approval belongs to another proposal or PDF revision");
    return this.#queue(async (guard) => {
      const exact = this.#paintPartitionReview;
      if (!exact || exact.request !== JSON.stringify(owned) || exact.proposal !== JSON.stringify(planned) || exact.revision !== this.#state?.revision)
        throw new Error("Propose and review this exact paint partition before applying it");
      const preview = this.#paintPartitionPreview;
      if (!preview || preview.request !== JSON.stringify(owned) || preview.proposal !== JSON.stringify(planned) || preview.approval !== JSON.stringify(approved) || preview.revision !== this.#state?.revision || !sameBytes(preview.font,font))
        throw new Error("Render and review this exact paint-partition approval and font before applying it");
      this.#paintPartitionReview = undefined; this.#paintPartitionPreview = undefined;
      const result = await this.#send("paintPartitionApply", { request:owned, proposal:planned, approval:approved, fontBytes:font });
      guard();
      if (result.report?.output_sha256 !== preview.candidateOutputSha256) throw new Error("Published paint-partition output differs from its reviewed preview");
      if (this.#bytes.byteLength <= 128 * 1024 * 1024) this.#undo.push(this.#bytes);
      this.#redo = []; this.#trim();
      const { bytes, ...state } = result;
      return this.#publish(state, bytes);
    });
  }
  annotations() { return this.#queue(() => this.#send("annotations")); }
  images(page) { return this.#queue(() => this.#send("images", { page })); }
  imageOcrSources(page) { return this.#queue(() => this.#send("imageOcrSources", { page })); }
  formTextSources(page) { return this.#queue(() => this.#send("formTextSources", { page })); }
  previewImageMove(request) {
    const owned = this.#boundRequest(request);
    return this.#queue(() => this.#send("imageMovePreview", { request: owned }));
  }
  /** Explicit native transaction, with the same cancellation/publication and
   * bounded undo semantics as a story checkpoint. Not implicit story reflow. */
  moveImage(request, approvedPlanSha256) {
    const owned = this.#boundRequest(request);
    if (typeof approvedPlanSha256 !== "string" || !/^[0-9a-f]{64}$/.test(approvedPlanSha256)) throw new TypeError("Expected an exact image preview receipt");
    return this.#queue(async (guard) => {
      const result = await this.#send("imageMove", { request: owned, approvedPlanSha256 });
      guard();
      if (this.#bytes.byteLength <= 128 * 1024 * 1024) this.#undo.push(this.#bytes);
      this.#redo = []; this.#trim();
      const { bytes, ...state } = result;
      return this.#publish(state, bytes);
    });
  }
  tags() { return this.#queue(() => this.#send("tags")); }
  synchronizeTableValues(request) { const owned = structuredClone(request); return this.#queue(() => this.#send("tableValues", { request: owned })); }
  render(page, dpi = 96) { return this.#queue(() => this.#send("render", { page, dpi })); }
  mergeText(request) { const owned = structuredClone(request); return this.#queue(() => this.#send("mergeText", { request: owned })); }
  mergeStructure(request) { const owned = structuredClone(request); return this.#queue(() => this.#send("mergeStructure", { request: owned })); }
  reviewStructure(request) {const owned=structuredClone(request);return this.#queue(()=>this.#send("reviewStructure",{request:owned}));}
  resolveStructure(request,resolution) {const owned=structuredClone(request),decisions=structuredClone(resolution);return this.#queue(()=>this.#send("resolveStructure",{request:owned,resolution:decisions}));}
  beginTextHistory(base) { const request = structuredClone({op:"text_history_new",base}); return this.#queue(() => this.#send("textHistory", {request})); }
  mergeTextHistories(base,histories) { const request = structuredClone({op:"text_history_merge",base,histories}); return this.#queue(() => this.#send("textHistory", {request})); }
  editTextHistory(base,history,edit) { const request = structuredClone({op:"text_history_edit",base,history,edit}); return this.#queue(() => this.#send("textHistory", {request})); }
  styleTextHistory(base,history,edit) { const request=structuredClone({op:"text_history_style",base,history,edit});return this.#queue(()=>this.#send("textHistory",{request})); }
  structureTextHistory(base,history,edit) { const request=structuredClone({op:"text_history_structure",base,history,edit});return this.#queue(()=>this.#send("textHistory",{request})); }
  inlineStyleTextHistory(base,history,edit) { const request=structuredClone({op:"text_history_inline_style",base,history,edit});return this.#queue(()=>this.#send("textHistory",{request})); }
  resolveInlineStyleTextHistory(base,history,resolution) { const request=structuredClone({op:"text_history_resolve_inline_style",base,history,resolution});return this.#queue(()=>this.#send("textHistory",{request})); }
  textHistoryDelta(base,history,peer) { const request = structuredClone({op:"text_history_delta",base,history,peer}); return this.#queue(() => this.#send("textHistory", {request})); }
  setTextHistoryOperationActive(base,history,change) { const request=structuredClone({op:"text_history_set_active",base,history,change});return this.#queue(()=>this.#send("textHistory",{request})); }
  setTextHistoryOperationsActive(base,history,change) { const request=structuredClone({op:"text_history_set_many_active",base,history,change});return this.#queue(()=>this.#send("textHistory",{request})); }
  #historyCommand(value) { const request=structuredClone(value);return this.#queue(()=>this.#send("historyCommand",{request})); }
  resumeHistory(storyId) {return this.#historyCommand({op:"history_resume",story_id:storyId});}
  prepareHistory(source) {return this.#historyCommand({op:"history_prepare",source});}
  joinHistory(source,histories) {return this.#historyCommand({op:"history_join",source,histories});}
  editHistory(source,edit) {return this.#historyCommand({op:"history_edit",source,edit});}
  styleHistory(source,edit) {return this.#historyCommand({op:"history_style",source,edit});}
  structureHistory(source,edit) {return this.#historyCommand({op:"history_structure",source,edit});}
  inlineStyleHistory(source,edit) {return this.#historyCommand({op:"history_inline_style",source,edit});}
  resolveInlineStyleHistory(source,resolution) {return this.#historyCommand({op:"history_resolve_inline_style",source,resolution});}
  historyDelta(source,peer) {return this.#historyCommand({op:"history_delta",source,peer});}
  setHistoryOperationActive(source,change) {return this.#historyCommand({op:"history_set_active",source,change});}
  setHistoryOperationsActive(source,change) {return this.#historyCommand({op:"history_set_many_active",source,change});}
  previewHistory(source) {return this.#historyCommand({op:"history_preview",source});}
  planHistoryCompaction(request) {return this.#historyCommand({op:"history_compaction_plan",request});}
  compactHistory(request,approvedPlanSha256) {
    const owned=structuredClone(request);
    if(typeof approvedPlanSha256!=="string"||!/^[0-9a-f]{64}$/i.test(approvedPlanSha256))throw new TypeError("Expected an exact history-compaction plan hash");
    return this.#queue(async guard=>{
      const result=await this.#send("historyCompaction",{request:owned,approvedPlanSha256});guard();
      if(this.#bytes.byteLength<=128*1024*1024)this.#undo.push(this.#bytes);
      this.#redo=[];this.#trim();const {bytes,...state}=result;return this.#publish(state,bytes);
    });
  }
  checkpointHistory(source,receipt) {
    const owned=structuredClone(source),approved=structuredClone(receipt);
    return this.#queue(async guard=>{
      const result=await this.#send("historyCheckpoint",{source:owned,receipt:approved});guard();
      if(this.#bytes.byteLength<=128*1024*1024)this.#undo.push(this.#bytes);
      this.#redo=[];this.#trim();const {bytes,...state}=result;return this.#publish(state,bytes);
    });
  }
  #trim() {
    while (this.#undo.length + this.#redo.length > 8 || [...this.#undo, ...this.#redo].reduce((n, b) => n + b.byteLength, 0) > 128 * 1024 * 1024) {
      if (this.#undo.length) this.#undo.shift(); else this.#redo.shift();
    }
  }
  #history(undo) {
    return this.#queue(async (guard) => {
      const from=undo?this.#undo:this.#redo,to=undo?this.#redo:this.#undo;
      if (!from.length) return false;
      const bytes = from.at(-1), state = await this.#openBytes(bytes);
      guard();
      from.pop(); to.push(this.#bytes); this.#trim(); this.#publish(state, bytes); return true;
    });
  }
  undo() { return this.#history(true); }
  redo() { return this.#history(false); }
  /** Abort queued/active work, including an unpublished checkpoint. Exact last
   * published bytes survive; previews and their receipts do not. */
  cancel() {
    if (this.#closed) return Promise.reject(new Error("Session is closed"));
    this.#stop(new DOMException("Editing cancelled", "AbortError")); this.#start();
    const epoch=this.#epoch;
    this.#ready = this.#bytes ? this.#openBytes(this.#bytes).then((state) => {
      if(epoch!==this.#epoch)throw new DOMException("Recovery superseded","AbortError");
      this.#state = state; return state;
    }) : Promise.resolve();
    return this.#ready.then(() => this.state);
  }
  close() {
    this.#closed = true; this.#stop(new DOMException("Session closed", "AbortError"));
    this.#bytes = undefined; this.#state = undefined; this.#undo = []; this.#redo = [];
  }
}

/** Device transforms come from the native renderer, including CropBox,
 * rotation and UserUnit. DOM coordinates are never used as PDF coordinates. */
export function pdfRectToCss(rect, geometry) {
  const [a,b,c,d,e,f] = geometry.pdf_to_device;
  const points = [[rect[0],rect[1]],[rect[2],rect[1]],[rect[2],rect[3]],[rect[0],rect[3]]]
    .map(([x,y]) => [a*x+c*y+e,b*x+d*y+f]);
  const xs=points.map(p=>p[0]),ys=points.map(p=>p[1]);
  return {left:100*Math.min(...xs)/geometry.width,top:100*Math.min(...ys)/geometry.height,
    width:100*(Math.max(...xs)-Math.min(...xs))/geometry.width,height:100*(Math.max(...ys)-Math.min(...ys))/geometry.height};
}
export function devicePointToPdf(x,y,geometry) {
  const [a,b,c,d,e,f]=geometry.pdf_to_device,det=a*d-b*c;
  if (!Number.isFinite(det)||Math.abs(det)<1e-12) throw new Error("Non-invertible page transform");
  return [(d*(x-e)-c*(y-f))/det,(-b*(x-e)+a*(y-f))/det];
}

/** HTML textareas normalize CR/CRLF and use UTF-16 selection offsets. The PDF
 * model uses Unicode scalar offsets. Refuse a half-surrogate selection and map
 * normalized newlines back to the exact source rather than guessing indices. */
export function sourceSelectionRange(logicalText,start,end) {
  if(!Number.isInteger(start)||!Number.isInteger(end)||start<0||end<start)throw new Error("Invalid source selection");
  const scalars=Array.from(logicalText);let dom=0,from,to;
  for(let i=0;i<=scalars.length;i++){
    if(dom===start)from=i;if(dom===end){to=i;break;}if(i===scalars.length)break;
    if(scalars[i]==="\r"){dom++;if(scalars[i+1]==="\n")i++;}else dom+=scalars[i].length;
  }
  if(from===undefined||to===undefined)throw new Error("Selection splits a Unicode scalar or exceeds the source");
  return [from,to];
}

/** Collapse one browser typing burst into the smallest single contiguous
 * replacement that covers every change in that burst. The returned range is
 * expressed in exact UTF-8 bytes of logicalText, not DOM UTF-16 offsets.
 * Grapheme boundaries prevent a coalesced edit from splitting a combining
 * sequence; sourceSelectionRange preserves CR/CRLF and supplementary scalars. */
export function coalescedTextEdit(logicalText,normalizedBefore,normalizedAfter) {
  if(typeof logicalText!=="string"||typeof normalizedBefore!=="string"||typeof normalizedAfter!=="string")throw new TypeError("Typing coalescing requires text strings");
  if(typeof Intl?.Segmenter!=="function")throw new Error("This browser cannot safely group Unicode typing; Intl.Segmenter is required");
  const normalizeNewlines=text=>text.replace(/\r\n|\r/g,"\n");
  if(normalizeNewlines(logicalText)!==normalizedBefore)throw new Error("Textarea preimage no longer matches the exact source text");
  const segmenter=new Intl.Segmenter(undefined,{granularity:"grapheme"});
  const segments=text=>Array.from(segmenter.segment(text),part=>({text:part.segment,start:part.index,end:part.index+part.segment.length}));
  const before=segments(normalizedBefore),after=segments(normalizedAfter);
  let prefix=0;
  while(prefix<before.length&&prefix<after.length&&before[prefix].text===after[prefix].text)prefix++;
  let suffix=0;
  while(suffix<before.length-prefix&&suffix<after.length-prefix&&before[before.length-1-suffix].text===after[after.length-1-suffix].text)suffix++;
  if(prefix===before.length&&prefix===after.length)return null;
  const beforeStart=prefix<before.length?before[prefix].start:normalizedBefore.length;
  const beforeEnd=suffix?before[before.length-suffix].start:normalizedBefore.length;
  const afterStart=prefix<after.length?after[prefix].start:normalizedAfter.length;
  const afterEnd=suffix?after[after.length-suffix].start:normalizedAfter.length;
  const [scalarStart,scalarEnd]=sourceSelectionRange(logicalText,beforeStart,beforeEnd),scalars=Array.from(logicalText),encoder=new TextEncoder();
  const expected=scalars.slice(scalarStart,scalarEnd).join(""),rawReplacement=normalizedAfter.slice(afterStart,afterEnd);
  const retainedEndings=Array.from(expected.matchAll(/\r\n|\r|\n/g),match=>match[0]);
  const exactBefore=scalars.slice(0,scalarStart).join(""),exactAfter=scalars.slice(scalarEnd).join("");
  const previous=Array.from(exactBefore.matchAll(/\r\n|\r|\n/g)).at(-1)?.[0],following=exactAfter.match(/\r\n|\r|\n/)?.[0];
  const localEnding=previous??following??"\n";let ending=0;
  const replacement=rawReplacement.replace(/\n/g,()=>retainedEndings[ending++]??localEnding);
  return {
    range:[encoder.encode(scalars.slice(0,scalarStart).join("")).length,encoder.encode(scalars.slice(0,scalarEnd).join("")).length],
    expected_text:expected,
    replacement
  };
}
