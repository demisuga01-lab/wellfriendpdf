// Regression source only. Not executed during the source-only implementation.
import test from "node:test";
import assert from "node:assert/strict";
import { StoryWorkerClient, pdfRectToCss, devicePointToPdf, sourceSelectionRange, coalescedTextEdit } from "./story-client.js";

class FakeWorker {
  terminated = false;
  holdPreview = false;
  bytes = new Uint8Array([1]);
  constructor() { this.waiting = new Promise(resolve => { this.markWaiting=resolve; }); }
  postMessage({id,command,payload}) {
    queueMicrotask(()=>{
      if(this.terminated)return;
      if((command==="preview"||command==="historyCommand"&&payload.request.op==="history_preview")&&this.holdPreview){this.markWaiting();return;}
      const state=()=>({revision:`r${this.bytes[0]}`,stories:[],pages:[{page:1}]});
      let result;
      if(command==="open"){this.bytes=payload.bytes.slice();result=state();}
      else if(command==="checkpoint"){this.bytes=new Uint8Array([2]);result={...state(),bytes:this.bytes.slice(),report:{}};}
      else if(command==="preview")result={preview:{},receipt:{revision_sha256:state().revision}};
      else if(command==="tableValues")result=payload.request;
      else if(command==="inspectFont"||command==="prepareFont"||command==="prepareFontInstance")result=payload;
      else if(command==="reviewStructure")result={request:payload.request};
      else if(command==="resolveStructure")result={request:payload.request,resolution:payload.resolution};
      else if(command==="textHistory")result={request:payload.request};
      else if(command==="historyCommand")result={request:payload.request};
      else if(command==="historyCompaction"){this.bytes=new Uint8Array([2]);result={...state(),bytes:this.bytes.slice(),report:{request:payload.request,approved_plan_sha256:payload.approvedPlanSha256}};}
      else if(command==="historyCheckpoint"){this.bytes=new Uint8Array([2]);result={...state(),bytes:this.bytes.slice(),report:{source:payload.source,receipt:payload.receipt}};}
      else if(command==="scopedPlan")result={plan_id:"p1",requested_operation:payload.request.operation,execution_operation:{request:{planned_output_sha256:"candidate"}}};
      else if(command==="scopedPreview")result={input_sha256:state().revision,plan_id:payload.plan.plan_id,candidate_output_sha256:payload.plan.execution_operation.request.planned_output_sha256,pages:[]};
      else if(command==="scopedApply"){const changed=payload.decision.change!==false;if(changed)this.bytes=new Uint8Array([2]);result={...state(),bytes:this.bytes.slice(),report:{changed}};}
      else if(command==="paintPartitionPropose")result={input_sha256:state().revision,proposal_id:"a".repeat(64),candidates:[{source_text_object:0,replacement_scalar_range:[0,1]}]};
      else if(command==="paintPartitionPreview")result={input_sha256:state().revision,proposal_id:payload.proposal.proposal_id,candidate_output_sha256:"b".repeat(64),page:1,dpi:96,before_png:new Uint8Array([1]),candidate_png:new Uint8Array([2]),report:{output_sha256:"b".repeat(64)}};
      else if(command==="paintPartitionApply"){this.bytes=new Uint8Array([2]);result={...state(),bytes:this.bytes.slice(),report:{output_sha256:"b".repeat(64),generated_paint_partitions:payload.approval.partitions}};}
      else result=state();
      this.onmessage?.({data:{id,ok:true,result}});
    });
  }
  terminate(){this.terminated=true;}
}
test("source selection preserves scalar indices across CRLF and supplementary characters",()=>{
  assert.deepEqual(sourceSelectionRange("A\r\n😀B",2,4),[3,4]);
  assert.throws(()=>sourceSelectionRange("😀",1,2));
  assert.deepEqual(sourceSelectionRange("A\rB",1,2),[1,2]);
});

test("typing bursts coalesce to one grapheme-safe exact-source replacement",()=>{
  assert.deepEqual(coalescedTextEdit("A\r\ne\u0301Z","A\ne\u0301Z","A\nHELLOZ"),{
    range:[3,6],expected_text:"e\u0301",replacement:"HELLO"
  });
  assert.deepEqual(coalescedTextEdit("A😀B","A😀B","A😀!B"),{
    range:[5,5],expected_text:"",replacement:"!"
  });
  assert.deepEqual(coalescedTextEdit("A\r\nB","A\nB","A\nX\nB"),{
    range:[3,3],expected_text:"",replacement:"X\r\n"
  });
  assert.equal(coalescedTextEdit("same","same","same"),null);
  assert.throws(()=>coalescedTextEdit("A\r\nB","A\r\nB","changed"),/preimage/);
});

test("table recalculation snapshots the draft and does not publish PDF bytes",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));
  const request={input_sha256:"r1",paragraphs:[{id:"cell",text:"original"}]};
  const queued=client.synchronizeTableValues(request);request.paragraphs[0].text="changed after call";
  assert.equal((await queued).paragraphs[0].text,"original");
  assert.equal(client.state.revision,"r1");assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});
test("font discovery and preparation snapshot byte arrays and explicit decisions without PDF mutation",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));
  const bytes=new Uint8Array([4,5,6]),selection={source_sha256:"hash",face_index:2,allow_signature_removal:false};
  const catalog=client.inspectFont(bytes),prepared=client.prepareFont("Chosen",bytes,selection);
  bytes[0]=9;selection.face_index=0;selection.allow_signature_removal=true;
  assert.deepEqual((await catalog).bytes,new Uint8Array([4,5,6]));
  assert.deepEqual((await prepared).selection,{source_sha256:"hash",face_index:2,allow_signature_removal:false});
  assert.deepEqual(client.bytes(),new Uint8Array([1]));assert.equal(client.canUndo,false);client.close();
});
test("font helpers reject invalid or oversized payloads before queueing work",()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  for(const bytes of [[],new Uint8Array(),new Uint8Array(4*1024*1024+1)])assert.throws(()=>client.inspectFont(bytes));
  assert.throws(()=>client.prepareFont(" ",new Uint8Array([1]),{source_sha256:"hash",face_index:0}));
  client.close();
});

test("paint partition publication requires the exact reviewed proposal and is undoable",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));
  const request={page:1,logical_start:0,logical_end:1,replacement_text:"B",mode:"paragraph_reflow_horizontal",style_policy:"inherit_leading",options:{region:[0,0,10,10],font_size:12,line_spacing:1.2,max_lines_or_columns:10,overflow_policy:"error",signature_policy_override:false,deterministic:true}};
  const proposal=await client.proposePaintPartitions(request);
  const approval={proposal_id:proposal.proposal_id,partitions:[{source_text_object:0,region:[0,0,10,10],final_lines:null}]};
  const altered=structuredClone(proposal);altered.candidates[0].replacement_scalar_range=[0,0];
  await assert.rejects(client.applyPaintPartitions(request,altered,approval),/review|proposal/);
  await assert.rejects(client.applyPaintPartitions(request,proposal,approval),/Render|review/);
  const preview=await client.previewPaintPartitions(request,proposal,approval,{dpi:96});
  assert.equal(preview.candidate_output_sha256,"b".repeat(64));assert.deepEqual(client.bytes(),new Uint8Array([1]));
  const published=await client.applyPaintPartitions(request,proposal,approval);
  assert.equal(published.revision,"r2");assert.equal(client.canUndo,true);assert.deepEqual(client.bytes(),new Uint8Array([2]));
  client.close();
});

test("static font requests snapshot axes names and approval without PDF mutation",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));
  const bytes=new Uint8Array([4,5]),request={selection:{source_sha256:"font",face_index:1},coordinates:{wght:700},naming:{family:"Chosen"},accept_redundant_metric_differences:false};
  const pending=client.prepareFontInstance("Chosen",bytes,request);
  bytes[0]=9;request.coordinates.wght=100;request.naming.family="Changed";request.accept_redundant_metric_differences=true;
  const result=await pending;assert.deepEqual(result.bytes,new Uint8Array([4,5]));assert.equal(result.request.coordinates.wght,700);assert.equal(result.request.naming.family,"Chosen");
  assert.equal(result.request.accept_redundant_metric_differences,false);assert.deepEqual(client.bytes(),new Uint8Array([1]));assert.equal(client.canUndo,false);client.close();
});
test("static font helper rejects empty names and oversized byte arrays",()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  assert.throws(()=>client.prepareFontInstance("",new Uint8Array([1]),{}));assert.throws(()=>client.prepareFontInstance("Name",new Uint8Array(4*1024*1024+1),{}));client.close();
});
test("contour-normalization tolerance and hint decision are snapshotted before worker queuing",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));
  const request={cff2_contours:{tolerance_font_units:0.001,allow_hint_loss:false}};
  const pending=client.prepareFontInstance("CFF",new Uint8Array([4]),request);
  request.cff2_contours.tolerance_font_units=0.125;request.cff2_contours.allow_hint_loss=true;
  assert.deepEqual((await pending).request.cff2_contours,{tolerance_font_units:0.001,allow_hint_loss:false});
  assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});
test("native matrix and inverse include rotated crop offsets",()=>{
  const geometry={width:400,height:200,pdf_to_device:[0,2,2,0,-40,-20]};
  assert.deepEqual(devicePointToPdf(0,0,geometry),[10,20]);
  assert.deepEqual(pdfRectToCss([10,20,30,60],geometry),{left:0,top:0,width:20,height:20});
});
test("queued undo/redo use current history and exports cannot mutate retained bytes",async()=>{
  const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});
  await client.open(new Uint8Array([1]));await client.checkpoint({input_sha256:"r1"},{});
  const undo=client.undo(),redo=client.redo();assert.equal(await undo,true);assert.equal(await redo,true);
  assert.equal(client.state.revision,"r2");const exported=client.bytes();exported[0]=9;assert.equal(client.bytes()[0],2);client.close();
});
test("cancellation terminates active CPU worker and restores exact published revision",async()=>{
  const workers=[];const client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>{const w=new FakeWorker();workers.push(w);return w;}});
  await client.open(new Uint8Array([1]));workers[0].holdPreview=true;
  const pending=client.preview({input_sha256:"r1"});const rejection=assert.rejects(pending,{name:"AbortError"});await workers[0].waiting;
  await client.cancel();await rejection;assert.equal(workers[0].terminated,true);assert.equal(client.state.revision,"r1");assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});

const scopedRequest=()=>({operation:{kind:"scoped_text",request:{source:{scope:"form",request:{target:{input_sha256:"r1"},edit:{replacement_text:"old"}}}}}});
const newClient=()=>new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>new FakeWorker()});

test("structure review and resolution snapshot queued inputs without publishing a revision",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const request={base:{input_sha256:"r1",paragraphs:[{id:"p",text:"original"}]},branches:[]};
  const resolution={expected_review_sha256:"review",acknowledged_conflicts:["conflict"],paragraphs:[{id:"p",text:"resolved"}],frame_geometry:[]};
  const reviewing=client.reviewStructure(request),resolving=client.resolveStructure(request,resolution);
  request.base.paragraphs[0].text="changed after call";resolution.paragraphs[0].text="changed";resolution.acknowledged_conflicts.length=0;
  assert.equal((await reviewing).request.base.paragraphs[0].text,"original");
  const result=await resolving;assert.equal(result.request.base.paragraphs[0].text,"original");
  assert.equal(result.resolution.paragraphs[0].text,"resolved");assert.deepEqual(result.resolution.acknowledged_conflicts,["conflict"]);
  assert.equal(client.state.revision,"r1");assert.equal(client.canUndo,false);assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});

test("scoped plan snapshots drafts; preview publishes no bytes and exact apply supports undo",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const request=scopedRequest(),pending=client.planScopedText(request);request.operation.request.source.request.edit.replacement_text="changed after call";
  const plan=await pending;assert.equal(plan.requested_operation.request.source.request.edit.replacement_text,"old");
  await assert.rejects(client.applyScopedText(plan,{}),/Preview this exact/);
  await client.previewScopedText(plan);assert.equal(client.canUndo,false);assert.deepEqual(client.bytes(),new Uint8Array([1]));
  await client.applyScopedText(plan,{});assert.equal(client.state.revision,"r2");assert.equal(client.canUndo,true);
  await client.undo();assert.equal(client.state.revision,"r1");client.close();
});

test("tampered plans, cancellation and queued revision changes invalidate scoped review",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));const plan=await client.planScopedText(scopedRequest());
  await client.previewScopedText(plan);await assert.rejects(client.applyScopedText({...plan,extra:"changed"},{}),/Preview this exact/);
  await client.cancel();await assert.rejects(client.applyScopedText(plan,{}),/Preview this exact/);
  await client.previewScopedText(plan);
  const changed=client.checkpoint({input_sha256:"r1"},{}),stale=client.applyScopedText(plan,{});
  const rejected=assert.rejects(stale,/another PDF revision/);await changed;await rejected;client.close();
});

test("a scoped no-change result creates no undo entry",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));const plan=await client.planScopedText(scopedRequest());
  await client.previewScopedText(plan);await client.applyScopedText(plan,{change:false});
  assert.equal(client.canUndo,false);assert.equal(client.state.revision,"r1");client.close();
});

test("field-wide scoped requests use the same revision, preview and history authority",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const request={operation:{kind:"scoped_text",request:{source:{scope:"widget_field",request:{target:{input_sha256:"r1",page:1,field:[12,0]},expected_value:"ABC",replacement_value:"XYZ",widgets:[]}}}}};
  const plan=await client.planScopedText(request);assert.equal(plan.requested_operation.request.source.scope,"widget_field");
  await client.previewScopedText(plan,{pages:[1,2]});await client.applyScopedText(plan,{});assert.equal(client.canUndo,true);
  await client.undo();assert.equal(client.state.revision,"r1");client.close();
});

test("causal history commands snapshot their inputs without publishing bytes or undo entries",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const base={input_sha256:"r1",paragraphs:[{id:"p",text:"before"}]};
  const pending=client.beginTextHistory(base);base.paragraphs[0].text="later";
  const result=await pending;assert.equal(result.request.base.paragraphs[0].text,"before");
  const histories=[{operations:[]}],merging=client.mergeTextHistories(base,histories);histories[0].operations.push({untrusted:"late"});
  assert.equal((await merging).request.histories[0].operations.length,0);
  assert.equal((await client.textHistoryDelta(base,{operations:[]},{a:1})).request.op,"text_history_delta");
  const style={paragraph_id:"p",replacement:{font_size:14}};
  const styling=client.styleTextHistory(base,{operations:[]},style);style.replacement.font_size=99;
  assert.equal((await styling).request.edit.replacement.font_size,14);
  const structure={paragraph_id:"p2",replacement:{present:true,position:{after:"p"}}};
  const structuring=client.structureTextHistory(base,{operations:[]},structure);structure.replacement.position.after=null;
  assert.equal((await structuring).request.edit.replacement.position.after,"p");
  const inline={paragraph_id:"p",range:[0,1],expected_text:"b",replacement:{rgb:[1,0,0]}};
  const marking=client.inlineStyleTextHistory(base,{operations:[]},inline);inline.replacement.rgb[0]=0;
  assert.deepEqual((await marking).request.edit.replacement.rgb,[1,0,0]);
  const resolution={paragraph_id:"p",targets:[{operation:null,start:0,end:1}],replacement:{clear:["rgb"]}};
  const resolving=client.resolveInlineStyleTextHistory(base,{operations:[]},resolution);resolution.targets[0].end=2;
  assert.equal((await resolving).request.resolution.targets[0].end,1);
  assert.equal((await client.setTextHistoryOperationsActive(base,{operations:[]},{targets:[]})).request.op,"text_history_set_many_active");
  assert.equal(client.state.revision,"r1");assert.equal(client.canUndo,false);assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});

test("durable history commands snapshot intent and publish checkpoints through exact byte undo",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const source={kind:"resume",input_sha256:"r1",story_id:"body",expected_checkpoint_sha256:"h1",history:{operations:[]}};
  const prepared=client.prepareHistory(source);source.history.operations.push({late:true});
  assert.equal((await prepared).request.source.history.operations.length,0);
  assert.equal((await client.resumeHistory("body")).request.op,"history_resume");
  assert.equal((await client.joinHistory(source,[])).request.op,"history_join");
  assert.equal((await client.editHistory(source,{})).request.op,"history_edit");
  assert.equal((await client.styleHistory(source,{replacement:{rgb:[0,0,0]}})).request.op,"history_style");
  assert.equal((await client.structureHistory(source,{replacement:{present:false}})).request.op,"history_structure");
  assert.equal((await client.inlineStyleHistory(source,{replacement:{font_size:14}})).request.op,"history_inline_style");
  assert.equal((await client.resolveInlineStyleHistory(source,{replacement:{clear:["font_size"]}})).request.op,"history_resolve_inline_style");
  assert.equal((await client.historyDelta(source,{})).request.op,"history_delta");
  assert.equal((await client.setHistoryOperationsActive(source,{targets:[]})).request.op,"history_set_many_active");
  assert.equal((await client.previewHistory(source)).request.op,"history_preview");
  assert.equal(client.canUndo,false);assert.equal(client.state.revision,"r1");
  const receipt={history_sha256:"reviewed"},pending=client.checkpointHistory(source,receipt);
  receipt.history_sha256="changed";source.story_id="changed";
  const saved=await pending;assert.equal(saved.report.receipt.history_sha256,"reviewed");assert.equal(saved.report.source.story_id,"body");
  assert.equal(client.state.revision,"r2");assert.equal(client.canUndo,true);
  await client.undo();assert.deepEqual(client.bytes(),new Uint8Array([1]));
  await client.redo();assert.deepEqual(client.bytes(),new Uint8Array([2]));client.close();
});

test("cancelled durable-history preview cannot publish or create an undo preimage",async()=>{
  const workers=[],client=new StoryWorkerClient({wasmModuleUrl:"https://example.invalid/sdk.js",workerFactory:()=>{const worker=new FakeWorker();workers.push(worker);return worker;}});
  await client.open(new Uint8Array([1]));workers[0].holdPreview=true;
  const pending=client.previewHistory({kind:"resume",input_sha256:"r1"}),rejected=assert.rejects(pending,{name:"AbortError"});
  await workers[0].waiting;await client.cancel();await rejected;
  assert.equal(workers[0].terminated,true);assert.deepEqual(client.bytes(),new Uint8Array([1]));assert.equal(client.canUndo,false);client.close();
});

test("history compaction snapshots its destructive approval and publishes one undoable revision",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const request={input_sha256:"r1",story_id:"body",expected_checkpoint_sha256:"checkpoint",expected_history_sha256:"history",
    acknowledged_frontier:{a:7},acknowledge_operation_and_undo_loss:true,acknowledge_prior_epoch_rejected:true};
  const planned=client.planHistoryCompaction(request);request.acknowledged_frontier.a=9;
  assert.equal((await planned).request.request.acknowledged_frontier.a,7);
  assert.throws(()=>client.compactHistory(request,"bad"),/plan hash/);
  const approved="a".repeat(64),pending=client.compactHistory(request,approved);request.story_id="changed";
  const result=await pending;assert.equal(result.report.request.story_id,"body");assert.equal(result.report.approved_plan_sha256,approved);
  assert.equal(client.state.revision,"r2");assert.equal(client.canUndo,true);
  await client.undo();assert.equal(client.state.revision,"r1");client.close();
});

test("selective activity snapshots the exact target and does not roll back the published PDF",async()=>{
  const client=newClient();await client.open(new Uint8Array([1]));
  const change={actor:"a",target:{actor:"a",sequence:1},expected_active:true,active:false,expected_history_sha256:"exact"};
  const pending=client.setHistoryOperationActive({kind:"resume",input_sha256:"r1"},change);
  change.target.sequence=9;change.active=true;
  const result=await pending;assert.equal(result.request.op,"history_set_active");
  assert.equal(result.request.change.target.sequence,1);assert.equal(result.request.change.active,false);
  assert.equal((await client.setTextHistoryOperationActive({input_sha256:"r1"},{operations:[]},change)).request.op,"text_history_set_active");
  assert.equal(client.state.revision,"r1");assert.equal(client.canUndo,false);assert.deepEqual(client.bytes(),new Uint8Array([1]));client.close();
});
