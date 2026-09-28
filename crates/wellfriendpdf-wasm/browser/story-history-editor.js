/** Offline logical text collaboration. Uses the shared native session; no
 * server, identity authority, PDF byte merge or automatic checkpoint. */
export class WellfriendStoryHistoryEditor extends HTMLElement {
  #client; #base; #source; #result; #adopted; #busy=false; #version=0; #inactiveKeys=new Set();
  #typingTimer; #typingCommitted; #typingPending=false; #typingTooLarge=false; #composing=false;
  #checkpointBefore; #savedHistorySha; #compaction;
  #revision(){return this.#source?.kind==="start"?this.#source.base.input_sha256:this.#source?.input_sha256;}
  #changed=()=>{this.#version++;this.#clearCompaction();if(this.#source&&this.#revision()!==this.#client?.state?.revision){clearTimeout(this.#typingTimer);this.#typingTimer=undefined;this.#status("PDF revision changed. Resume its verified saved history to continue; local operations and any unrecorded typing draft remain available.");}this.#buttons();};
  constructor(){
    super();this.attachShadow({mode:"open"}).innerHTML=`<style>
      :host{display:block;font:14px system-ui}label{display:block;margin:8px 0}textarea,select,input{font:inherit;max-width:100%}textarea,select{width:100%}textarea{min-height:100px}button{font:inherit;margin:3px;padding:6px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:180px;overflow:auto}.notice{font-size:12px} :focus-visible{outline:3px solid #3668ce}
    </style><details><summary>Offline logical text history</summary>
      <p class="notice">Stable text identities retain concurrent insertions and remove only observed characters. Paragraph, paragraph-list and inline formatting registers preserve explicit conflicts, but they do not merge arbitrary layout/source ownership or provide redaction. Review wording and formatting; convergence does not mean semantic agreement. Replica IDs are not authenticated. Exported history retains deleted text and the base story's supplied font assets.</p>
      <label>Replica ID <input id="actor" maxlength="128" spellcheck="false"></label>
      <label><input id="replace-epoch" type="checkbox"> When beginning a NEW epoch, explicitly replace any saved epoch (old historical PDF revisions remain)</label>
      <label>Paragraph <select id="paragraph"></select></label>
      <label>Select text or an insertion position<textarea id="text" readonly spellcheck="false"></textarea></label>
      <label>Replacement<textarea id="replacement" spellcheck="false"></textarea></label>
      <button id="edit">Record text edit</button><button id="import">Merge history file</button>
      <details><summary>Automatic typing groups</summary>
        <p class="notice">Edit the complete paragraph below. Consecutive keystrokes are collapsed after 750 ms of inactivity into one grapheme-safe history operation, so selective undo treats the burst atomically. Flush or discard the pending burst before switching paragraphs or changing history.</p>
        <label>Live paragraph draft<textarea id="typing" spellcheck="true"></textarea></label>
        <button id="flush-typing">Record pending typing group</button><button id="discard-typing">Discard pending typing</button>
      </details>
      <details><summary>Causal paragraph formatting</summary>
        <p class="notice">Enter a typed paragraph-style patch as JSON, for example {"font_size":14,"rgb":[0.1,0.2,0.3]}. Disjoint concurrent fields merge. Different concurrent values for one field suppress publication until this exact conflict set is reviewed and replaced.</p>
        <label>Style patch JSON<textarea id="style-patch" spellcheck="false">{}</textarea></label>
        <button id="style">Record style change / resolve this paragraph's conflicts</button>
      </details>
      <details><summary>Causal inline rich text</summary>
        <p class="notice">Select complete graphemes in the read-only paragraph, then apply an atom-bound patch such as {"font_size":14,"rgb":[0.1,0.2,0.3]}. Use {"clear":["font_size"]} to restore paragraph inheritance. Concurrent unequal values are retained as explicit per-atom conflicts. Supported horizontal/vertical stories and table cells use native styled-run preview/checkpoint.</p>
        <label>Inline patch JSON<textarea id="inline-patch" spellcheck="false">{"font_size":14}</textarea></label>
        <button id="inline-style">Record inline style for selected text</button>
        <label>Conflict field <select id="inline-conflict-field"></select></label>
        <label>Resolution patch JSON<textarea id="inline-resolution" spellcheck="false">{"font_size":14}</textarea></label>
        <button id="resolve-inline">Resolve every conflicting atom for this field</button>
        <pre id="inline-report">No inline styles or conflicts.</pre>
      </details>
      <details><summary>Causal paragraph list</summary>
        <p class="notice">Insert, move or delete a stable paragraph ID. Concurrent incompatible positions and delete-versus-content edits suppress publication until every conflicting field for the selected paragraph is resolved.</p>
        <label>Position after <select id="structure-after"></select></label>
        <button id="move-paragraph">Move selected paragraph</button><button id="delete-paragraph">Delete selected paragraph</button>
        <label>New paragraph ID <input id="new-paragraph-id" maxlength="1024" spellcheck="false"></label>
        <label>New paragraph text<textarea id="new-paragraph-text" spellcheck="true"></textarea></label>
        <button id="insert-paragraph">Insert new paragraph</button>
        <label>Conflict presence decision <select id="structure-present"><option value="true">Keep / restore</option><option value="false">Delete</option></select></label>
        <button id="resolve-structure">Resolve all selected paragraph structure conflicts</button>
      </details>
      <details><summary>Selective undo / redo of this replica's text, style and structure edits</summary>
        <p class="notice">This records a new causal event, not a PDF rollback. Other edits remain, including text anchored inside an undone insertion. Restored wording may need review. Replica IDs must be provisioned and authorized by the host; typing an ID is not authentication.</p>
        <label>Recent own edits in this paragraph (latest 200)<select id="operation-options"></select></label>
        <label>Exact own edit sequence (older edits may be entered)<input id="operation-sequence" type="number" min="1" max="9007199254740991" step="1"></label>
        <button id="undo-edit">Undo this edit</button><button id="redo-edit">Redo this edit</button>
        <label>Atomic edit group (comma/space-separated own sequences)<input id="operation-group" spellcheck="false" placeholder="12, 13, 14"></label>
        <button id="undo-group">Undo complete group</button><button id="redo-group">Redo complete group</button>
      </details>
      <details><summary>Start a compact replacement epoch</summary>
        <p class="notice">This is destructive logical-history compaction for the currently saved, fully acknowledged epoch. It does not shrink or sanitize historical PDF bytes. Export anything you need first. Every prior-epoch replica must be retired; it cannot later rejoin the replacement epoch.</p>
        <label><input id="compact-loss" type="checkbox"> I accept permanent loss of operation history and selective undo in the new epoch</label>
        <label><input id="compact-retire" type="checkbox"> I confirm the displayed frontier is complete and all prior-epoch replicas will be retired</label>
        <button id="plan-compaction">Inspect exact compaction plan</button>
        <pre id="compaction-report">No compaction plan.</pre>
        <label><input id="review-compaction" type="checkbox"> I reviewed this exact plan hash and approve the new epoch</label>
        <button id="apply-compaction">Apply reviewed compaction</button>
      </details>
      <input id="file" type="file" accept="application/json,.json" hidden>
      <button id="export">Export history package</button>
      <label><input id="review" type="checkbox"> I reviewed the combined wording; use it as an unsaved story draft</label>
      <button id="adopt">Use reviewed draft</button><pre id="report"></pre>
      <p id="status" role="status" aria-live="polite">Start from a story draft using the host editor.</p>
    </details>`;
    this.#q("#actor").value=crypto.randomUUID();
    this.#q("#paragraph").addEventListener("change",()=>this.#select());
    this.#q("#review").addEventListener("change",()=>this.#buttons());
    this.#q("#edit").addEventListener("click",()=>this.#run(()=>this.#edit()));
    this.#q("#actor").addEventListener("input",()=>{this.#operationOptions();this.#buttons();});
    this.#q("#operation-options").addEventListener("change",()=>{this.#q("#operation-sequence").value=this.#q("#operation-options").value;this.#buttons();});
    this.#q("#operation-sequence").addEventListener("input",()=>this.#buttons());
    this.#q("#operation-group").addEventListener("input",()=>this.#buttons());
    this.#q("#typing").addEventListener("input",()=>this.#typingInput());
    this.#q("#typing").addEventListener("compositionstart",()=>{this.#composing=true;clearTimeout(this.#typingTimer);this.#typingTimer=undefined;this.#buttons();});
    this.#q("#typing").addEventListener("compositionend",()=>{this.#composing=false;this.#typingInput();});
    this.#q("#typing").addEventListener("blur",()=>{if(this.#typingPending&&!this.#composing)this.#scheduleTyping(0);});
    this.#q("#flush-typing").addEventListener("click",()=>this.#run(()=>this.#flushTyping()));
    this.#q("#discard-typing").addEventListener("click",()=>this.#discardTyping());
    this.#q("#style").addEventListener("click",()=>this.#run(()=>this.#style()));
    this.#q("#inline-style").addEventListener("click",()=>this.#run(()=>this.#inlineStyle()));
    this.#q("#resolve-inline").addEventListener("click",()=>this.#run(()=>this.#resolveInlineStyle()));
    this.#q("#inline-conflict-field").addEventListener("change",()=>this.#inlineReport());
    this.#q("#insert-paragraph").addEventListener("click",()=>this.#run(()=>this.#structure("insert")));
    this.#q("#move-paragraph").addEventListener("click",()=>this.#run(()=>this.#structure("move")));
    this.#q("#delete-paragraph").addEventListener("click",()=>this.#run(()=>this.#structure("delete")));
    this.#q("#resolve-structure").addEventListener("click",()=>this.#run(()=>this.#structure("resolve")));
    this.#q("#undo-edit").addEventListener("click",()=>this.#run(()=>this.#setActive(false)));
    this.#q("#redo-edit").addEventListener("click",()=>this.#run(()=>this.#setActive(true)));
    this.#q("#undo-group").addEventListener("click",()=>this.#run(()=>this.#setManyActive(false)));
    this.#q("#redo-group").addEventListener("click",()=>this.#run(()=>this.#setManyActive(true)));
    this.#q("#compact-loss").addEventListener("change",()=>{this.#clearCompaction();this.#buttons();});
    this.#q("#compact-retire").addEventListener("change",()=>{this.#clearCompaction();this.#buttons();});
    this.#q("#review-compaction").addEventListener("change",()=>this.#buttons());
    this.#q("#plan-compaction").addEventListener("click",()=>this.#run(()=>this.#planCompaction()));
    this.#q("#apply-compaction").addEventListener("click",()=>this.#run(()=>this.#applyCompaction()));
    this.#q("#import").addEventListener("click",()=>{this.#q("#file").value="";this.#q("#file").click();});
    this.#q("#file").addEventListener("change",()=>this.#run(()=>this.#import()));
    this.#q("#export").addEventListener("click",()=>this.#run(()=>this.#export()));
    this.#q("#adopt").addEventListener("click",()=>this.#run(()=>this.#adopt()));this.#buttons();
  }
  #q(selector){return this.shadowRoot.querySelector(selector);}
  #status(text){this.#q("#status").textContent=text;}
  connectedCallback(){this.#client?.addEventListener("change",this.#changed);this.#buttons();}
  disconnectedCallback(){this.#client?.removeEventListener("change",this.#changed);clearTimeout(this.#typingTimer);this.#typingTimer=undefined;this.#version++;}
  set client(value){if(this.#busy)throw new Error("History editor is busy");this.#client?.removeEventListener("change",this.#changed);this.#client=value;if(this.isConnected)value?.addEventListener("change",this.#changed);this.#changed();}
  get client(){return this.#client;}
  async begin(base){
    if(this.#busy)throw new Error("History editor is busy");
    if(this.#result?.history.operations.length&&!globalThis.confirm("Replace this local collaboration epoch? Export its history first if you need to retain it. The PDF is unchanged."))return;
    await this.#run(async()=>{const owned=structuredClone(base),version=this.#version;const empty=await this.#client.beginTextHistory(owned);
      const prepared=await this.#client.prepareHistory({kind:"start",base:owned,history:empty.history,replace_epoch:this.#q("#replace-epoch").checked});
      if(version!==this.#version)throw new DOMException("History base changed","AbortError");
      this.#adopted=structuredClone(owned);this.#publish(prepared);this.#q("#replace-epoch").checked=false;this.#q("details").open=true;});
  }
  async resume(storyId,expectedDraft){
    await this.#run(async()=>{const version=this.#version;let prepared=await this.#client.resumeHistory(storyId),savedHistorySha=prepared.result.history_sha256;
      if(this.#result?.history.operations.length){
        const same=this.#result.history.base_revision_sha256===prepared.result.history.base_revision_sha256&&this.#result.history.base_story_sha256===prepared.result.history.base_story_sha256;
        if(same)prepared=await this.#client.joinHistory(prepared.source,[this.#result.history]);
        else if(!globalThis.confirm("Switch to a different saved history epoch? Export local operations first if needed."))return;
      }
      if(version!==this.#version)return;this.#adopted=expectedDraft===undefined?undefined:structuredClone(expectedDraft);this.#publish(prepared,savedHistorySha);this.#q("details").open=true;
    });
  }
  #buttons(){const same=!!this.#source&&this.#revision()===this.#client?.state?.revision;
    for(const field of this.shadowRoot.querySelectorAll("button,input,select,textarea"))field.disabled=this.#busy||!same;
    this.#q("#export").disabled=this.#busy||!this.#result;
    this.#q("#import").disabled=this.#busy||!this.#client?.state||!!this.#source&&!same;
    this.#q("#replace-epoch").disabled=this.#busy||!this.#client?.state;
    this.#q("#file").disabled=this.#q("#import").disabled;
    const selectedId=this.#q("#paragraph").value,selectedVisible=!!this.#result?.merged?.paragraphs?.some(paragraph=>paragraph.id===selectedId);
    this.#q("#edit").disabled||=!selectedVisible;
    this.#q("#adopt").disabled||=!this.#result?.merged||!this.#q("#review").checked;
    const typing=this.#typingPending;
    this.#q("#flush-typing").disabled=this.#busy||!same||!typing||this.#typingTooLarge||this.#composing||!selectedVisible;
    this.#q("#discard-typing").disabled=this.#busy||!typing;
    this.#q("#paragraph").disabled||=typing;
    this.#q("#actor").disabled||=typing;this.#q("#replace-epoch").disabled||=typing;
    this.#q("#edit").disabled||=typing;this.#q("#replacement").disabled||=typing;
    this.#q("#typing").disabled||=!selectedVisible;
    const styleConflicts=this.#result?.style_conflicts??[],hasStyleConflict=styleConflicts.some(conflict=>conflict.paragraph_id===this.#q("#paragraph").value);
    this.#q("#style").disabled=this.#busy||!same||typing||!this.#result||!!this.#result.missing_dependencies?.length||!selectedId||!selectedVisible&&!hasStyleConflict;
    this.#q("#style-patch").disabled=this.#q("#style").disabled;
    const inlineConflicts=this.#result?.inline_conflicts??[],hasInlineConflict=inlineConflicts.some(conflict=>conflict.paragraph_id===selectedId);
    this.#q("#inline-style").disabled=this.#busy||!same||typing||!this.#result?.merged||!selectedVisible||!!inlineConflicts.length;
    this.#q("#inline-patch").disabled=this.#q("#inline-style").disabled;
    this.#q("#resolve-inline").disabled=this.#busy||!same||typing||!hasInlineConflict||!!this.#result?.missing_dependencies?.length;
    this.#q("#inline-conflict-field").disabled=this.#q("#resolve-inline").disabled;
    this.#q("#inline-resolution").disabled=this.#q("#resolve-inline").disabled;
    const structureConflicts=this.#result?.structure_conflicts??[],hasStructureConflict=structureConflicts.some(conflict=>conflict.paragraph_id===this.#q("#paragraph").value);
    this.#q("#insert-paragraph").disabled=this.#busy||!same||typing||!this.#result?.merged;
    this.#q("#move-paragraph").disabled=this.#busy||!same||typing||!selectedVisible;
    this.#q("#delete-paragraph").disabled=this.#q("#move-paragraph").disabled;
    this.#q("#resolve-structure").disabled=this.#busy||!same||typing||!hasStructureConflict||!!this.#result?.missing_dependencies?.length;
    for(const id of ["#structure-after","#structure-present","#new-paragraph-id","#new-paragraph-text"])this.#q(id).disabled=this.#busy||!same||typing;
    this.#q("#import").disabled||=typing;this.#q("#adopt").disabled||=typing;
    const operation=this.#ownOperation(),active=operation&&!this.#inactive(operation.id);
    const complete=!!this.#result&&!this.#result.missing_dependencies?.length;
    this.#q("#undo-edit").disabled||=!complete||!operation||!active;
    this.#q("#redo-edit").disabled||=!complete||!operation||!!active;
    const group=this.#ownOperations(),states=group?.map(operation=>!this.#inactive(operation.id));
    this.#q("#undo-group").disabled||=!complete||!group||!states.every(Boolean);
    this.#q("#redo-group").disabled||=!complete||!group||!states.every(state=>!state);
    for(const id of ["#undo-edit","#redo-edit","#undo-group","#redo-group"])this.#q(id).disabled||=typing;
    const resumable=same&&this.#source?.kind==="resume"&&!!this.#checkpointBefore&&this.#result?.history_sha256===this.#savedHistorySha&&!!this.#result?.history?.operations?.length;
    this.#q("#plan-compaction").disabled=this.#busy||typing||!resumable||!this.#q("#compact-loss").checked||!this.#q("#compact-retire").checked;
    this.#q("#review-compaction").disabled=this.#busy||!this.#compaction;
    this.#q("#apply-compaction").disabled=this.#busy||!this.#compaction||!this.#q("#review-compaction").checked;
  }
  async #run(action){if(this.#busy)return;this.#busy=true;this.#buttons();try{await action();}catch(error){if(error.name!=="AbortError")this.#status(error.message??String(error));}finally{this.#busy=false;this.#buttons();}}
  #publish(prepared,savedHistorySha){this.#clearCompaction();this.#source=prepared.source;this.#base=prepared.source.kind==="start"?prepared.source.base:undefined;this.#checkpointBefore=prepared.checkpoint_before;const result=prepared.result;if(typeof savedHistorySha==="string")this.#savedHistorySha=savedHistorySha;else if(prepared.source.kind!=="resume")this.#savedHistorySha=undefined;this.#result=result;this.#inactiveKeys=new Set((result.inactive_operations??[]).map(id=>JSON.stringify([id.actor,id.sequence])));this.#q("#review").checked=false;
    const select=this.#q("#paragraph"),previous=select.value;select.replaceChildren();
    const visibleIds=result.merged?.paragraphs?.map(paragraph=>paragraph.id)??[],visibleSet=new Set(visibleIds),allIds=result.paragraph_ids??[...new Set([...(result.style_conflicts??[]).map(conflict=>conflict.paragraph_id),...(result.structure_conflicts??[]).map(conflict=>conflict.paragraph_id),...(result.inline_conflicts??[]).map(conflict=>conflict.paragraph_id)])],paragraphIds=[...visibleIds,...allIds.filter(id=>!visibleSet.has(id))];
    for(const paragraphId of paragraphIds)select.add(new Option(paragraphId,paragraphId));
    if([...select.options].some(option=>option.value===previous))select.value=previous;
    const after=this.#q("#structure-after"),oldAfter=after.value;after.replaceChildren(new Option("Start of story",""));
    for(const paragraphId of paragraphIds)after.add(new Option(paragraphId,paragraphId));
    if([...after.options].some(option=>option.value===oldAfter))after.value=oldAfter;this.#select();
    this.#q("#report").textContent=JSON.stringify({checkpoint_before:prepared.checkpoint_before,generation_before:prepared.generation_before,history_sha256:result.history_sha256,operations:result.history.operations.length,frontier:result.frontier,
      missing_dependencies:result.missing_dependencies,paragraph_ids:result.paragraph_ids,atoms:result.atom_count,tombstones:result.tombstone_count,
      inactive_operations:result.inactive_operations,suppressed_atoms:result.suppressed_atom_count,
      style_conflicts_sha256:result.style_conflicts_sha256,style_conflicts:result.style_conflicts,
      structure_conflicts_sha256:result.structure_conflicts_sha256,structure_conflicts:result.structure_conflicts,
      inline_conflicts_sha256:result.inline_conflicts_sha256,inline_style_runs:result.inline_style_runs,inline_conflicts:result.inline_conflicts,limits:result.limits},null,2);
    this.#status(result.inline_conflicts?.length?"Concurrent inline formatting conflicts require an exact per-field resolution over every conflicting atom.":result.merged?"Projection ready for review. PDF bytes and undo history are unchanged.":result.structure_conflicts?.length?"Paragraph presence/order conflicts require an exact resolution for every conflicting field in one selected paragraph.":result.style_conflicts?.length?"Concurrent paragraph formatting conflicts require an explicit replacement for every conflicting field in one paragraph.":"Waiting for missing dependencies. Imported operations are retained; no partial draft can be adopted.");
  }
  #clearCompaction(){this.#compaction=undefined;const report=this.shadowRoot?.querySelector("#compaction-report"),review=this.shadowRoot?.querySelector("#review-compaction");if(report)report.textContent="No compaction plan.";if(review)review.checked=false;}
  #compactionRequest(){if(this.#source?.kind!=="resume"||!this.#checkpointBefore||!this.#result?.history?.operations?.length)throw new Error("Resume a saved non-empty causal history before compaction");
    return {input_sha256:this.#client.state.revision,story_id:this.#source.story_id,expected_checkpoint_sha256:this.#checkpointBefore,
      expected_history_sha256:this.#result.history_sha256,acknowledged_frontier:structuredClone(this.#result.frontier),
      acknowledge_operation_and_undo_loss:this.#q("#compact-loss").checked,acknowledge_prior_epoch_rejected:this.#q("#compact-retire").checked};}
  async #planCompaction(){if(this.#typingPending)throw new Error("Flush or discard pending typing before compaction");const request=this.#compactionRequest(),version=this.#version,plan=await this.#client.planHistoryCompaction(request);
    if(version!==this.#version)throw new DOMException("History revision changed","AbortError");this.#compaction={request,plan};this.#q("#review-compaction").checked=false;this.#q("#compaction-report").textContent=JSON.stringify(plan,null,2);this.#status("Compaction plan ready. Review its exact frontier, retired counts, limits and plan hash before approval.");}
  async #applyCompaction(){if(!this.#compaction||!this.#q("#review-compaction").checked)throw new Error("Review the exact compaction plan first");
    const {request,plan}=this.#compaction,storyId=request.story_id,adopted=this.#adopted;const saved=await this.#client.compactHistory(request,plan.plan_sha256),prepared=await this.#client.resumeHistory(storyId);
    this.#adopted=adopted;this.#publish(prepared,prepared.result.history_sha256);this.#q("#compact-loss").checked=false;this.#q("#compact-retire").checked=false;
    this.#status(`New empty history epoch saved at generation ${saved.report.generation_after}. ${saved.report.exact_session_undo_available?"Exact byte undo is available in this session.":"The prior PDF exceeded the session undo budget; exact byte undo was not retained."}`);}
  #select(){const paragraphId=this.#q("#paragraph").value,text=this.#result?.merged?.paragraphs.find(p=>p.id===paragraphId)?.text??"";this.#q("#text").value=text;this.#q("#replacement").value="";
    if(!this.#typingPending){const field=this.#q("#typing");field.value=text;this.#typingTooLarge=false;this.#typingCommitted={paragraphId,text,normalized:field.value,historySha256:this.#result?.history_sha256};}
    this.#operationOptions();this.#inlineConflictOptions();this.#inlineReport();this.#buttons();}
  #typingInput(){if(!this.#typingCommitted||!this.#result?.merged)return;const value=this.#q("#typing").value;this.#typingPending=value!==this.#typingCommitted.normalized;
    const limit=4*1024*1024;this.#typingTooLarge=value.length>limit||value.length>Math.floor(limit/3)&&new TextEncoder().encode(value).length>limit;
    if(this.#typingPending&&!this.#typingTooLarge&&!this.#composing)this.#scheduleTyping(750);else{clearTimeout(this.#typingTimer);this.#typingTimer=undefined;}
    if(this.#typingTooLarge)this.#status("Typing draft exceeds the 4 MiB paragraph/edit budget; shorten or discard it before recording.");this.#buttons();}
  #scheduleTyping(delay){clearTimeout(this.#typingTimer);this.#typingTimer=setTimeout(()=>{this.#typingTimer=undefined;this.#run(()=>this.#flushTyping());},delay);}
  #discardTyping(){clearTimeout(this.#typingTimer);this.#typingTimer=undefined;if(this.#typingCommitted)this.#q("#typing").value=this.#typingCommitted.normalized;this.#typingPending=false;this.#typingTooLarge=false;this.#buttons();this.#status("Pending typing group discarded; history and PDF bytes are unchanged.");}
  async #flushTyping(){clearTimeout(this.#typingTimer);this.#typingTimer=undefined;if(!this.#typingPending)return;
    if(this.#typingTooLarge)throw new Error("Typing draft exceeds the 4 MiB paragraph/edit budget");
    const committed=this.#typingCommitted,draft=this.#q("#typing").value;
    if(!committed||committed.paragraphId!==this.#q("#paragraph").value||committed.historySha256!==this.#result?.history_sha256)throw new Error("Typing base changed; discard the pending draft and resume from the verified history");
    const {coalescedTextEdit}=await import("./story-client.js"),change=coalescedTextEdit(committed.text,committed.normalized,draft);
    if(!change){this.#typingPending=false;this.#select();return;}
    const version=this.#version,prepared=await this.#client.editHistory(this.#source,{actor:this.#q("#actor").value,paragraph_id:committed.paragraphId,...change,expected_history_sha256:committed.historySha256});
    if(version===this.#version){this.#typingPending=false;this.#publish(prepared);this.#status("Typing burst recorded as one atomic text-history operation. Review the complete projection before checkpointing.");}
  }
  #inactive(id){return this.#inactiveKeys.has(JSON.stringify([id.actor,id.sequence]));}
  #ownOperation(){const sequence=Number(this.#q("#operation-sequence").value);if(!Number.isSafeInteger(sequence)||sequence<1)return;
    return this.#result?.history.operations.find(op=>!op.visibility&&op.id.actor===this.#q("#actor").value&&op.id.sequence===sequence&&op.paragraph_id===this.#q("#paragraph").value);}
  #ownOperations(){const values=this.#q("#operation-group").value.trim().split(/[\s,]+/).filter(Boolean).map(Number);if(!values.length||values.length>4096||values.some(value=>!Number.isSafeInteger(value)||value<1)||new Set(values).size!==values.length)return;
    const actor=this.#q("#actor").value,operations=values.map(sequence=>this.#result?.history.operations.find(op=>!op.visibility&&op.id.actor===actor&&op.id.sequence===sequence));
    return operations.every(Boolean)?operations:undefined;}
  #operationOptions(){const select=this.#q("#operation-options"),previous=this.#q("#operation-sequence").value;
    select.replaceChildren(new Option("Choose an edit or enter its sequence", ""));
    const operations=(this.#result?.history.operations??[]).filter(op=>!op.visibility&&op.id.actor===this.#q("#actor").value&&op.paragraph_id===this.#q("#paragraph").value).slice(-200).reverse();
    for(const op of operations){const effect=op.paragraph_structure?`structure fields ${Object.keys(op.paragraph_structure).join(", ")}`:op.paragraph_style?`paragraph style fields ${Object.keys(op.paragraph_style).join(", ")}`:op.inline_style?`inline style fields ${Object.keys(op.inline_style.patch).join(", ")}; ${op.inline_style.targets.length} atom ranges`:`inserted ${Array.from(op.inserted).length} characters; ${op.removed.length} deletion ranges`;
      select.add(new Option(`${op.id.sequence}: ${this.#inactive(op.id)?"undone":"active"}; ${effect}`,String(op.id.sequence)));}
    if(operations.some(op=>String(op.id.sequence)===previous))select.value=previous;
  }
  async #setActive(active){const operation=this.#ownOperation();if(!operation||!this.#result||this.#result.missing_dependencies?.length)throw new Error("Select an original text/style/structure edit belonging to this replica in the current paragraph");
    const version=this.#version,prepared=await this.#client.setHistoryOperationActive(this.#source,{actor:this.#q("#actor").value,target:operation.id,
      expected_active:!this.#inactive(operation.id),active,expected_history_sha256:this.#result.history_sha256});
    if(version===this.#version)this.#publish(prepared);
  }
  async #setManyActive(active){const operations=this.#ownOperations();if(!operations||!this.#result||this.#result.missing_dependencies?.length)throw new Error("Enter 1..=4096 unique original text/style/structure edit sequences belonging to this replica");
    const states=operations.map(operation=>!this.#inactive(operation.id));if(!states.every(state=>state===states[0]))throw new Error("Every edit in an atomic group must have the same current activity");
    const version=this.#version,prepared=await this.#client.setHistoryOperationsActive(this.#source,{actor:this.#q("#actor").value,targets:operations.map(operation=>operation.id),
      expected_active:states[0],active,expected_history_sha256:this.#result.history_sha256});
    if(version===this.#version)this.#publish(prepared);
  }
  async #style(){const paragraphId=this.#q("#paragraph").value;if(!paragraphId)throw new Error("Choose a paragraph");let replacement;
    try{replacement=JSON.parse(this.#q("#style-patch").value);}catch{throw new Error("Style patch must be valid JSON");}
    if(!replacement||Array.isArray(replacement)||typeof replacement!=="object"||!Object.keys(replacement).length)throw new Error("Style patch must be a non-empty JSON object");
    const conflicts=(this.#result.style_conflicts??[]).filter(conflict=>conflict.paragraph_id===paragraphId),expected={};
    if(conflicts.length){const required=new Set(conflicts.map(conflict=>conflict.field)),supplied=new Set(Object.keys(replacement));
      if(required.size!==supplied.size||[...required].some(field=>!supplied.has(field)))throw new Error(`Resolve exactly these conflicting fields: ${[...required].join(", ")}`);
    }else{const paragraph=this.#result.merged?.paragraphs.find(candidate=>candidate.id===paragraphId);if(!paragraph)throw new Error("Paragraph projection is unavailable");
      const defaults={rgb:[0,0,0],rtl:false,keep_with_next:false,keep_together:false,break_before:false,page_break_before:"none",orphans:2,widows:2,space_before:0,space_after:0,shaping:{},line_break:{}};
      for(const field of Object.keys(replacement)){if(field in paragraph)expected[field]=structuredClone(paragraph[field]);else if(field in defaults)expected[field]=structuredClone(defaults[field]);else throw new Error(`Unknown paragraph-style field: ${field}`);}}
    const version=this.#version,prepared=await this.#client.styleHistory(this.#source,{expected_history_sha256:this.#result.history_sha256,
      expected_style_conflicts_sha256:this.#result.style_conflicts_sha256,actor:this.#q("#actor").value,paragraph_id:paragraphId,expected,replacement});
    if(version===this.#version){this.#publish(prepared);this.#status(prepared.result.merged?"Causal paragraph style recorded. Review native reflow before checkpointing.":"Style operation recorded; remaining conflicts still require explicit resolution.");}
  }
  #inlineConflictOptions(){const select=this.#q("#inline-conflict-field"),previous=select.value,paragraphId=this.#q("#paragraph").value,fields=[...new Set((this.#result?.inline_conflicts??[]).filter(conflict=>conflict.paragraph_id===paragraphId).map(conflict=>conflict.field))];select.replaceChildren(new Option(fields.length?"Choose a conflicting field":"No inline conflict", ""));for(const field of fields)select.add(new Option(field,field));if(fields.includes(previous))select.value=previous;else if(fields.length===1)select.value=fields[0];}
  #inlineReport(){const paragraphId=this.#q("#paragraph").value,field=this.#q("#inline-conflict-field").value,runs=(this.#result?.inline_style_runs??[]).filter(run=>run.paragraph_id===paragraphId),conflicts=(this.#result?.inline_conflicts??[]).filter(conflict=>conflict.paragraph_id===paragraphId&&(!field||conflict.field===field));this.#q("#inline-report").textContent=runs.length||conflicts.length?JSON.stringify({runs,conflicts},null,2):"No inline styles or conflicts.";}
  async #inlineSelection(){const paragraphId=this.#q("#paragraph").value,text=this.#result?.merged?.paragraphs.find(paragraph=>paragraph.id===paragraphId)?.text;if(text===undefined)throw new Error("Choose a visible paragraph");const field=this.#q("#text"),{sourceSelectionRange}=await import("./story-client.js"),[start,end]=sourceSelectionRange(text,field.selectionStart,field.selectionEnd),scalars=Array.from(text),encoder=new TextEncoder(),range=[encoder.encode(scalars.slice(0,start).join("")).length,encoder.encode(scalars.slice(0,end).join("")).length];if(range[0]===range[1])throw new Error("Select one or more complete graphemes to style");return {paragraphId,range,expected_text:scalars.slice(start,end).join("")};}
  #inlinePatch(id){let patch;try{patch=JSON.parse(this.#q(id).value);}catch{throw new Error("Inline style patch must be valid JSON");}if(!patch||Array.isArray(patch)||typeof patch!=="object"||!Object.keys(patch).length)throw new Error("Inline style patch must be a non-empty JSON object");return patch;}
  async #inlineStyle(){if(!this.#result||this.#result.missing_dependencies?.length||this.#result.inline_conflicts?.length)throw new Error("Receive dependencies and resolve inline conflicts before adding another mark");const selection=await this.#inlineSelection(),replacement=this.#inlinePatch("#inline-patch"),version=this.#version,prepared=await this.#client.inlineStyleHistory(this.#source,{expected_history_sha256:this.#result.history_sha256,expected_inline_conflicts_sha256:this.#result.inline_conflicts_sha256,actor:this.#q("#actor").value,paragraph_id:selection.paragraphId,range:selection.range,expected_text:selection.expected_text,replacement});if(version===this.#version){this.#publish(prepared);this.#status("Causal inline style recorded. Review the native styled-run preview before checkpointing.");}}
  async #resolveInlineStyle(){if(!this.#result||this.#result.missing_dependencies?.length)throw new Error("Receive the complete history before resolving inline formatting");const paragraphId=this.#q("#paragraph").value,field=this.#q("#inline-conflict-field").value;if(!paragraphId||!field)throw new Error("Choose a paragraph and conflicting inline field");const conflicts=this.#result.inline_conflicts.filter(conflict=>conflict.paragraph_id===paragraphId&&conflict.field===field);if(!conflicts.length)throw new Error("The selected inline conflict is no longer current");const replacement=this.#inlinePatch("#inline-resolution"),direct=Object.prototype.hasOwnProperty.call(replacement,field),cleared=Array.isArray(replacement.clear)&&replacement.clear.length===1&&replacement.clear[0]===field;if(Object.keys(replacement).some(key=>key!==field&&key!=="clear")||direct===cleared)throw new Error(`Resolve exactly the ${field} field by assigning it or clearing it`);const targets=conflicts.map(conflict=>({operation:structuredClone(conflict.target.operation),start:conflict.target.offset,end:conflict.target.offset+1})),version=this.#version,prepared=await this.#client.resolveInlineStyleHistory(this.#source,{expected_history_sha256:this.#result.history_sha256,expected_inline_conflicts_sha256:this.#result.inline_conflicts_sha256,actor:this.#q("#actor").value,paragraph_id:paragraphId,targets,replacement});if(version===this.#version){this.#publish(prepared);this.#status(prepared.result.merged?"Inline conflict resolved. Review the complete logical draft.":"Inline resolution recorded; other concurrent conflicts remain.");}}
  async #structure(kind){
    if(!this.#result||this.#result.missing_dependencies?.length)throw new Error("Receive the complete history before changing paragraph structure");
    const actor=this.#q("#actor").value,selected=this.#q("#paragraph").value,after=this.#q("#structure-after").value||null;
    let paragraphId=selected,expected_absent=false,expected={},replacement={};
    if(kind==="insert"){
      paragraphId=this.#q("#new-paragraph-id").value.trim();if(!paragraphId)throw new Error("Enter a new stable paragraph ID");
      if(this.#result.history.operations.some(op=>op.paragraph_id===paragraphId)||this.#result.merged?.paragraphs.some(paragraph=>paragraph.id===paragraphId))throw new Error("Paragraph ID already exists in this epoch");
      const template=this.#result.merged?.paragraphs.find(paragraph=>paragraph.id===selected)??this.#result.merged?.paragraphs[0];if(!template)throw new Error("A visible paragraph is required as the style template");
      const paragraph=structuredClone(template);paragraph.id=paragraphId;paragraph.text=this.#q("#new-paragraph-text").value;
      expected_absent=true;replacement={present:true,position:{after},inserted_paragraph:paragraph};
    }else if(kind==="resolve"){
      if(!selected)throw new Error("Choose a conflicted paragraph");
      const fields=new Set((this.#result.structure_conflicts??[]).filter(conflict=>conflict.paragraph_id===selected).map(conflict=>conflict.field));
      if(!fields.size)throw new Error("Selected paragraph has no structure conflict");
      if(fields.has("present"))replacement.present=this.#q("#structure-present").value==="true";
      if(fields.has("position")){if(after===selected)throw new Error("A paragraph cannot follow itself");replacement.position={after};}
    }else{
      if(!selected)throw new Error("Choose a paragraph");
      const paragraphs=this.#result.merged?.paragraphs;if(!paragraphs)throw new Error("Resolve current conflicts before another structure edit");
      const index=paragraphs.findIndex(paragraph=>paragraph.id===selected);if(index<0)throw new Error("Selected paragraph is not visible");
      if(kind==="delete"){expected={present:true};replacement={present:false};}
      else if(kind==="move"){if(after===selected)throw new Error("A paragraph cannot follow itself");expected={position:{after:index?paragraphs[index-1].id:null}};replacement={position:{after}};}
      else throw new Error("Unknown paragraph structure action");
    }
    const version=this.#version,prepared=await this.#client.structureHistory(this.#source,{expected_history_sha256:this.#result.history_sha256,
      expected_structure_conflicts_sha256:this.#result.structure_conflicts_sha256,actor,paragraph_id:paragraphId,expected_absent,expected,replacement});
    if(version===this.#version){this.#publish(prepared);this.#status(prepared.result.merged?"Causal paragraph structure recorded. Review native pagination before checkpointing.":"Structure operation recorded; remaining conflicts still require explicit resolution.");}
  }
  async #edit(){const field=this.#q("#text"),text=this.#result.merged.paragraphs.find(p=>p.id===this.#q("#paragraph").value)?.text;
    if(text===undefined)throw new Error("Choose a paragraph");
    // Textarea values normalize CRLF; map normalized UTF-16 cursor positions
    // back to exact original Unicode scalars before producing UTF-8 offsets.
    const {sourceSelectionRange}=await import("./story-client.js");
    const [start,end]=sourceSelectionRange(text,field.selectionStart,field.selectionEnd),scalars=Array.from(text),encoder=new TextEncoder();
    const range=[encoder.encode(scalars.slice(0,start).join("")).length,encoder.encode(scalars.slice(0,end).join("")).length];
    const version=this.#version,result=await this.#client.editHistory(this.#source,{actor:this.#q("#actor").value,
      paragraph_id:this.#q("#paragraph").value,range,expected_text:scalars.slice(start,end).join(""),replacement:this.#q("#replacement").value,expected_history_sha256:this.#result.history_sha256});
    if(version===this.#version)this.#publish(result);
  }
  async #import(){const file=this.#q("#file").files[0];if(!file)return;if(file.size>32*1024*1024)throw new Error("History file exceeds 32 MiB");
    const version=this.#version,value=JSON.parse(await file.text());
    if(![1,2].includes(value.schema_version)||!value.history)throw new Error("Invalid history package");
    // The native base fingerprints, not a supplied filename or mutable view,
    // decide whether imported events belong to this exact collaboration epoch.
    let source=this.#source;
    if(!source){
      if(value.base)source={kind:"start",base:value.base,history:value.history,replace_epoch:false};
      else if(typeof value.story_id==="string")source=(await this.#client.resumeHistory(value.story_id)).source;
      else throw new Error("Package needs a base story or a saved story identity");
    }
    const result=await this.#client.joinHistory(source,[value.history]);
    if(version===this.#version){if(!this.#source)this.#adopted=undefined;this.#publish(result);}
  }
  #export(){const json=JSON.stringify({schema_version:2,story_id:this.#source.kind==="start"?this.#source.base.story_id:this.#source.story_id,base:this.#base,history:this.#result.history});if(new TextEncoder().encode(json).length>32*1024*1024)throw new Error("History package exceeds 32 MiB; use the host delta API");
    const url=URL.createObjectURL(new Blob([json],{type:"application/json"})),link=document.createElement("a");link.href=url;link.download="pdf-story-history.json";link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
  }
  #adopt(){if(!this.#result?.merged||!this.#q("#review").checked)throw new Error("Review the complete projection first");
    if(this.#revision()!==this.#client?.state?.revision)throw new Error("The PDF revision changed; resume its saved history before replacing the current draft");
    const request=structuredClone(this.#result.merged),expected=this.#adopted===undefined?undefined:structuredClone(this.#adopted);
    if(!this.dispatchEvent(new CustomEvent("historydraft",{detail:{request,expected,source:structuredClone(this.#source),history_sha256:this.#result.history_sha256},bubbles:true,composed:true,cancelable:true})))throw new Error("Host draft changed or is busy. No draft was replaced; resolve or begin a new epoch explicitly.");
    this.#adopted=structuredClone(request);this.#q("#review").checked=false;this.#status("Reviewed projection offered as an unsaved draft. Native layout preview and checkpoint approval are still required.");
  }
}
if(!customElements.get("wellfriend-story-history-editor"))customElements.define("wellfriend-story-history-editor",WellfriendStoryHistoryEditor);
