/** Review UI for exact-revision multi-slot text replacement. It never paints a
 * DOM text overlay and never publishes before the exact approval/font has been
 * privately rendered by the worker. */
export class WellfriendPaintPartitionEditor extends HTMLElement {
  #client; #request; #proposal; #preview; #approval; #fontBytes; #fontSha256;
  #urls=[]; #busy=false; #version=0;
  #changed=()=>{this.#resetAll();this.#status("The PDF revision changed. Propose the partition again.");};
  constructor(){
    super();
    this.attachShadow({mode:"open"}).innerHTML=`<style>
      :host{display:block;font:14px system-ui;color:inherit}*{box-sizing:border-box}
      button,input{font:inherit}button{padding:6px 10px;margin:4px}fieldset{margin:10px 0;border:1px solid #aeb5c0;border-radius:8px}
      .rect{display:grid;grid-template-columns:repeat(4,minmax(70px,1fr));gap:6px}.rect label{font-size:12px}.rect input{width:100%}
      .images{display:grid;grid-template-columns:1fr 1fr;gap:10px}.images figure{margin:0}.images img{display:block;max-width:100%;background:white}
      pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:260px;overflow:auto}.notice{font-size:12px}
      :focus-visible{outline:3px solid #3668ce;outline-offset:2px}
    </style><details><summary>Exact PDF paint-slot replacement</summary>
      <p class="notice">This workflow preserves intervening PDF paint by mapping replacement graphemes to original BT/ET slots. Review every physical region and the private before/candidate render. It is ordinary editing, not sanitizing redaction.</p>
      <button id="propose">Propose source-slot mapping</button><div id="candidates"></div>
      <label>Approved shaping font, when source coverage is insufficient <input id="font" type="file" accept=".ttf,.otf,.ttc,.otc,font/ttf,font/otf"></label>
      <label>Preview DPI <input id="dpi" type="number" min="24" max="600" value="96"></label>
      <button id="preview">Render private candidate</button><div class="images" id="images"></div>
      <pre id="report" aria-label="Paint partition proposal and preview receipt"></pre>
      <label><input id="approve" type="checkbox"> I reviewed every source slot, region, font and displayed candidate page</label>
      <button id="apply">Publish reviewed PDF bytes</button><button id="cancel">Cancel worker work</button>
      <p id="status" role="status" aria-live="polite">Set a client and a paragraph-reflow request.</p>
    </details>`;
    this.#q("#propose").addEventListener("click",()=>this.#run(()=>this.#propose()));
    this.#q("#preview").addEventListener("click",()=>this.#run(()=>this.#render()));
    this.#q("#apply").addEventListener("click",()=>this.#run(()=>this.#apply()));
    this.#q("#cancel").addEventListener("click",()=>this.#run(async()=>{await this.#client?.cancel();this.#resetAll();this.#status("Cancelled. The last published PDF is retained.");}));
    this.#q("#font").addEventListener("change",()=>this.#run(()=>this.#readFont()));
    this.shadowRoot.addEventListener("input",event=>{if(event.target.id!=="approve"&&event.target.id!=="font")this.#invalidatePreview();this.#buttons();});
    this.#q("#approve").addEventListener("change",()=>this.#buttons());this.#buttons();
  }
  #q(value){return this.shadowRoot.querySelector(value);}
  connectedCallback(){this.#client?.addEventListener("change",this.#changed);this.#buttons();}
  disconnectedCallback(){this.#client?.removeEventListener("change",this.#changed);this.#revoke();}
  set client(value){if(this.#busy)throw new Error("Cannot replace a busy paint-partition client");this.#client?.removeEventListener("change",this.#changed);this.#client=value;if(this.isConnected)value?.addEventListener("change",this.#changed);this.#resetAll();}
  get client(){return this.#client;}
  set request(value){if(this.#busy)throw new Error("Cannot replace a busy paint-partition request");this.#request=value==null?undefined:structuredClone(value);this.#resetAll(false);}
  get request(){return this.#request==null?undefined:structuredClone(this.#request);}
  #status(value){this.#q("#status").textContent=value;}
  #revoke(){for(const url of this.#urls)URL.revokeObjectURL(url);this.#urls=[];}
  #invalidatePreview(){this.#version++;this.#preview=undefined;this.#approval=undefined;this.#q("#approve").checked=false;this.#revoke();this.#q("#images").replaceChildren();}
  #resetAll(clearRequest=true){this.#invalidatePreview();this.#proposal=undefined;this.#fontBytes=undefined;this.#fontSha256=undefined;this.#q("#font").value="";this.#q("#candidates").replaceChildren();this.#q("#report").textContent="";if(clearRequest)this.#request=undefined;this.#buttons();}
  #buttons(){const ready=!!this.#client?.state&&!!this.#request;for(const field of this.shadowRoot.querySelectorAll("button,input"))field.disabled=this.#busy||!ready;this.#q("#cancel").disabled=!this.#client;this.#q("#preview").disabled||=!this.#proposal;this.#q("#apply").disabled||=!this.#preview||!this.#q("#approve").checked;}
  async #run(action){if(this.#busy)return;this.#busy=true;this.#buttons();try{await action();}catch(error){if(error?.name!=="AbortError")this.#status(error?.message??String(error));}finally{this.#busy=false;this.#buttons();}}
  async #propose(){if(!this.#client?.state||!this.#request)throw new Error("Set an open client and paragraph-reflow request");this.#resetAll(false);const version=this.#version;const proposal=await this.#client.proposePaintPartitions(this.#request);if(version!==this.#version)return;this.#proposal=proposal;
    for(const candidate of proposal.candidates){const box=document.createElement("fieldset"),legend=document.createElement("legend");legend.textContent=`Source paint slot ${candidate.source_text_object}; replacement scalars ${candidate.replacement_scalar_range.join("..")}`;box.append(legend);const note=document.createElement("p");note.className="notice";note.textContent=`${candidate.selected_span_ids.length} source span(s); boundaries ${candidate.start_boundary_class} / ${candidate.end_boundary_class}`;box.append(note);const rect=document.createElement("div");rect.className="rect";const initial=candidate.suggested_region??this.#request.options?.region??[0,0,0,0];for(const [index,name]of ["Left","Bottom","Right","Top"].entries()){const label=document.createElement("label"),input=document.createElement("input");label.textContent=name;input.type="number";input.step="any";input.value=String(initial[index]);input.dataset.coordinate=String(index);label.append(input);rect.append(label);}box.dataset.sourceTextObject=String(candidate.source_text_object);box.append(rect);this.#q("#candidates").append(box);}
    this.#q("#report").textContent=JSON.stringify(proposal,null,2);this.#status("Proposal created without mutation. Review every region, then render the candidate.");this.#buttons();
  }
  async #readFont(){this.#invalidatePreview();const file=this.#q("#font").files?.[0];this.#fontBytes=undefined;this.#fontSha256=undefined;if(!file)return;if(file.size<1||file.size>4*1024*1024)throw new Error("Font must be 1..=4 MiB");const bytes=new Uint8Array(await file.arrayBuffer());const digest=new Uint8Array(await crypto.subtle.digest("SHA-256",bytes));this.#fontBytes=bytes;this.#fontSha256=[...digest].map(value=>value.toString(16).padStart(2,"0")).join("");this.#status(`Approved font loaded: ${this.#fontSha256}`);}
  #buildApproval(){if(!this.#proposal)throw new Error("Propose source slots first");const partitions=[...this.#q("#candidates").querySelectorAll("fieldset")].map(box=>{const region=[...box.querySelectorAll("input")].map(input=>Number(input.value));if(!region.every(Number.isFinite)||region[0]>=region[2]||region[1]>=region[3])throw new Error("Every source slot needs a finite nonempty region");return{source_text_object:Number(box.dataset.sourceTextObject),region,final_lines:null};});const approval={proposal_id:this.#proposal.proposal_id,partitions};if(this.#fontSha256)approval.font_sha256=this.#fontSha256;return approval;}
  async #render(){this.#invalidatePreview();const approval=this.#buildApproval(),version=this.#version,dpi=Number(this.#q("#dpi").value);const preview=await this.#client.previewPaintPartitions(this.#request,this.#proposal,approval,{dpi,fontBytes:this.#fontBytes});if(version!==this.#version)return;this.#approval=approval;this.#preview=preview;
    for(const [label,bytes]of [["Before",preview.before_png],["Candidate",preview.candidate_png]]){const figure=document.createElement("figure"),caption=document.createElement("figcaption"),image=document.createElement("img");caption.textContent=`${label}, page ${preview.page}`;image.alt=caption.textContent;const url=URL.createObjectURL(new Blob([bytes],{type:"image/png"}));this.#urls.push(url);image.src=url;figure.append(caption,image);this.#q("#images").append(figure);await image.decode();if(version!==this.#version)return;}
    this.#q("#report").textContent=JSON.stringify({proposal:this.#proposal,approval,preview:{input_sha256:preview.input_sha256,candidate_output_sha256:preview.candidate_output_sha256,page:preview.page,dpi:preview.dpi,report:preview.report}},null,2);this.#status("Exact private candidate displayed. Review both images and explicitly approve publication.");this.#buttons();
  }
  async #apply(){if(!this.#preview||!this.#approval||!this.#q("#approve").checked)throw new Error("Render and approve the exact candidate first");if(!this.dispatchEvent(new CustomEvent("beforepaintpartitionedit",{bubbles:true,composed:true,cancelable:true})))throw new Error("Host blocked publication");const result=await this.#client.applyPaintPartitions(this.#request,this.#proposal,this.#approval,this.#fontBytes);this.#resetAll();this.#status("Published the reviewed native PDF edit. Undo retains the prior bytes.");this.dispatchEvent(new CustomEvent("paintpartitionedit",{detail:result,bubbles:true,composed:true}));}
}
if(!customElements.get("wellfriend-paint-partition-editor"))customElements.define("wellfriend-paint-partition-editor",WellfriendPaintPartitionEditor);
