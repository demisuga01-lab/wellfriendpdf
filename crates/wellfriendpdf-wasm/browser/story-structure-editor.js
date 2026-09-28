/** Explicit logical conflict review over the shared native session. Imported
 * JSON is data; this component neither publishes PDFs nor merges authority. */
export class WellfriendStoryStructureEditor extends HTMLElement {
  #client;#request;#review;#resolved;#expected;#acks=new Set();#page=0;#busy=false;#version=0;
  #changed=()=>{this.#version++;this.#buttons();if(this.#request&&this.#request.base.input_sha256!==this.#client?.state?.revision)this.#status("PDF revision changed. Export these branches before opening a newly bound review; old source selections cannot replay.");};
  constructor(){super();this.attachShadow({mode:"open"}).innerHTML=`<style>
    :host{display:block;font:14px system-ui}label{display:block;margin:8px 0}button,textarea{font:inherit}button{margin:3px;padding:6px}textarea{width:100%;min-height:180px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:220px;overflow:auto}fieldset{margin:8px 0}legend{overflow-wrap:anywhere;max-width:240px}.notice{font-size:12px}:focus-visible{outline:3px solid #3668ce}
    </style><details><summary>Structural branch conflict review</summary>
      <p class="notice">Merge logical text/style, paragraph structure and frame geometry. Source owners, font files, table topology, images, anchors and permissions cannot be granted here. Normal saving may detach a stored causal text history. This panel does not publish PDF bytes.</p>
      <button id="import">Import merge package</button><button id="branch">Add branch snapshot</button><button id="export">Export merge package</button>
      <input hidden id="package-file" type="file" accept="application/json,.json"><input hidden id="branch-file" type="file" accept="application/json,.json">
      <p id="identity"></p><div id="conflicts"></div><button id="previous">Previous conflicts</button><button id="next">Next conflicts</button><p id="progress"></p>
      <p class="notice">Alternatives change only their displayed conflict scope. Restored/new paragraphs append; edit the JSON order explicitly before resolution. Non-conflicting values remain protected by native validation. Unplaced paragraphs are included at the end for review, not silently discarded.</p>
      <label>Proposed logical paragraphs and frame geometry<textarea id="draft" spellcheck="false"></textarea></label>
      <button id="recalculate">Propose typed table recalculation</button><button id="resolve">Validate conflict decisions</button>
      <label><input id="approve" type="checkbox"> I reviewed the complete resolved draft, including ordering and potential causal-history detachment</label>
      <button id="adopt">Use reviewed structural draft</button><pre id="report"></pre><p id="status" role="status" aria-live="polite">Begin from the host draft or import a package.</p>
    </details>`;
    for(const [button,file] of [["#import","#package-file"],["#branch","#branch-file"]])this.#q(button).addEventListener("click",()=>{this.#q(file).value="";this.#q(file).click();});
    this.#q("#package-file").addEventListener("change",()=>this.#run(()=>this.#import(false)));
    this.#q("#branch-file").addEventListener("change",()=>this.#run(()=>this.#import(true)));
    this.#q("#export").addEventListener("click",()=>this.#run(()=>this.#export()));
    this.#q("#draft").addEventListener("input",()=>this.#invalidate());
    this.#q("#approve").addEventListener("change",()=>this.#buttons());
    this.#q("#resolve").addEventListener("click",()=>this.#run(()=>this.#resolve()));
    this.#q("#recalculate").addEventListener("click",()=>this.#run(()=>this.#recalculate()));
    this.#q("#adopt").addEventListener("click",()=>this.#run(()=>this.#adopt()));
    this.#q("#previous").addEventListener("click",()=>{this.#page--;this.#conflicts();this.#buttons();});
    this.#q("#next").addEventListener("click",()=>{this.#page++;this.#conflicts();this.#buttons();});this.#buttons();
  }
  #q(selector){return this.shadowRoot.querySelector(selector);}
  #status(value){this.#q("#status").textContent=value;}
  set client(value){if(this.#busy)throw new Error("Structure review is busy");this.#client?.removeEventListener("change",this.#changed);this.#client=value;if(this.isConnected)value?.addEventListener("change",this.#changed);this.#changed();}
  get client(){return this.#client;}
  connectedCallback(){this.#client?.addEventListener("change",this.#changed);this.#buttons();}
  disconnectedCallback(){this.#client?.removeEventListener("change",this.#changed);this.#version++;}
  #buttons(){const same=!!this.#review&&this.#request.base.input_sha256===this.#client?.state?.revision;
    for(const element of this.shadowRoot.querySelectorAll("button,input,textarea"))element.disabled=this.#busy||!same;
    this.#q("#import").disabled=this.#busy||!this.#client?.state;this.#q("#package-file").disabled=this.#q("#import").disabled;
    this.#q("#export").disabled=this.#busy||!this.#request;
    this.#q("#previous").disabled||=this.#page===0;this.#q("#next").disabled||=(this.#page+1)*25>=(this.#review?.conflicts.length??0);
    this.#q("#recalculate").disabled||=!this.#review?.candidate.table_layout;
    this.#q("#resolve").disabled||=this.#acks.size!==(this.#review?.conflicts.length??0);
    this.#q("#adopt").disabled||=!this.#resolved||!this.#q("#approve").checked;
  }
  async #run(action){if(this.#busy)return;this.#busy=true;this.#buttons();try{await action();}catch(error){if(error.name!=="AbortError")this.#status(error.message??String(error));}finally{this.#busy=false;this.#buttons();}}
  async begin(base){if(this.#busy)throw new Error("Structure review is busy");if(this.#request&&!globalThis.confirm("Replace the local structural review? Export its branches first if needed."))return;
    await this.#run(async()=>{const owned=structuredClone(base);await this.#reviewRequest({base:owned,branches:[]},owned);});}
  async #reviewRequest(request,expected){const version=this.#version,review=await this.#client.reviewStructure(request);if(version!==this.#version)return;
    this.#request=structuredClone(request);this.#expected=expected===undefined?undefined:structuredClone(expected);this.#review=review;this.#page=0;this.#acks.clear();this.#invalidate();
    this.#writeDraft({paragraphs:[...review.candidate.paragraphs,...review.unplaced_paragraphs],frame_geometry:review.candidate.frames.map(f=>({frame_id:f.id,rect:f.rect,exclusions:f.exclusions}))});
    this.#q("#identity").textContent=`Base story hash: ${review.base_story_sha256}; ${request.branches.length} branches. Exported packages include source text and supplied font assets.`;
    this.#conflicts();this.#q("details").open=true;this.#status("Review all conflicts and the complete candidate before validating. No PDF bytes changed.");}
  #invalidate(){this.#resolved=undefined;this.#q("#approve").checked=false;this.#q("#report").textContent="Resolution is not validated; native layout preview will still be required.";this.#buttons();}
  #draft(){const value=JSON.parse(this.#q("#draft").value);if(!value||Object.keys(value).sort().join(",")!=="frame_geometry,paragraphs"||!Array.isArray(value.paragraphs)||!Array.isArray(value.frame_geometry))throw new Error("Draft needs only paragraphs and frame_geometry arrays");return value;}
  #writeDraft(value){this.#q("#draft").value=JSON.stringify(value,null,2);this.#invalidate();}
  #conflicts(){const root=this.#q("#conflicts");root.replaceChildren();
    for(const conflict of this.#review?.conflicts.slice(this.#page*25,(this.#page+1)*25)??[]){
      const card=document.createElement("fieldset"),legend=document.createElement("legend"),reason=document.createElement("p");legend.textContent=conflict.path;reason.textContent=conflict.reason;card.append(legend,reason);
      for(const alternative of conflict.alternatives){const detail=document.createElement("details"),title=document.createElement("summary"),pre=document.createElement("pre"),button=document.createElement("button");
        title.textContent=alternative.branch_id===null?"Base value":`Branch ${alternative.branch_id}`;button.textContent="Use this alternative";
        detail.append(title,pre,button);detail.addEventListener("toggle",()=>{if(detail.open&&!pre.textContent)pre.textContent=JSON.stringify(alternative.value,null,2);});
        button.addEventListener("click",()=>this.#run(()=>this.#choose(conflict.target,alternative.value)));card.append(detail);}
      const label=document.createElement("label"),ack=document.createElement("input");ack.type="checkbox";ack.checked=this.#acks.has(conflict.conflict_id);label.append(ack,document.createTextNode(" I reviewed and resolved this conflict in the candidate"));
      ack.addEventListener("change",()=>{if(ack.checked)this.#acks.add(conflict.conflict_id);else this.#acks.delete(conflict.conflict_id);this.#invalidate();this.#progress();});card.append(label);root.append(card);
    }this.#progress();}
  #progress(){this.#q("#progress").textContent=`${this.#acks.size}/${this.#review?.conflicts.length??0} conflicts acknowledged; page ${this.#page+1}.`;}
  #choose(target,value){const draft=this.#draft();value=structuredClone(value);
    const setParagraph=(id,paragraph)=>{const index=draft.paragraphs.findIndex(p=>p.id===id);if(paragraph===null){if(index>=0)draft.paragraphs.splice(index,1);}else{if(paragraph.id!==id)throw new Error("Alternative paragraph ID mismatch");if(index>=0)draft.paragraphs[index]=paragraph;else draft.paragraphs.push(paragraph);}};
    if(target.kind==="frame_field"){const frame=draft.frame_geometry.find(f=>f.frame_id===target.frame_id);if(!frame||!["rect","exclusions"].includes(target.field))throw new Error("Unknown frame field");frame[target.field]=value;}
    else if(target.kind==="paragraph_field"){const paragraph=draft.paragraphs.find(p=>p.id===target.paragraph_id),optional=target.field==="page_break_before";if(!paragraph||target.field==="id"||!optional&&![...Object.keys(paragraph)].includes(target.field))throw new Error("Restore the paragraph or edit its typed field explicitly");if(optional&&value==="none")delete paragraph[target.field];else paragraph[target.field]=value;}
    else if(target.kind==="paragraph"||target.kind==="retained_paragraph")setParagraph(target.paragraph_id,value);
    else if(target.kind==="paragraph_order"){const map=new Map(draft.paragraphs.map(p=>[p.id,p]));const ordered=[];for(const id of value){if(map.has(id)){ordered.push(map.get(id));map.delete(id);}}draft.paragraphs=[...ordered,...map.values()];}
    else if(target.kind==="insertions"){draft.paragraphs=draft.paragraphs.filter(p=>!target.paragraph_ids.includes(p.id));for(const paragraph of value)draft.paragraphs.push(paragraph);}
    else if(target.kind==="table_projection"){for(const paragraph of value){const current=draft.paragraphs.find(p=>p.id===paragraph.id);if(current)current.text=paragraph.text;else draft.paragraphs.push(paragraph);}}
    else throw new Error("Unknown conflict target");this.#writeDraft(draft);
  }
  async #resolve(){const draft=this.#draft(),version=this.#version;const resolved=await this.#client.resolveStructure(this.#request,{...draft,expected_review_sha256:this.#review.review_sha256,acknowledged_conflicts:[...this.#acks]});
    if(version!==this.#version)return;this.#resolved=resolved;this.#q("#approve").checked=false;this.#q("#report").textContent=JSON.stringify({review_sha256:resolved.review_sha256,resolution_sha256:resolved.resolution_sha256,resolved_conflict_ids:resolved.resolved_conflict_ids,limits:resolved.limits},null,2);this.#status("Logical resolution validated. Review it, then adopt it as an unsaved draft for native layout approval.");}
  async #recalculate(){const draft=this.#draft(),version=this.#version,request=structuredClone(this.#review.candidate);request.paragraphs=draft.paragraphs;
    const recalculated=await this.#client.synchronizeTableValues(request);if(version===this.#version){draft.paragraphs=recalculated.paragraphs;this.#writeDraft(draft);this.#status("Typed table recalculation proposed. Review the changed text; no conflict was automatically acknowledged.");}}
  async #import(branch){const file=this.#q(branch?"#branch-file":"#package-file").files[0];if(!file)return;if(file.size>32*1024*1024)throw new Error("Merge JSON exceeds 32 MiB");const version=this.#version,value=JSON.parse(await file.text());if(version!==this.#version)return;
    if(branch){if(!this.#request)throw new Error("Begin or import a base review first");if(!globalThis.confirm("Adding a branch recomputes the candidate and resets local decisions. Export pending decisions first if needed. Continue?"))return;const request=structuredClone(this.#request);request.branches.push(value);await this.#reviewRequest(request,this.#expected);}
    else{if(value.schema_version!==1||!value.request)throw new Error("Invalid merge package");if(this.#request&&!globalThis.confirm("Replace the local merge package? Export pending decisions first if needed."))return;await this.#reviewRequest(value.request,undefined);
      if(version!==this.#version)return;if(value.draft&&value.expected_review_sha256===this.#review?.review_sha256){if(Object.keys(value.draft).sort().join(",")!=="frame_geometry,paragraphs"||!Array.isArray(value.draft.paragraphs)||!Array.isArray(value.draft.frame_geometry))throw new Error("Invalid saved logical draft");this.#writeDraft(value.draft);this.#status("Pending draft restored against the verified review. Conflict acknowledgments and all approvals were reset.");}}}
  #export(){const text=JSON.stringify({schema_version:1,request:this.#request,base_story_sha256:this.#review?.base_story_sha256,expected_review_sha256:this.#review?.review_sha256,draft:this.#draft(),resolution_sha256:this.#resolved?.resolution_sha256??null});if(new TextEncoder().encode(text).length>32*1024*1024)throw new Error("Merge package exceeds 32 MiB");const url=URL.createObjectURL(new Blob([text],{type:"application/json"})),link=document.createElement("a");link.href=url;link.download="pdf-story-merge.json";link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
  #adopt(){if(!this.#resolved||!this.#q("#approve").checked)throw new Error("Approve the validated complete resolution first");if(this.#request.base.input_sha256!==this.#client?.state?.revision)throw new Error("The PDF revision changed");
    const request=structuredClone(this.#resolved.merged);if(!this.dispatchEvent(new CustomEvent("structuredraft",{bubbles:true,composed:true,cancelable:true,detail:{request,expected:this.#expected===undefined?undefined:structuredClone(this.#expected),review_sha256:this.#resolved.review_sha256,resolution_sha256:this.#resolved.resolution_sha256}})))throw new Error("Host draft changed, is busy or has an attached causal history. Resolve that explicitly before adoption.");this.#expected=structuredClone(request);this.#q("#approve").checked=false;this.#status("Structural resolution adopted as an unsaved draft. Preview and approve native layout before saving.");}
}
if(!customElements.get("wellfriend-story-structure-editor"))customElements.define("wellfriend-story-structure-editor",WellfriendStoryStructureEditor);
