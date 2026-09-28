import { sourceSelectionRange } from "./story-client.js";

/** Native source-local occurrence editing over the host's existing session.
 * No DOM overlay is exported. No second document store or implicit approval. */
export class WellfriendScopedTextEditor extends HTMLElement {
  #client; #occurrences = []; #widgetFields = []; #plan; #preview; #urls = []; #busy = false; #version = 0;
  #changed = () => { this.#clear(); this.#occurrences = []; this.#widgetFields = []; this.#q("#occurrence").replaceChildren(); this.#q("#source").value = ""; this.#pages(); };
  constructor() {
    super();
    this.attachShadow({mode:"open"}).innerHTML = `<style>
      :host{display:block;font:14px system-ui;color:inherit}*{box-sizing:border-box}
      label{display:block;margin:8px 0}button,input,select,textarea{font:inherit;max-width:100%}
      textarea,select{width:100%}textarea{min-height:80px}button{padding:6px;margin:3px}
      input[type=number]{width:65px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:8px}
      figure{margin:0}img{max-width:100%;background:white}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto}
      .notice{font-size:12px}button:focus-visible,input:focus-visible,select:focus-visible,textarea:focus-visible{outline:3px solid #3668ce}
    </style><details><summary>Native Form / annotation text editing</summary>
      <p class="notice">Select an exact source occurrence, not a page-coordinate guess. Forms/annotations clone this occurrence. Text-field editing coordinates every widget and the field value. Source rectangles are before Form/AP transforms. Tagged migration, specialized/rich/scripted fields and signature-invalidating rewrites require an explicit advanced request.</p>
      <label>Page <select id="page"></select></label><button id="load">Load native text occurrences</button>
      <label>Occurrence <select id="occurrence"></select></label>
      <label>Select the source text to replace<textarea id="source" readonly spellcheck="false"></textarea></label>
      <label>Replacement<textarea id="replacement" spellcheck="false"></textarea></label>
      <label>Placement <select id="mode"><option value="safe_patch">Same-width source patch</option><option value="paragraph_reflow_horizontal">Horizontal source-local reflow</option><option value="paragraph_reflow_rtl">RTL source-local reflow</option><option value="paragraph_reflow_vertical">Vertical source-local reflow</option></select></label>
      <label>Approved source rectangle (left, bottom, right, top)<span id="rect"></span></label>
      <label>Replacement size <input id="size" type="number" min="1" max="1000" value="12" step=".1"></label>
      <label><input id="fallback" type="checkbox"> Allow disclosed generated-font substitution</label>
      <label>Annotation / field semantics <select id="metadata"><option value="">Choose for an appearance edit</option><option value="preserve_annotation_metadata">Preserve comments / metadata</option><option value="synchronize_free_text_plain_text">FreeText: sync Contents and discard rich text</option><option value="coordinate_text_field">Text field: update value and every widget; keep source defaults/reset value</option></select></label>
      <p class="notice">Field mode requires every normal widget display to equal the plain field value. It applies the selected text change to all widgets, inheriting each source style and using its own source rectangle (the selected rectangle above overrides only the selected widget). The replacement size is shared. Review every displayed widget page. No actions execute.</p>
      <label>Preview DPI <input id="dpi" type="number" min="24" max="600" value="96"></label>
      <label><input id="exact" type="checkbox"> Require exact renderer support (refuse unsupported rendering)</label>
      <button id="preview">Plan and render candidate</button>
      <div id="images"></div><pre id="report" aria-label="Exact candidate and substitutions"></pre>
      <label><input id="approve" type="checkbox"> I reviewed these native pages, source scope, fonts and metadata decisions</label>
      <button id="apply">Apply this reviewed candidate</button>
      <button id="cancel">Cancel worker work</button>
      <p id="status" role="status" aria-live="polite">Open a PDF in the shared session.</p>
    </details>`;
    for (const title of ["left","bottom","right","top"]) {
      const field=document.createElement("input");field.type="number";field.step="any";field.setAttribute("aria-label",`Source rectangle ${title}`);this.#q("#rect").append(field);
    }
    this.shadowRoot.addEventListener("input",event=>{if(event.target.id!=="approve"&&event.target.id!=="source")this.#clear();});
    this.#q("#source").addEventListener("select",()=>this.#clear());
    this.#q("#page").addEventListener("change",()=>{this.#clear();this.#occurrences=[];this.#widgetFields=[];this.#q("#occurrence").replaceChildren();this.#q("#source").value="";});
    this.#q("#occurrence").addEventListener("change",()=>this.#select());
    this.#q("#approve").addEventListener("change",()=>this.#buttons());
    for(const [id,action]of [["load",()=>this.#load()],["preview",()=>this.#render()],["apply",()=>this.#apply()]])this.#q(`#${id}`).addEventListener("click",()=>this.#run(action));
    this.#q("#cancel").addEventListener("click",async()=>{
      this.#clear();try{await this.#client?.cancel();this.#status("Cancelled; exact last published PDF retained. Re-preview before applying.");}catch(error){this.#status(error.message);}
    });
    this.#buttons();
  }
  #q(selector){return this.shadowRoot.querySelector(selector);}
  connectedCallback(){this.#client?.addEventListener("change",this.#changed);this.#pages();}
  disconnectedCallback(){this.#client?.removeEventListener("change",this.#changed);this.#clear();}
  set client(value){if(this.#busy)throw new Error("Cannot replace a busy native editor client");this.#client?.removeEventListener("change",this.#changed);this.#client=value;if(this.isConnected)value?.addEventListener("change",this.#changed);this.#changed();}
  get client(){return this.#client;}
  #status(text){this.#q("#status").textContent=text;}
  #pages(){const select=this.#q("#page"),selected=select.value;select.replaceChildren();for(const page of this.#client?.state?.pages??[])select.add(new Option(`Page ${page.page}`,String(page.page)));if([...select.options].some(o=>o.value===selected))select.value=selected;this.#buttons();}
  #clear(){this.#version++;this.#plan=undefined;this.#preview=undefined;this.#q("#approve").checked=false;for(const url of this.#urls)URL.revokeObjectURL(url);this.#urls=[];this.#q("#images").replaceChildren();this.#q("#report").textContent="";this.#buttons();}
  #buttons(){const open=!!this.#client?.state;for(const field of this.shadowRoot.querySelectorAll("button,input,select,textarea"))field.disabled=!open||this.#busy;
    this.#q("#cancel").disabled=!this.#client;this.#q("#apply").disabled||=!this.#plan||!this.#preview||!this.#q("#approve").checked;
  }
  async #run(action){if(this.#busy)return;this.#busy=true;this.#buttons();try{await action();}catch(error){if(error.name!=="AbortError")this.#status(error.message??String(error));}finally{this.#busy=false;this.#buttons();}}
  async #load(){this.#clear();const version=this.#version,revision=this.#client.state?.revision;
    const result=await this.#client.scopedSources(Number(this.#q("#page").value));
    if(version!==this.#version||revision!==this.#client.state?.revision)return;
    this.#occurrences=result.pages.flatMap(page=>[...page.forms.occurrences.map(value=>({scope:"form",value})),...page.appearances.occurrences.map(value=>({scope:"appearance",value}))]);
    this.#widgetFields=result.widget_fields??[];
    const select=this.#q("#occurrence");select.replaceChildren();for(const [index,item]of this.#occurrences.entries())select.add(new Option(`${item.scope} ${index+1}: ${item.value.text.logical_text.slice(0,90)}`,String(index)));
    this.#select();this.#status(`${this.#occurrences.length} source-local occurrences. Select exact text and review its placement.`);
  }
  #select(){this.#clear();const item=this.#occurrences[Number(this.#q("#occurrence").value)];this.#q("#source").value=item?.value.text.logical_text??"";this.#q("#replacement").value="";this.#q("#metadata").value="";
    const rect=item?.value.form_bbox??item?.value.source_bbox??[0,0,0,0];[...this.#q("#rect").children].forEach((input,i)=>{input.value=rect[i];});
  }
  async #request(){const item=this.#occurrences[Number(this.#q("#occurrence").value)];if(!item)throw new Error("Load and select a native occurrence");const field=this.#q("#source");
    const [start,end]=sourceSelectionRange(item.value.text.logical_text,field.selectionStart,field.selectionEnd);if(start===end)throw new Error("Select a non-empty source range");
    const region=[...this.#q("#rect").children].map(input=>Number(input.value)),size=Number(this.#q("#size").value);
    if(!region.every(Number.isFinite)||region[2]<=region[0]||region[3]<=region[1]||!Number.isFinite(size)||size<1||size>1000)throw new Error("Enter a valid source-local rectangle and font size");
    const edit={page:item.value.target.page,logical_start:start,logical_end:end,replacement_text:this.#q("#replacement").value,mode:this.#q("#mode").value,style_policy:"inherit_leading",
      options:{region,font_size:size,line_spacing:1.2,max_lines_or_columns:4096,overflow_policy:"error",signature_policy_override:false,deterministic:true}};
    const request={target:structuredClone(item.value.target),edit};
    const allowFont=this.#q("#fallback").checked;
    if(item.scope==="appearance"&&item.value.annotation_subtype==="Widget"){
      if(this.#q("#metadata").value!=="coordinate_text_field")throw new Error("Choose the explicit field-wide synchronization policy for a widget");
      const sameId=(a,b)=>a?.[0]===b?.[0]&&a?.[1]===b?.[1];
      const owner=this.#widgetFields.find(field=>field.widgets.some(w=>sameId(w.annotation,item.value.target.annotation)));
      if(!owner||owner.value!==item.value.text.logical_text||item.value.target.invocation_path.length)throw new Error("This display requires a typed field mapping; the panel supports direct plain-value appearances");
      const value=Array.from(owner.value),replacement=[...value.slice(0,start),edit.replacement_text,...value.slice(end)].join("");
      const inventories=new Map([[item.value.target.page,this.#occurrences.filter(o=>o.scope==="appearance").map(o=>o.value)]]);
      const widgets=[];
      for(const widget of owner.widgets){
        if(widget.display_text!==owner.value)throw new Error("Formatted or masked field displays need explicit per-widget display mappings");
        if(!inventories.has(widget.page)){const source=await this.#client.scopedSources(widget.page);inventories.set(widget.page,source.pages.find(p=>p.page===widget.page)?.appearances.occurrences??[]);}
        const occurrence=inventories.get(widget.page).find(o=>sameId(o.target.annotation,widget.annotation)&&o.target.invocation_path.length===0);
        if(!occurrence||occurrence.text.logical_text!==owner.value)throw new Error("Nested or decorated widget text needs an explicit source mapping");
        const localEdit=structuredClone(edit);localEdit.page=widget.page;localEdit.logical_start=0;localEdit.logical_end=value.length;localEdit.replacement_text=replacement;
        if(!sameId(widget.annotation,item.value.target.annotation))localEdit.options.region=occurrence.source_bbox;
        widgets.push({appearance:{target:structuredClone(occurrence.target),edit:localEdit,metadata_policy:"preserve_annotation_metadata"},expected_display:owner.value,replacement_display:replacement});
      }
      return {operation:{kind:"scoped_text",request:{source:{scope:"widget_field",request:{target:structuredClone(owner.target),expected_value:owner.value,replacement_value:replacement,widgets,default_appearance:{kind:"preserve_source_defaults"}}}}},policy:{allow_font_substitution:allowFont}};
    }
    if(item.scope==="form")request.shared_form_policy="clone_edit_one_instance";
    else{request.metadata_policy=this.#q("#metadata").value;if(!request.metadata_policy||request.metadata_policy==="coordinate_text_field")throw new Error("Choose whether annotation comments are preserved or FreeText is synchronized");}
    return {operation:{kind:"scoped_text",request:{source:{scope:item.scope,request}}},policy:{allow_font_substitution:allowFont}};
  }
  async #render(){this.#clear();const version=this.#version;const request=await this.#request();if(version!==this.#version)return;
    this.#status("Planning the private candidate…");const plan=await this.#client.planScopedText(request);if(version!==this.#version)return;
    if(!["ready","approval_required"].includes(plan.state)){this.#q("#report").textContent=JSON.stringify(plan,null,2);throw new Error(`Edit not applicable: ${plan.state}`);}
    const pages=request.operation.request.source.scope==="widget_field"?[...new Set(request.operation.request.source.request.widgets.map(w=>w.appearance.target.page))]:[];
    if(pages.length>8)throw new Error("Review this field through the paginated preview API: the panel displays at most eight widget pages per candidate");
    this.#status("Rendering before/candidate pages…");const preview=await this.#client.previewScopedText(plan,{pages,dpi:Number(this.#q("#dpi").value),require_exact:this.#q("#exact").checked});if(version!==this.#version)return;
    // Decode both PNGs before enabling review. A successful backend render is
    // not evidence that the browser actually displayed the candidate images.
    for(const page of preview.pages){const row=document.createElement("div");row.className="pair";this.#q("#images").append(row);
      for(const [label,side]of [["Before",page.before],["Candidate",page.candidate]]){const figure=document.createElement("figure"),caption=document.createElement("figcaption"),img=document.createElement("img");caption.textContent=`${label}, page ${page.page}`;img.alt=caption.textContent;
        const url=URL.createObjectURL(new Blob([side.png],{type:"image/png"}));this.#urls.push(url);img.src=url;figure.append(caption,img);row.append(figure);await img.decode();if(version!==this.#version)return;}
    }
    this.#plan=plan;this.#preview=preview;
    this.#q("#report").textContent=JSON.stringify({plan_id:plan.plan_id,candidate_output_sha256:preview.candidate_output_sha256,
      affected_pages_not_previewed:preview.affected_pages_not_previewed,font:plan.preview.font,source:plan.preview.source,
      pages:preview.pages.map(p=>({page:p.page,difference:p.difference,before:p.before.diagnostics,candidate:p.candidate.diagnostics})),limitations:preview.limitations},null,2);
    this.#status("Native candidate displayed. This is not independent-render certification. Review and approve explicitly.");
  }
  async #apply(){if(!this.#plan||!this.#preview||!this.#q("#approve").checked)throw new Error("Review and approve a rendered candidate first");
    if(!this.dispatchEvent(new CustomEvent("beforescopededit",{bubbles:true,composed:true,cancelable:true})))throw new Error("Finish or clear the logical story draft before applying a native edit");
    const plan=this.#plan;const state=await this.#client.applyScopedText(plan,{selected_candidate_ids:plan.selected_candidate_ids,
      approved_font:plan.preview.font.required_approved_font??null,mutation_mode:plan.policy.mutation_mode,accept_visual_change:true,accept_signature_invalidation:false});
    this.#clear();this.#status(state.report?.changed?"Published native PDF edit. Undo retains the previous bytes; this is not sanitizing redaction.":"No edit published; inspect the operation report.");
    this.#q("#report").textContent=JSON.stringify(state.report,null,2);this.dispatchEvent(new CustomEvent("scopededit",{detail:state,bubbles:true,composed:true}));
  }
}
if(!customElements.get("wellfriend-scoped-text-editor"))customElements.define("wellfriend-scoped-text-editor",WellfriendScopedTextEditor);
