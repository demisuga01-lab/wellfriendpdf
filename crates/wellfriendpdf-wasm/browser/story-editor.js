import { pdfRectToCss, devicePointToPdf, sourceSelectionRange } from "./story-client.js";
import "./scoped-text-editor.js";
import "./story-history-editor.js";
import "./story-structure-editor.js";
import "./font-picker.js";

/** An embeddable, native-save editor for explicitly approved linked frames.
 * The side panel edits logical paragraphs; the page shows native saved pixels
 * and clearly labelled geometry previews. DOM text is never exported as a PDF. */
export class WellfriendStoryEditor extends HTMLElement {
  #client; #request; #preview; #receipt; #geometry; #model; #rect; #drag;
  #page = 1; #version = 0; #renderVersion = 0; #busy = false; #mutating = false;
  #timer; #objectUrl; #drawing = false; #autoPreview = false; #tagSources; #tagRevision;
  #imageSources; #imageRevision; #ocrSources; #formTextSources;
  #annotationSources; #annotationRevision;
  #historySource; #historyModel;
  constructor() {
    super();
    const root=this.attachShadow({mode:"open"});
    root.innerHTML=`<style>
      :host{display:block;color:#202124;background:#f4f5f7;font:14px system-ui;border:1px solid #d4d7dc;border-radius:12px;overflow:hidden}
      *{box-sizing:border-box} button,input,select,textarea{font:inherit}button,select,input[type=number]{padding:7px;border:1px solid #c7cbd1;border-radius:6px;background:white;color:inherit}
      button{cursor:pointer}button:disabled{opacity:.45;cursor:default}button:focus-visible,textarea:focus-visible,input:focus-visible{outline:3px solid #3668ce;outline-offset:2px}
      nav{display:flex;gap:8px;flex-wrap:wrap;align-items:center;padding:12px;background:white;border-bottom:1px solid #d4d7dc}
      main{display:grid;grid-template-columns:minmax(0,1fr) 310px;min-height:650px}section{overflow:auto;padding:24px;max-height:85vh}
      .paper{position:relative;max-width:100%;margin:auto;background:white;box-shadow:0 3px 12px #0002}.paper img{display:block;width:100%;pointer-events:none}
      .layer{position:absolute;inset:0}.frame{position:absolute;border:2px dashed #3962ac;background:#315fad15;min-width:6px;min-height:6px;padding:0;border-radius:2px}
      .figure-preview{position:absolute;border:2px dotted #7252a1;background:#7252a11a;pointer-events:none;font-size:11px;overflow:hidden;overflow-wrap:anywhere}
      .draw{touch-action:none;cursor:crosshair}.selection{position:absolute;border:2px solid #275ac1;pointer-events:none;background:#275ac122}
      aside{padding:14px;background:white;border-left:1px solid #d4d7dc;overflow:auto;max-height:85vh}textarea{width:100%;min-height:90px;resize:vertical;padding:8px;border:1px solid #c7cbd1;border-radius:6px}
      label{display:block;margin:8px 0}fieldset{border:1px solid #d9dde3;border-radius:8px;margin:12px 0;padding:10px}legend{max-width:240px;overflow-wrap:anywhere}
      .row{display:flex;gap:6px;align-items:center;flex-wrap:wrap}.small{font-size:12px;color:#555}.status{padding:10px 14px;margin:0;white-space:pre-wrap;overflow-wrap:anywhere;border-top:1px solid #d4d7dc}
      input[type=number]{width:85px}pre{white-space:pre-wrap;overflow-wrap:anywhere;max-height:160px;overflow:auto}.hidden{display:none!important}
      @media(max-width:800px){main{grid-template-columns:1fr}aside{border-left:0;border-top:1px solid #d4d7dc;max-height:none}section{padding:10px;max-height:65vh}}
    </style>
    <nav aria-label="PDF editing actions">
      <button data-action="open">Open PDF</button><button data-action="import">Import story</button>
      <select id="stories" aria-label="Saved story"><option value="">Choose saved story</option></select>
      <button data-action="new">New story</button><button data-action="clear-draft">Clear story draft</button><button data-action="undo">Undo</button><button data-action="redo">Redo</button>
      <button data-action="preview">Preview layout</button><button data-action="save">Apply approved edit</button>
      <button data-action="cancel">Cancel work</button><button data-action="download">Download PDF</button>
      <label>Page <select id="pages" aria-label="Page"></select></label>
    </nav>
    <main><section aria-label="PDF page"><div class="paper"><img alt="Native rendering of the last saved PDF page"><div class="layer"></div></div></section>
    <aside aria-label="Logical story editor">
      <p class="small">Dashed boxes are layout previews, not rendered replacement text. Apply an approved edit to see native PDF output.</p>
      <wellfriend-scoped-text-editor></wellfriend-scoped-text-editor>
      <button data-action="history">Begin text history from this draft</button>
      <button data-action="resume-history">Resume saved text history</button><button data-action="detach-history">Detach collaboration draft</button>
      <wellfriend-story-history-editor></wellfriend-story-history-editor>
      <button data-action="structure">Review structural branches from this draft</button><wellfriend-story-structure-editor></wellfriend-story-structure-editor>
      <label><input id="overflow" type="checkbox"> Allow continuation pages</label>
      <label><input id="fallback" type="checkbox" checked> Allow disclosed font substitution</label>
      <wellfriend-font-picker></wellfriend-font-picker>
      <label>Story writing mode <select id="writing-mode"><option value="horizontal_tb">Horizontal</option><option value="vertical_rl">Vertical, columns right to left</option><option value="vertical_lr">Vertical, columns left to right</option></select></label>
      <details id="source-panel"><summary>Select original source text and frame</summary>
        <button data-action="source">Load page source</button>
        <label>Source-order text (select the exact range)<textarea id="source" readonly spellcheck="false"></textarea></label>
        <button data-action="draw">Draw approved frame</button><button data-action="frame">Link selected text/frame</button>
        <div id="rect-inputs" class="row"><label>Left <input aria-label="Frame left" type="number" step="any"></label><label>Bottom <input aria-label="Frame bottom" type="number" step="any"></label><label>Right <input aria-label="Frame right" type="number" step="any"></label><label>Top <input aria-label="Frame top" type="number" step="any"></label></div>
        <p class="small">Logical source order is not inferred reading order. Select text, then draw its layout region. Linked frames must be in approved page order.</p>
      </details>
      <details><summary>Tagged PDF paragraph and Figure ownership</summary>
        <button data-action="tags">Load structure owners</button>
        <label>Existing sibling paragraph owners <select id="tag-owners" multiple size="6" style="width:100%"></select></label>
        <button data-action="bind-tags">Bind selected owners</button>
        <p class="small">Select complete sibling paragraphs and Figure owners in logical order. Assign image owners separately from captions below. New Figures require description review. Preview verifies exact source paint and text ownership; bounded contentless Figure descendants require explicit preservation, while content-bearing or shared subtrees require a separate migration policy.</p>
      </details>
      <details><summary>Annotation, popup and reply anchors</summary>
        <button data-action="annotations">Load annotation groups</button>
        <label>Anchor source <select id="annotation-sources" style="width:100%"></select></label>
        <pre id="annotation-group" aria-label="Complete annotation group membership"></pre>
        <div class="row"><label>Horizontal offset <input id="annotation-x" type="number" step="any" value="0"></label><label>Vertical offset <input id="annotation-y" type="number" step="any" value="0"></label></div>
        <label><input id="annotation-approve" type="checkbox"> Approve moving every listed group member together</label>
        <label><input id="annotation-rename" type="checkbox"> Allow destination-name collision repair (review every rename in preview)</label>
        <p class="small">Existing annotation names are page-local. Collision repair renames only arriving annotations. Script/FDF references to old names are not inferred or rewritten.</p>
        <p class="small">Offsets place the selected rectangle's lower-left corner relative to the paragraph's first line. All connected popups and replies keep their relative positions. Attach below, then review every destination in the native preview. Unlinking leaves annotations in the last saved PDF; it does not delete them.</p>
      </details>
      <details><summary>Native images and captions</summary>
        <button data-action="images">Load page images</button>
        <label>Source occurrence <select id="image-sources" style="width:100%"><option value="">Load images first</option></select></label>
        <label>Invisible OCR operands to move with this image <select id="image-ocr" multiple size="5" style="width:100%"></select></label>
        <label><input id="image-ocr-unrelated" type="checkbox"> Confirm page OCR is unrelated to this image (move none)</label>
        <p class="small">Select exact source operands, not matching words elsewhere. Select a complete ActualText owner. For nested images, carriers come only from the exact revision-bound Form occurrence. Unselected OCR stays in place; do not also select these glyphs as paragraph source text. For a tagged image, each chosen OCR span must belong to the Figure or an explicitly assigned content-only sibling owner. Bounded contentless Figure descendants can be preserved; other subtrees require an explicit migration. Boxes preview geometry only.</p>
      </details>
      <div id="figure-removals"></div><div id="paragraphs"></div><button data-action="paragraph">Add paragraph</button>
      <button data-action="table-values">Recalculate typed table values</button>
      <pre id="report" aria-label="Layout and substitution report"></pre>
      <label><input id="approve" type="checkbox"> Approve this exact layout, affected pages, fonts and semantic ownership decisions</label>
    </aside></main><p class="status" role="status" aria-live="polite">Attach a StoryWorkerClient, then open a PDF.</p>
    <input id="pdf-file" class="hidden" type="file" accept="application/pdf,.pdf"><input id="story-file" class="hidden" type="file" accept="application/json,.json">`;
    root.addEventListener("beforescopededit",event=>{if(this.#busy||this.#request)event.preventDefault();});
    root.addEventListener("fontprepared",event=>{
      const asset=event.detail.asset;
      if(this.#busy||!this.#request||this.#historySource||(this.#request.fonts?.length??0)>=128
        ||this.#request.fonts?.some(font=>font.lookup_name===asset.lookup_name)){event.preventDefault();return;}
      this.#request.fonts??=[];this.#request.fonts.push(structuredClone(asset));
      this.#invalidate(false);this.#paragraphs();this.#status(`Font ${asset.lookup_name} added to this draft. Select it on a paragraph and preview.`);
    });
    root.addEventListener("historydraft",event=>{
      if(this.#busy||JSON.stringify(this.#request)!==JSON.stringify(event.detail.expected)){event.preventDefault();return;}
      try{this.setRequest(event.detail.request);this.#historySource=structuredClone(event.detail.source);this.#historyModel=JSON.stringify(this.#request);}catch(error){event.preventDefault();this.#status(error.message??String(error));}
    });
    root.addEventListener("structuredraft",event=>{
      if(this.#busy||this.#historySource||JSON.stringify(this.#request)!==JSON.stringify(event.detail.expected)){event.preventDefault();return;}
      try{this.setRequest(event.detail.request);}catch(error){event.preventDefault();this.#status(error.message??String(error));}
    });
    root.addEventListener("scopededit",()=>this.#run(async()=>{this.#model=undefined;this.#invalidate(false);this.#refresh();await this.#render();}));
    root.addEventListener("click",(event)=>{const action=event.target.closest("button[data-action]")?.dataset.action;if(action)this.#run(()=>this.#action(action));});
    root.querySelector("#image-sources").addEventListener("change",()=>this.#imageSelection());
    root.querySelector("#annotation-sources").addEventListener("change",()=>this.#annotationSelection());
    root.querySelector("#image-ocr").addEventListener("change",()=>{if(this.#q("#image-ocr").selectedOptions.length)this.#q("#image-ocr-unrelated").checked=false;});
    root.querySelector("#image-ocr-unrelated").addEventListener("change",()=>{if(this.#q("#image-ocr-unrelated").checked)for(const option of this.#q("#image-ocr").options)option.selected=false;});
    root.querySelector("#pdf-file").addEventListener("change",(event)=>this.#run(async()=>{
      const file=event.target.files[0];if(!file)return;if(file.size>256*1024*1024)throw new Error("PDF exceeds 256 MiB");
      if(this.#busy)return;this.#busy=true;this.#mutating=true;this.#buttons();
      const version=this.#version;
      try{const bytes=new Uint8Array(await file.arrayBuffer());if(version!==this.#version)throw new DOMException("File opening cancelled","AbortError");await this.#client.open(bytes);this.#historySource=undefined;this.#historyModel=undefined;this.#request=undefined;this.#page=1;this.#invalidate(false);this.#refresh();await this.#render();}
      finally{this.#busy=false;this.#mutating=false;}
    }));
    root.querySelector("#story-file").addEventListener("change",(event)=>this.#run(async()=>{
      const file=event.target.files[0];if(!file)return;if(file.size>32*1024*1024)throw new Error("Story JSON exceeds 32 MiB");
      this.setRequest(JSON.parse(await file.text()));
    }));
    root.querySelector("#pages").addEventListener("change",(event)=>this.#run(async()=>{
      this.#page=Number(event.target.value);this.#model=undefined;this.#rect=undefined;this.#imageSources=undefined;this.#ocrSources=undefined;this.#formTextSources=undefined;this.#imageRevision=undefined;this.#q("#image-sources").replaceChildren();this.#q("#image-ocr").replaceChildren();this.#q("#source").value="";this.#paragraphs();await this.#render();
    }));
    root.querySelector("#stories").addEventListener("change",(event)=>{
      const story=this.#client.state?.stories.find(s=>s.request.story_id===event.target.value);if(story)this.setRequest(story.request);
    });
    const prune=document.createElement("label");prune.innerHTML='<input type="checkbox" id="prune"> Remove empty owned continuation pages';root.querySelector("#overflow").closest("label").after(prune);
    this.#q("#writing-mode").addEventListener("change",()=>{if(!this.#request)return;this.#request.writing_mode=this.#q("#writing-mode").value;this.#invalidate();});
    for(const id of ["overflow","fallback","prune"])root.querySelector(`#${id}`).addEventListener("change",()=>{
      if(!this.#request)return;this.#request[{overflow:"allow_page_creation",fallback:"allow_font_substitution",prune:"prune_empty_pages"}[id]]=this.#q(`#${id}`).checked;this.#invalidate();
    });
    this.#q("#approve").addEventListener("change",()=>this.#buttons());
    const layer=root.querySelector(".layer");
    layer.addEventListener("pointerdown",(event)=>{
      if(!this.#drawing||!this.#geometry||this.#busy)return;
      const r=layer.getBoundingClientRect();this.#drag={id:event.pointerId,start:[(event.clientX-r.left)/r.width,(event.clientY-r.top)/r.height]};layer.setPointerCapture(event.pointerId);event.preventDefault();
    });
    layer.addEventListener("pointerup",(event)=>{
      if(this.#drag?.id!==event.pointerId)return;const r=layer.getBoundingClientRect(),g=this.#geometry;
      const end=[Math.max(0,Math.min(1,(event.clientX-r.left)/r.width)),Math.max(0,Math.min(1,(event.clientY-r.top)/r.height))];
      const a=devicePointToPdf(this.#drag.start[0]*g.width,this.#drag.start[1]*g.height,g),b=devicePointToPdf(end[0]*g.width,end[1]*g.height,g);
      this.#rect=[Math.min(a[0],b[0]),Math.min(a[1],b[1]),Math.max(a[0],b[0]),Math.max(a[1],b[1])];
      [...this.shadowRoot.querySelectorAll("#rect-inputs input")].forEach((input,i)=>{input.value=this.#rect[i];});
      layer.releasePointerCapture(event.pointerId);this.#drag=undefined;this.#drawing=false;layer.classList.remove("draw");this.#overlays();
    });
    layer.addEventListener("pointercancel",()=>{this.#drag=undefined;this.#drawing=false;layer.classList.remove("draw");});
    this.#buttons();
  }
  #q(selector){return this.shadowRoot.querySelector(selector);}
  #imageSelection(){
    const image=this.#imageSources?.[Number(this.#q("#image-sources").value)],carriers=this.#q("#image-ocr");
    carriers.replaceChildren();this.#q("#image-ocr-unrelated").checked=false;
    if(!image){this.#ocrSources=[];return;}
    if(image.invocation_path?.length){
      const path=JSON.stringify(image.invocation_path),occurrence=this.#formTextSources?.occurrences?.find(item=>item.target.content_stream_index===image.content_stream_index&&JSON.stringify(item.target.invocation_path)===path);
      this.#ocrSources=occurrence&&!occurrence.external_actual_text_owner?occurrence.text.source_spans.filter(span=>span.text_render_mode===3).map(span=>({...span,form_target:occurrence.target})):[];
    }else this.#ocrSources=(this.#formTextSources?.page_spans??[]).map(span=>({...span,form_target:null}));
    for(const span of this.#ocrSources){const option=new Option(`${span.span_id}: ${span.text.slice(0,160)}${span.flow_relocatable?"":" (Figure ownership requires native validation)"}`,span.span_id);carriers.add(option);}
  }
  set client(client){if(this.#busy)throw new Error("Cannot replace a busy editor client");this.#q("wellfriend-scoped-text-editor").client=client;this.#q("wellfriend-story-history-editor").client=client;this.#q("wellfriend-story-structure-editor").client=client;this.#q("wellfriend-font-picker").client=client;this.#client=client;this.#refresh();}
  get client(){return this.#client;}
  setRequest(request){
    if(this.#mutating)throw new Error("Wait for the current checkpoint or cancel it before replacing the draft");
    if(!this.#client?.state||request.input_sha256!==this.#client.state.revision)throw new Error("Story source revision differs from the open PDF");
    if(!Array.isArray(request.paragraphs)||!Array.isArray(request.frames)||request.paragraphs.length>100000)throw new Error("Invalid story schema");
    this.#historySource=undefined;this.#historyModel=undefined;this.#request=structuredClone(request);this.#q("#overflow").checked=!!request.allow_page_creation;this.#q("#fallback").checked=!!request.allow_font_substitution;this.#q("#prune").checked=!!request.prune_empty_pages;
    this.#q("#writing-mode").value=request.writing_mode??"horizontal_tb";
    this.#invalidate(false);this.#paragraphs();this.#overlays();this.#status("Story loaded. Review the source frames and preview your changes.");
  }
  get request(){return this.#request?structuredClone(this.#request):undefined;}
  #status(text){this.#q(".status").textContent=text;}
  async #run(action){try{await action();}catch(error){if(error.name!=="AbortError")this.#status(error.message??String(error));}finally{this.#buttons();}}
  #invalidate(schedule=true){this.#autoPreview=schedule;this.#version++;this.#receipt=undefined;this.#preview=undefined;this.#q("#approve").checked=false;this.#q("#report").textContent="Unsaved logical changes; preview required.";clearTimeout(this.#timer);this.#buttons();
    if(schedule)this.#timer=setTimeout(()=>{if(!this.#busy)this.#run(()=>this.#previewLayout());},300);}
  #buttons(){
    const open=!!this.#client?.state;
    this.#q("wellfriend-font-picker").disabled=!open||this.#busy||!this.#request||!!this.#historySource;
    for(const button of this.shadowRoot.querySelectorAll("button[data-action]")){
      const a=button.dataset.action;button.disabled=!this.#client||(this.#busy&&a!=="cancel")||(!open&&a!=="open");
      if(a==="save")button.disabled ||= !this.#receipt||!this.#q("#approve").checked;
      if(a==="undo")button.disabled ||= !this.#client?.canUndo;if(a==="redo")button.disabled ||= !this.#client?.canRedo;
      if(a==="table-values")button.disabled ||= !this.#request?.table_layout;
      if(a==="paragraph"||a==="frame"||a==="bind-tags")button.disabled ||= !!this.#request?.table_layout;
    }
    for(const input of this.shadowRoot.querySelectorAll("aside input,aside textarea,aside select,#pages,#stories"))input.disabled=this.#mutating;
  }
  #refresh(){
    this.#annotationSources=undefined;this.#annotationRevision=undefined;this.#q("#annotation-sources").replaceChildren();this.#annotationSelection();
    this.#tagSources=undefined;this.#tagRevision=undefined;this.#q("#tag-owners").replaceChildren();
    this.#imageSources=undefined;this.#imageRevision=undefined;this.#ocrSources=undefined;this.#formTextSources=undefined;this.#q("#image-sources").replaceChildren();this.#q("#image-ocr").replaceChildren();this.#q("#image-ocr-unrelated").checked=false;
    const state=this.#client?.state;for(const [selector,values,label] of [["#pages",state?.pages??[],p=>[p.page,`Page ${p.page}`]],["#stories",state?.stories??[],s=>[s.request.story_id,s.request.story_id]]]){
      const select=this.#q(selector);select.replaceChildren();if(selector==="#stories")select.add(new Option("Choose saved story",""));
      for(const v of values){const [id,text]=label(v);select.add(new Option(text,String(id)));}
    }
    if(state)this.#page=Math.min(this.#page,state.pages.length)||1;this.#q("#pages").value=String(this.#page);this.#paragraphs();this.#buttons();
  }
  #paragraphs(){
    const host=this.#q("#paragraphs");host.replaceChildren();
    const removed=this.#q("#figure-removals");removed.replaceChildren();
    const savedStory=this.#client?.state?.stories.find(s=>s.request.story_id===this.#request?.story_id)?.request;
    for(const removal of this.#request?.figure_removals??[]){
      const row=document.createElement("div"),label=document.createElement("p"),cancel=document.createElement("button");
      label.textContent=`Pending image deletion: ${removal.figure_id}. Removes this painted occurrence only, not historical bytes. Caption text is separate.`;
      cancel.textContent="Cancel image deletion";const original=savedStory?.figures?.find(f=>f.id===removal.figure_id);cancel.disabled=!original;
      cancel.addEventListener("click",()=>this.#run(async()=>{if(this.#busy||!original)return;
        if(this.#request.figures?.some(f=>f.id===original.id||f.caption_paragraph===original.caption_paragraph))throw new Error("Remove the conflicting draft figure before restoring this caption association");
        if(!this.#request.paragraphs.some(p=>p.id===original.caption_paragraph)){const caption=savedStory.paragraphs.find(p=>p.id===original.caption_paragraph);if(!caption)throw new Error("Saved caption is missing");this.#request.paragraphs.push(structuredClone(caption));
          if(this.#request.source_tags&&savedStory.source_tags){this.#request.source_tags.paragraph_sources??={};this.#setRecord(this.#request.source_tags.paragraph_sources,caption.id,structuredClone(this.#record(savedStory.source_tags.paragraph_sources,caption.id)??null));}
        }
        this.#request.figures??=[];this.#request.figures.push(structuredClone(original));this.#request.figure_removals=this.#request.figure_removals.filter(r=>r!==removal);const tagBinding=this.#request.source_tags?this.#record(this.#request.source_tags.figures,removal.figure_id):null;if(tagBinding)tagBinding.delete_semantic_subtree=false;this.#invalidate();this.#paragraphs();
      }));row.append(label,cancel);removed.append(row);
    }
    const reviewedOwners=new Set();
    const cellsByParagraph=new Map();for(const cell of this.#request?.table_layout?.cells??[]){for(const id of cell.paragraph_ids?.length?cell.paragraph_ids:[cell.id])cellsByParagraph.set(id,cell);}
    const positions=new Map();let position=0;for(const cell of [...(this.#request?.table_layout?.cells??[])].sort((a,b)=>a.row-b.row||a.column-b.column)){for(const id of cell.paragraph_ids?.length?cell.paragraph_ids:[cell.id])positions.set(id,position++);}
    const displayed=[...(this.#request?.paragraphs??[]).entries()];if(this.#request?.table_layout)displayed.sort((a,b)=>(positions.get(a[1].id)??Infinity)-(positions.get(b[1].id)??Infinity));
    for(const [index,p] of displayed){
      const cell=cellsByParagraph.get(p.id),blockIds=cell?(cell.paragraph_ids?.length?cell.paragraph_ids:[cell.id]):[],blockIndex=blockIds.indexOf(p.id);
      const group=document.createElement("fieldset"),legend=document.createElement("legend");legend.textContent=cell?`Cell ${cell.id} · paragraph ${blockIndex+1}`:`Paragraph ${index+1}`;group.append(legend);
      const text=document.createElement("textarea");text.value=p.text;text.dir=p.rtl?"rtl":"auto";text.setAttribute("aria-label",`Paragraph ${index+1} text`);text.dataset.paragraph=p.id;
      text.readOnly=!!cell?.value&&cell.value.kind!=="text";
      if(text.readOnly)text.title="Typed numeric/formula cell: edit the table value in request JSON, then recalculate and preview.";
      text.addEventListener("input",()=>{p.text=text.value;if(cell?.value?.kind==="text")cell.value.text=text.value;this.#invalidate();});group.append(text);
      const size=document.createElement("input");size.type="number";size.min="1";size.max="1000";size.step=".1";size.value=p.font_size;size.setAttribute("aria-label","Font size");
      size.addEventListener("change",()=>{const n=Number(size.value);if(n>0&&n<=1000){p.font_size=n;p.line_height=Math.max(p.line_height,n*1.2);this.#invalidate();}});group.append(size);
      const fontLabel=document.createElement("label"),fontChoice=document.createElement("select");fontLabel.textContent="Paragraph font ";fontChoice.setAttribute("aria-label",`Paragraph ${index+1} font`);
      for(const name of new Set([p.preferred_font,...(this.#request.fonts??[]).map(font=>font.lookup_name)]))fontChoice.add(new Option(name,name));
      fontChoice.value=p.preferred_font;fontChoice.addEventListener("change",()=>{if(this.#busy)return;p.preferred_font=fontChoice.value;this.#invalidate();});fontLabel.append(fontChoice);group.append(fontLabel);
      this.#lineBreakControls(group,p);
      if(!this.#request.table_layout){
        const pageLabel=document.createElement("label"),pageBreak=document.createElement("select");pageLabel.textContent="Start on ";
        for(const [label,value]of [["current flow position","none"],["next page","next_page"],["next odd page","next_odd_page"],["next even page","next_even_page"]])pageBreak.add(new Option(label,value));
        pageBreak.value=p.page_break_before??"none";pageBreak.setAttribute("aria-label",`Paragraph ${index+1} physical page break`);
        pageBreak.addEventListener("change",()=>{if(this.#busy)return;if(pageBreak.value==="none")delete p.page_break_before;else p.page_break_before=pageBreak.value;if(p.page_break_before)p.break_before=false;this.#invalidate();});pageLabel.append(pageBreak);group.append(pageLabel);
        const forced=[...p.text].filter(value=>value==="\f").length,note=document.createElement("span");note.className="small";note.textContent=` ${forced} in-paragraph page break${forced===1?"":"s"}`;group.append(note);
        const insert=document.createElement("button");insert.type="button";insert.textContent="Insert page break at cursor";insert.disabled=!!this.#request.figures?.some(figure=>figure.caption_paragraph===p.id);
        insert.title=insert.disabled?"A native figure and its caption must stay on one page.":"Insert U+000C and force the following text onto a later physical page.";
        insert.addEventListener("click",()=>{if(this.#busy||insert.disabled)return;text.setRangeText("\f",text.selectionStart,text.selectionEnd,"end");p.text=text.value;this.#invalidate();this.#paragraphs();});group.append(insert);
      }
      this.#annotationControls(group,p);
      const figure=this.#request.figures?.find(f=>f.caption_paragraph===p.id);
      if(figure){
        const note=document.createElement("p");note.textContent=`Caption for ${figure.id}. Image and caption stay in one frame.`;group.append(note);let subtreeDeletionApproval=null;
        const ocrNote=document.createElement("p");ocrNote.className="small";ocrNote.textContent=figure.ocr?`Move ${figure.ocr.span_ids.length} exact OCR operands: ${figure.ocr.expected_text.slice(0,300)}`:figure.ocr_unrelated?"Approved: page OCR is unrelated and stays in place.":figure.source.kind==="owned"?"Saved native group: any captured OCR moves and is deleted with this image.":"No OCR association selected.";group.append(ocrNote);
        for(const [key,title]of [["width","Image width"],["height","Image height"],["gap","Caption gap"]]){
          const label=document.createElement("label"),input=document.createElement("input");label.textContent=`${title} `;input.type="number";input.step="any";input.min=key==="gap"?"0":"0.001";input.max="1000000";input.value=figure[key]??0;input.setAttribute("aria-label",`${title} for ${figure.id}`);
          input.addEventListener("change",()=>{if(this.#busy)return;const value=Number(input.value);if(Number.isFinite(value)&&value>=Number(input.min)&&value<=1e6){figure[key]=value;this.#invalidate();}});label.append(input);group.append(label);
        }
        for(const [key,values]of [["alignment",["left","center","right"]],["stack",["background","foreground"]]]){
          const label=document.createElement("label"),select=document.createElement("select");label.textContent=`Image ${key} `;for(const value of values)select.add(new Option(value,value));select.value=figure[key]??values[0];select.addEventListener("change",()=>{if(this.#busy)return;figure[key]=select.value;this.#invalidate();});label.append(select);group.append(label);
        }
        if(this.#request.source_tags){
          const tags=this.#request.source_tags,binding=this.#record(tags.figures,figure.id),label=document.createElement("label"),select=document.createElement("select");label.textContent="Figure semantic owner ";
          select.add(new Option("Choose Figure owner","unbound"));select.add(new Option("New Figure (untagged source image)","new"));
          for(const [i,owner]of tags.selected.entries()){const source=this.#tagSource(owner);select.add(new Option(`Selected owner ${i+1} (/${source?.role??"unknown"}, object ${owner.object})`,String(i)));}
          const source=binding?.source,selected=source?tags.selected.findIndex(v=>v.key&&source.key?v.key===source.key:v.object===source.object&&(v.generation??0)===(source.generation??0)):-1;select.value=!binding?"unbound":source?String(selected):"new";
          select.addEventListener("change",()=>{if(this.#busy||select.value==="unbound")return;tags.figures??={};this.#setRecord(tags.figures,figure.id,{source:select.value==="new"?null:structuredClone(tags.selected[Number(select.value)]),semantic_text:select.value==="new"?{alternate:null,expansion:null}:null,split_reused_form_semantics:false,outbound_ref_split:null,inbound_ref_split:null,preserve_semantic_subtree:false,delete_semantic_subtree:false,clone_semantic_subtree_for_reused_form:false,separate_ocr_owner:null,separate_ocr_owners:[]});this.#invalidate();this.#paragraphs();});label.append(select);group.append(label);
          if(binding){
            this.#semanticReview(group,`Figure ${figure.id} descriptions`,binding.source,binding.semantic_text,value=>{binding.semantic_text=value;});
            if(binding.source){const subtreeLabel=document.createElement("label"),subtree=document.createElement("input");subtree.type="checkbox";subtree.checked=!!binding.preserve_semantic_subtree;subtree.setAttribute("aria-label",`Preserve semantic subtree for ${figure.id}`);subtree.addEventListener("change",()=>{if(this.#busy)return;binding.preserve_semantic_subtree=subtree.checked;if(!subtree.checked){binding.delete_semantic_subtree=false;binding.clone_semantic_subtree_for_reused_form=false;}this.#invalidate();this.#paragraphs();});subtreeLabel.append(subtree,document.createTextNode(" Preserve contentless semantic descendants under this Figure"));group.append(subtreeLabel);if(binding.preserve_semantic_subtree){const subtreeNote=document.createElement("p");subtreeNote.className="small";subtreeNote.textContent="The complete descendant tree is retained only when descendants own no text, paint, OBJR or marked-content items. Valid descendant page bindings follow the Figure destination page. A reused-Form split requires separate clone approval; internal relationships follow the clone, while external relationships require the outbound/incoming decisions below.";group.append(subtreeNote);const deleteLabel=document.createElement("label");subtreeDeletionApproval=document.createElement("input");subtreeDeletionApproval.type="checkbox";subtreeDeletionApproval.setAttribute("aria-label",`Delete complete semantic subtree with ${figure.id}`);deleteLabel.append(subtreeDeletionApproval,document.createTextNode(" Delete the complete validated semantic subtree with this image"));group.append(deleteLabel);}}
            if(binding.source&&figure.ocr){
              const sameTag=(left,right)=>!!left&&!!right&&(left.key&&right.key?left.key===right.key:left.object===right.object&&(left.generation??0)===(right.generation??0));
              const candidates=tags.selected.map((owner,index)=>({owner,index,source:this.#tagSource(owner)})).filter(candidate=>["P","Span"].includes(candidate.source?.role)&&candidate.source?.text_leaf&&!sameTag(candidate.owner,binding.source));
              const assignment=new Map();
              for(const spanId of figure.ocr.span_ids){
                const explicit=binding.separate_ocr_owners?.find(owner=>owner.span_ids?.includes(spanId));
                const legacy=binding.separate_ocr_owner;
                const owner=explicit??(legacy&&(!legacy.span_ids?.length||legacy.span_ids.includes(spanId))?legacy:null);
                const index=owner?tags.selected.findIndex(candidate=>sameTag(candidate,owner.source)):-1;
                assignment.set(spanId,index<0?"":String(index));
              }
              const rebuildOwners=()=>{
                const grouped=new Map();
                for(const [spanId,index]of assignment){if(index==="")continue;const spans=grouped.get(index)??[];spans.push(spanId);grouped.set(index,spans);}
                binding.separate_ocr_owner=null;
                binding.separate_ocr_owners=[...grouped].map(([index,span_ids])=>({source:structuredClone(tags.selected[Number(index)]),policy:"merge_into_figure",span_ids}));
                this.#invalidate();
              };
              for(const [spanIndex,spanId]of figure.ocr.span_ids.entries()){
                const ocrLabel=document.createElement("label"),ocrOwner=document.createElement("select");
                ocrLabel.textContent=`OCR span ${spanIndex+1} semantic owner `;
                ocrOwner.setAttribute("aria-label",`Semantic owner for OCR span ${spanIndex+1} of ${figure.id}`);
                ocrOwner.add(new Option("Same Figure owner",""));
                for(const candidate of candidates)ocrOwner.add(new Option(`Consume selected owner ${candidate.index+1} (/${candidate.source.role}, object ${candidate.owner.object})`,String(candidate.index)));
                ocrOwner.value=assignment.get(spanId)??"";
                ocrOwner.addEventListener("change",()=>{if(this.#busy)return;assignment.set(spanId,ocrOwner.value);rebuildOwners();});
                ocrLabel.append(ocrOwner);group.append(ocrLabel);
              }
              const note=document.createElement("p");note.className="small";note.textContent="Each exact OCR span may stay with the Figure or consume one explicitly selected content-only /P or /Span sibling. One owner may cover several spans; different owners remain separate entries. Merge removes only approved empty owners. Independent semantics, relationships, overlapping spans and inferred associations refuse.";group.append(note);
            }
            if(binding.source){const splitLabel=document.createElement("label"),split=document.createElement("input");split.type="checkbox";split.checked=!!binding.split_reused_form_semantics;split.setAttribute("aria-label",`Split reused Form semantics for ${figure.id}`);split.addEventListener("change",()=>{if(this.#busy)return;binding.split_reused_form_semantics=split.checked;if(!split.checked){binding.outbound_ref_split=null;binding.inbound_ref_split=null;binding.clone_semantic_subtree_for_reused_form=false;}this.#invalidate();this.#paragraphs();});splitLabel.append(split,document.createTextNode(" Split a reused Form occurrence while preserving residual Figure and approved OCR-owner semantics"));group.append(splitLabel);
              if(binding.split_reused_form_semantics){const refLabel=document.createElement("label"),refPolicy=document.createElement("select");refLabel.textContent="Outbound /Ref ownership ";for(const [label,value]of [["No relationship decision",""],["Move with selected occurrence","move_with_selected"],["Retain with residual occurrences","retain_with_residual"],["Copy to both owners","copy_to_both"]])refPolicy.add(new Option(label,value));refPolicy.value=binding.outbound_ref_split??"";refPolicy.addEventListener("change",()=>{if(this.#busy)return;binding.outbound_ref_split=refPolicy.value||null;this.#invalidate();});refLabel.append(refPolicy);group.append(refLabel);
                const incomingLabel=document.createElement("label"),incomingPolicy=document.createElement("select");incomingLabel.textContent="Incoming /Ref ownership ";for(const [label,value]of [["No relationship decision",""],["Follow selected occurrence","follow_selected"],["Retarget to residual occurrences","retarget_residual"],["Reference both owners","reference_both"]])incomingPolicy.add(new Option(label,value));incomingPolicy.value=binding.inbound_ref_split??"";incomingPolicy.addEventListener("change",()=>{if(this.#busy)return;binding.inbound_ref_split=incomingPolicy.value||null;this.#invalidate();});incomingLabel.append(incomingPolicy);group.append(incomingLabel);if(binding.preserve_semantic_subtree){const cloneLabel=document.createElement("label"),clone=document.createElement("input");clone.type="checkbox";clone.checked=!!binding.clone_semantic_subtree_for_reused_form;clone.setAttribute("aria-label",`Clone semantic subtree for residual ${figure.id}`);clone.addEventListener("change",()=>{if(this.#busy)return;binding.clone_semantic_subtree_for_reused_form=clone.checked;this.#invalidate();});cloneLabel.append(clone,document.createTextNode(" Clone the approved contentless subtree for residual Form occurrences"));group.append(cloneLabel);}}}
          }
        }
        const persisted=savedStory?.figures?.find(f=>f.id===figure.id),remove=document.createElement("button");
        remove.textContent=persisted?"Queue image deletion":"Cancel image attachment";
        remove.title=persisted?"Delete this saved image occurrence after preview approval. Caption text is retained. This is not redaction.":"Leave the original PDF image untouched.";
        remove.addEventListener("click",()=>this.#run(async()=>{if(this.#busy)return;
          if(persisted){if(persisted.source.kind!=="owned")throw new Error("Reload the saved native figure binding first");const tagBinding=this.#request.source_tags?this.#record(this.#request.source_tags.figures,figure.id):null;if(tagBinding?.preserve_semantic_subtree){if(!subtreeDeletionApproval?.checked)throw new Error("Approve deletion of the complete semantic subtree first");tagBinding.delete_semantic_subtree=true;}else if(tagBinding)tagBinding.delete_semantic_subtree=false;this.#request.figure_removals??=[];if(this.#request.figure_removals.some(r=>r.figure_id===figure.id))throw new Error("This figure already has a pending deletion");this.#request.figure_removals.push({figure_id:figure.id,binding:structuredClone(persisted.source.binding)});}
          if(!persisted&&this.#request.source_tags?.figures)delete this.#request.source_tags.figures[figure.id];
          this.#request.figures=this.#request.figures.filter(f=>f!==figure);this.#invalidate();this.#paragraphs();
        }));group.append(remove);
      }else if(!this.#request.table_layout){
        const attach=document.createElement("button");attach.textContent="Attach chosen image as figure";attach.disabled=this.#imageRevision!==this.#client?.state?.revision;
        attach.addEventListener("click",()=>this.#run(async()=>{
          if(this.#busy)return;if(this.#imageRevision!==this.#client?.state?.revision)throw new Error("Reload image occurrences for the current PDF revision");
          const image=this.#imageSources?.[Number(this.#q("#image-sources").value)];if(!image)throw new Error("Select an image occurrence");
          if(this.#request.figures?.some(f=>f.source.kind==="occurrence"&&f.source.occurrence_id===image.occurrence_id&&f.source.content_stream_index===image.content_stream_index))throw new Error("This image is already attached to a paragraph");
          const natural=image.bbox[2]-image.bbox[0],height=image.bbox[3]-image.bbox[1],frame=this.#request.frames[0]?.rect;
          if(!(natural>0&&height>0)||!image.bbox.every(Number.isFinite))throw new Error("Image has no finite placement rectangle");
          const width=Math.min(natural,frame?frame[2]-frame[0]:180);if(!(width>0))throw new Error("Approve a nonempty frame first");
          const selected=new Set([...this.#q("#image-ocr").selectedOptions].map(option=>option.value));
          const spans=(this.#ocrSources??[]).filter(span=>selected.has(span.span_id));
          if(spans.length!==selected.size)throw new Error("Reload OCR source operands for this PDF revision");
          if(this.#request.figures?.some(f=>f.ocr?.span_ids.some(id=>selected.has(id))))throw new Error("One OCR operand cannot belong to two images");
          const formTarget=image.invocation_path?.length?spans[0]?.form_target:null;if(spans.some(span=>JSON.stringify(span.form_target)!==JSON.stringify(formTarget)))throw new Error("Selected OCR operands do not share one exact Form occurrence");
          const ocr=spans.length?{span_ids:spans.map(s=>s.span_id),expected_text:spans.map(s=>s.text).join(""),...(formTarget?{form_target:structuredClone(formTarget)}:{})}:null;
          const ocr_unrelated=this.#q("#image-ocr-unrelated").checked;
          if(ocr&&ocr_unrelated)throw new Error("Choose an OCR association or confirm none, not both");
          this.#request.figures??=[];this.#request.figures.push({id:`figure-${crypto.randomUUID()}`,caption_paragraph:p.id,source:{kind:"occurrence",page:image.page,content_stream_index:image.content_stream_index,occurrence_id:image.occurrence_id},ocr,ocr_unrelated,width,height:height*width/natural,gap:6,alignment:"left",stack:"foreground"});
          for(const option of this.#q("#image-ocr").options)option.selected=false;
          this.#q("#image-ocr-unrelated").checked=false;
          this.#invalidate();this.#paragraphs();
        }));group.append(attach);
      }
      if(this.#request.source_tags){
        const tags=this.#request.source_tags,label=document.createElement("label"),select=document.createElement("select");label.textContent="Semantic owner ";
        select.add(new Option("New paragraph (P)","new"));
        for(const [i,owner]of tags.selected.entries())select.add(new Option(`Reuse selected owner ${i+1} (${owner.object})`,String(i)));
        const current=Object.hasOwn(tags.paragraph_sources??{},p.id)?tags.paragraph_sources[p.id]:(tags.selected.length===this.#request.paragraphs.length?tags.selected[index]:null);
        const selected=current?tags.selected.findIndex(v=>v.key&&current.key?v.key===current.key:v.object===current.object&&(v.generation??0)===(current.generation??0)):-1;select.value=selected<0?"new":String(selected);
        select.addEventListener("change",()=>{this.#ensureTagBindings();this.#setRecord(tags.paragraph_sources,p.id,select.value==="new"?null:structuredClone(tags.selected[Number(select.value)]));if(tags.new_roles)delete tags.new_roles[p.id];if(tags.semantic_text)delete tags.semantic_text[p.id];this.#invalidate();this.#paragraphs();});
        label.append(select);group.append(label);
        const original=this.#tagSource(current);
        const reviewed=this.#record(tags.semantic_text,p.id),reviewLabel=document.createElement("label"),review=document.createElement("input");
        review.type="checkbox";review.checked=!!reviewed;reviewLabel.append(review,document.createTextNode(" Review alternate/expanded text (blank removes)"));group.append(reviewLabel);
        const semanticInputs={};
        for(const [name,caption]of [["alternate","Alternate description"],["expansion","Expanded wording"]]){
          const label=document.createElement("label"),input=document.createElement("textarea");label.textContent=caption;
          input.value=(reviewed?reviewed[name]:original?.[name])??"";input.setAttribute("aria-label",`${caption} for paragraph ${index+1}`);label.append(input);group.append(label);semanticInputs[name]=input;
        }
        const applySemantic=()=>{tags.semantic_text??={};this.#storeReview(tags.semantic_text,p.id,review.checked?{alternate:semanticInputs.alternate.value||null,expansion:semanticInputs.expansion.value||null}:null);this.#invalidate();};
        review.addEventListener("change",applySemantic);
        for(const input of Object.values(semanticInputs))input.addEventListener("input",()=>{review.checked=true;applySemantic();});
      }
      const tableTags=this.#request.table_layout?.tagging;
      const blockOwned=!!tableTags&&blockIds.every(id=>Object.hasOwn(tableTags.blocks??{},id));
      if(tableTags){
        const cellId=cell?.id??p.id,meaning=this.#record(tableTags.semantics,cellId),summary=document.createElement("p");
        summary.textContent=`/${meaning?.role??"Unbound"}; scope: ${meaning?.scope??"none"}; headers: ${(meaning?.headers??[]).join(", ")||"none"}. Structure changes use the approved table request.`;group.append(summary);
        if(blockIndex<=0)this.#semanticReview(group,`Cell ${cellId} descriptions`,this.#record(tableTags.cells,cellId),this.#record(tableTags.semantic_text,cellId),value=>{tableTags.semantic_text??={};this.#storeReview(tableTags.semantic_text,cellId,value);});
        const block=this.#record(tableTags.blocks,p.id),path=block?.path??this.#record(tableTags.content_paths,cellId)??[];
        for(const [depth,owner]of path.entries()){const identity=owner.source.key??`${owner.source.object}:${owner.source.generation??0}`;
          if(reviewedOwners.has(identity)){const note=document.createElement("p");note.textContent=`Shared semantic owner ${depth+1}: use its review control in the earlier paragraph.`;group.append(note);continue;}
          reviewedOwners.add(identity);this.#semanticReview(group,`Paragraph ${p.id} owner ${depth+1} (shared owners reviewed together)`,owner.source,owner.semantic_text,value=>{this.#reviewTableOwner(tableTags,owner,value);});}
        if(block&&path.length===0)this.#semanticReview(group,`New paragraph ${p.id} descriptions`,null,block.semantic_text,value=>{block.semantic_text=value;});
        if(cell&&!blockOwned&&blockIds.length===1){const convert=document.createElement("button");convert.textContent="Separate paragraph tags";
          convert.addEventListener("click",()=>{if(this.#busy)return;tableTags.blocks??={};this.#storeReview(tableTags.blocks,p.id,{path:structuredClone(this.#record(tableTags.content_paths,cellId)??[])});if(tableTags.content_paths)delete tableTags.content_paths[cellId];cell.paragraph_ids=[p.id];this.#invalidate();this.#paragraphs();});group.append(convert);}
      }
      for(const [name,delta] of [["Move up",-1],["Move down",1],["Delete",0]]){const button=document.createElement("button");button.textContent=name;
        button.disabled=this.#request.table_layout?(!cell||!!cell.value||!!tableTags&&!blockOwned||(delta===0?blockIds.length<=1:blockIndex+delta<0||blockIndex+delta>=blockIds.length)):delta!==0&&(index+delta<0||index+delta>=this.#request.paragraphs.length);
        if(figure&&delta===0){button.disabled=true;button.title="Queue image deletion or cancel its attachment before deleting the caption.";}
        button.addEventListener("click",()=>{if(this.#busy||button.disabled)return;
          if(cell){cell.paragraph_ids=[...blockIds];if(delta===0){cell.paragraph_ids.splice(blockIndex,1);this.#request.paragraphs.splice(index,1);if(blockOwned)delete tableTags.blocks[p.id];}else{const ids=cell.paragraph_ids;[ids[blockIndex],ids[blockIndex+delta]]=[ids[blockIndex+delta],ids[blockIndex]];}}
          else{this.#ensureTagBindings();if(delta===0){this.#request.paragraphs.splice(index,1);if(this.#request.source_tags){delete this.#request.source_tags.paragraph_sources[p.id];if(this.#request.source_tags.new_roles)delete this.#request.source_tags.new_roles[p.id];if(this.#request.source_tags.semantic_text)delete this.#request.source_tags.semantic_text[p.id];}}else{const a=this.#request.paragraphs;[a[index],a[index+delta]]=[a[index+delta],a[index]];}}
          this.#invalidate();this.#paragraphs();});group.append(button);}
      if(cell&&blockIndex===blockIds.length-1){const add=document.createElement("button");add.textContent="Add paragraph in cell";add.disabled=!!cell.value||!!tableTags&&!blockOwned;
        add.addEventListener("click",()=>{if(this.#busy||add.disabled)return;const next={...structuredClone(p),id:`block-${crypto.randomUUID()}`,text:"",keep_with_next:false,keep_together:false,break_before:false,page_break_before:"none"};cell.paragraph_ids=[...blockIds,next.id];if(blockOwned)this.#storeReview(tableTags.blocks,next.id,{path:[],new_role:"P"});this.#request.paragraphs.splice(index+1,0,next);this.#invalidate();this.#paragraphs();});group.append(add);}
      host.append(group);
    }
    const table=this.#request?.table_layout,tags=table?.tagging;
    if(tags){
      this.#semanticReview(host,"Table description",tags.source,tags.table_text,value=>{tags.table_text=value;});
      for(const group of tags.groups??[])this.#semanticReview(host,`/${group.role} ${group.id} description`,group.source,group.semantic_text,value=>{group.semantic_text=value;});
      for(const row of table.rows)this.#semanticReview(host,`Row ${row.id} description`,this.#record(tags.rows,row.id),this.#record(tags.row_text,row.id),value=>{tags.row_text??={};this.#storeReview(tags.row_text,row.id,value);});
    }
  }
  #record(record,id){return record&&Object.hasOwn(record,id)?record[id]:undefined;}
  #setRecord(record,id,value){Object.defineProperty(record,id,{value,enumerable:true,writable:true,configurable:true});}
  #storeReview(record,id,value){if(value===null)delete record[id];else this.#setRecord(record,id,value);}
  #reviewTableOwner(tags,owner,value){const same=(a,b)=>a.key&&b.key?a.key===b.key:!a.key&&!b.key&&a.object===b.object&&(a.generation??0)===(b.generation??0);
    const paths=[...Object.values(tags.content_paths??{}),...Object.values(tags.blocks??{}).map(block=>block.path??[])];for(const path of paths)for(const item of path)if(same(item.source,owner.source))item.semantic_text=structuredClone(value);}
  #tagSource(owner){return this.#tagSources?.find(tag=>owner&&(owner.key?tag.reference.key===owner.key||tag.stable_keys?.includes(owner.key):tag.reference.object===owner.object&&(tag.reference.generation??0)===(owner.generation??0)));}
  #semanticReview(host,title,owner,reviewed,apply){
    const details=document.createElement("details"),summary=document.createElement("summary");summary.textContent=title;details.append(summary);host.append(details);let initialized=false;
    details.addEventListener("toggle",()=>{
      if(!details.open||initialized)return;initialized=true;
      const original=this.#tagSource(owner);
      const label=document.createElement("label"),review=document.createElement("input");review.type="checkbox";review.checked=!!reviewed;review.disabled=this.#mutating;label.append(review,document.createTextNode(" Approve replacement/removal (blank removes)"));details.append(label);
      const fields={};
      for(const [name,caption]of [["alternate","Alternate description"],["expansion","Expanded wording"]]){const label=document.createElement("label"),input=document.createElement("textarea");label.textContent=caption;input.value=(reviewed?reviewed[name]:original?.[name])??"";input.disabled=this.#mutating;input.setAttribute("aria-label",`${caption}: ${title}`);label.append(input);details.append(label);fields[name]=input;}
      const update=()=>{if(this.#mutating)return;apply(review.checked?{alternate:fields.alternate.value||null,expansion:fields.expansion.value||null}:null);this.#invalidate();};
      review.addEventListener("change",update);for(const input of Object.values(fields))input.addEventListener("input",()=>{review.checked=true;update();});
    });
  }
  async #render(){
    if(!this.#client?.state)return;const version=++this.#renderVersion,page=this.#page;
    this.#geometry=undefined;this.#q(".layer").replaceChildren();
    const result=await this.#client.render(page,96);if(version!==this.#renderVersion||page!==this.#page)return;
    if(this.#objectUrl)URL.revokeObjectURL(this.#objectUrl);this.#objectUrl=URL.createObjectURL(new Blob([result.png],{type:"image/png"}));
    this.#q("img").src=this.#objectUrl;this.#q(".paper").style.width=`${result.geometry.width}px`;this.#geometry=result.geometry;this.#overlays();
  }
  #overlays(){
    const layer=this.#q(".layer");layer.replaceChildren();if(!this.#geometry)return;
    const frames=this.#preview?.frames.map(f=>f.frame)??this.#request?.frames??[];
    // The image is the saved input page, while preview destinations use output
    // page numbering. Match stable frame IDs before placing geometry over it.
    const sourcePage=f=>this.#request?.frames.find(s=>s.id===f.id)?.page;
    for(const f of frames.filter(f=>sourcePage(f)===this.#page)){
      const button=document.createElement("button");button.className="frame";button.setAttribute("aria-label",`Edit frame ${f.id}`);button.title=`Approved frame ${f.id}`;
      this.#position(button,f.rect);button.addEventListener("click",()=>{const p=this.#preview?.frames.find(v=>v.frame.id===f.id)?.paragraph_ids[0]??this.#request?.paragraphs[0]?.id;
        [...this.shadowRoot.querySelectorAll("textarea[data-paragraph]")].find(t=>t.dataset.paragraph===p)?.focus();});layer.append(button);
    }
    for(const frame of this.#preview?.frames??[]){if(sourcePage(frame.frame)!==this.#page)continue;for(const figure of frame.figures??[]){const box=document.createElement("div");box.className="figure-preview";box.textContent=`Figure ${figure.figure_id} — geometry preview`;this.#position(box,figure.rect);layer.append(box);}}
    if(this.#rect){const box=document.createElement("div");box.className="selection";this.#position(box,this.#rect);layer.append(box);}
  }
  #position(element,rect){for(const [key,value] of Object.entries(pdfRectToCss(rect,this.#geometry)))element.style[key]=`${value}%`;}
  #lineBreakControls(group,p){
    const details=document.createElement("details"),summary=document.createElement("summary");summary.textContent="Line wrapping and Japanese punctuation";details.append(summary);
    const set=(key,value)=>{if(this.#busy)return;p.line_break={...(p.line_break??{}),[key]:value};this.#invalidate();};
    for(const[key,title,choices,fallback]of[
      ["profile","Line-break profile",[["unicode","Unicode"],["japanese_strict","Japanese strict"]],"unicode"],
      ["composition","Line composition",[["greedy","Fast local wrapping"],["balanced","Balance whole paragraph (bounded work)"]],"greedy"],
      ["emergency","Overlong words",[["break_word","Wrap between letters if needed"],["preserve_words","Keep words together; require enough space"]],"break_word"]
    ]){const label=document.createElement("label"),select=document.createElement("select");label.textContent=`${title} `;select.setAttribute("aria-label",`${title} for ${p.id}`);for(const[value,text]of choices)select.add(new Option(text,value));select.value=p.line_break?.[key]??fallback;select.addEventListener("change",()=>set(key,select.value));label.append(select);details.append(label);}
    for(const[key,title]of[["prohibit_start","Additional characters forbidden at line start"],["prohibit_end","Additional characters forbidden at line end"]]){
      const label=document.createElement("label"),input=document.createElement("input");label.textContent=`${title} `;input.type="text";input.value=p.line_break?.[key]??"";input.maxLength=4096;input.setAttribute("aria-label",`${title} for ${p.id}`);
      input.addEventListener("input",()=>{if(this.#busy)return;const value=input.value;const invalid=new TextEncoder().encode(value).length>4096||/[\u0000-\u0020\u007f-\u009f\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/u.test(value);input.setCustomValidity(invalid?"Use at most 4096 UTF-8 bytes, without whitespace or control characters.":"");set(key,value);});label.append(input);details.append(label);
    }
    const note=document.createElement("p");note.className="small";note.textContent="Nonbreaking controls and graphemes stay intact. Explicit newlines override punctuation rules. Balanced composition compares whole forced-break segments at the current frame width; large inputs can exceed its work limit. Preview again after changes; a protected sequence may require a wider frame.";details.append(note);group.append(details);
  }
  #newParagraph(text=""){return{id:crypto.randomUUID(),text,preferred_font:"Helvetica",font_size:12,line_height:14.4,rgb:[0,0,0],rtl:false,orphans:2,widows:2};}
  #ensureTagBindings(){const tags=this.#request?.source_tags;if(!tags)return;tags.paragraph_sources??={};for(const [i,p]of this.#request.paragraphs.entries())if(!Object.hasOwn(tags.paragraph_sources,p.id))this.#setRecord(tags.paragraph_sources,p.id,tags.selected.length===this.#request.paragraphs.length?structuredClone(tags.selected[i]):null);}
  #addParagraph(text=""){this.#ensureTagBindings();const p=this.#newParagraph(text);if(this.#request.source_tags)this.#setRecord(this.#request.source_tags.paragraph_sources,p.id,null);this.#request.paragraphs.push(p);}
  #newStory(){this.#historySource=undefined;this.#historyModel=undefined;this.#request={story_id:crypto.randomUUID(),input_sha256:this.#client.state.revision,writing_mode:this.#q("#writing-mode").value,frames:[],paragraphs:[],fonts:[],annotation_anchors:[],allow_font_substitution:this.#q("#fallback").checked,
    allow_page_creation:this.#q("#overflow").checked,prune_empty_pages:this.#q("#prune").checked,max_new_pages:64,mode:"flow_document",signature_policy_override:false};this.#invalidate(false);this.#paragraphs();this.#overlays();}
  async #previewLayout(){
    if(this.#historySource&&JSON.stringify(this.#request)!==this.#historyModel)throw new Error("This draft changed outside its causal history. Reconcile using the history panel or explicitly detach the collaboration draft.");
    if(!this.#request||this.#busy)return;const version=this.#version;this.#busy=true;this.#buttons();this.#status("Preparing native layout…");
    try{const result=this.#historySource?await this.#client.previewHistory(this.#historySource):await this.#client.preview(this.#request);if(version!==this.#version)return;this.#preview=result.preview;this.#receipt=result.receipt;
      if(result.prepared)this.#historySource=result.prepared.source;
      this.#q("#report").textContent=JSON.stringify({changed_pages:result.preview.changed_pages,new_pages:result.preview.generated_pages,page_breaks:result.preview.page_breaks,page_pruning:result.preview.page_pruning,font_choices:result.preview.font_choices,anchor_moves:result.preview.anchor_moves,figures:result.preview.frames.filter(f=>f.figures?.length).map(f=>({page:f.frame.page,figures:f.figures})),figure_removals:result.preview.figure_removals,source_tags:this.#request.source_tags,table_tags:this.#request.table_layout?.tagging,table_fragments:result.preview.frames.filter(f=>f.table_cells?.length).map(f=>({page:f.frame.page,cells:f.table_cells})),limits:result.preview.exact_limits},null,2);
      if(this.#historySource)this.#q("#report").textContent+="\n"+JSON.stringify({causal_history_sha256:result.receipt.history_sha256,checkpoint_before:result.prepared.checkpoint_before,persists_deleted_text:true},null,2);
      this.#overlays();this.#status("Layout ready. Review the report and approve before applying.");
    }finally{this.#busy=false;this.#buttons();if(version!==this.#version&&this.#request&&this.#autoPreview)this.#timer=setTimeout(()=>this.#run(()=>this.#previewLayout()),100);}
  }
  #annotationSelection(){
    this.#q("#annotation-approve").checked=false;
    this.#q("#annotation-rename").checked=false;
    const value=this.#q("#annotation-sources").value;
    const source=value===""?undefined:this.#annotationSources?.[Number(value)];
    this.#q("#annotation-group").textContent=source?JSON.stringify({source:source.annotation_id,name:source.name,page:source.page,rect:source.rect,group:source.group??null},null,2):"Load annotations for the current PDF revision.";
  }
  #annotationControls(group,paragraph){
    for(const anchor of this.#request.annotation_anchors??[]){
      if(anchor.paragraph_id!==paragraph.id)continue;
      const row=document.createElement("div"),label=document.createElement("p");label.textContent=`Anchor ${anchor.annotation_id}; ${anchor.group?.members.length??1} member(s).`;row.append(label);
      const renameLabel=document.createElement("label"),rename=document.createElement("input");rename.type="checkbox";rename.checked=!!anchor.rename_conflicting_names;renameLabel.append(rename,document.createTextNode(" Allow destination-name collision repair; scripts/FDF names are not rewritten"));rename.addEventListener("change",()=>{if(this.#busy)return;anchor.rename_conflicting_names=rename.checked;this.#invalidate();});row.append(renameLabel);
      for(const [axis,title]of [[0,"Horizontal"],[1,"Vertical"]]){
        const label=document.createElement("label"),input=document.createElement("input");label.textContent=`${title} anchor offset `;input.type="number";input.step="any";input.value=anchor.offset[axis];input.setAttribute("aria-label",`${title} offset for ${anchor.annotation_id}`);
        input.addEventListener("change",()=>{if(this.#busy)return;const value=Number(input.value);if(input.value!==""&&Number.isFinite(value)){anchor.offset[axis]=value;this.#invalidate();}});label.append(input);row.append(label);
      }
      const unlink=document.createElement("button");unlink.textContent="Unlink annotation group (keep objects)";unlink.addEventListener("click",()=>{if(this.#busy)return;this.#request.annotation_anchors=this.#request.annotation_anchors.filter(a=>a!==anchor);this.#invalidate();this.#paragraphs();});row.append(unlink);group.append(row);
    }
    if(!this.#client?.state||this.#annotationRevision!==this.#client.state.revision)return;
    const attach=document.createElement("button");attach.textContent="Attach approved annotation group to this paragraph";
    attach.addEventListener("click",()=>this.#run(async()=>{
      if(this.#busy)return;
      if(this.#annotationRevision!==this.#client.state.revision||!this.#q("#annotation-approve").checked)throw new Error("Load and explicitly approve the current annotation group first");
      const index=this.#q("#annotation-sources").value,source=index===""?undefined:this.#annotationSources?.[Number(index)];
      if(!source)throw new Error("Select an annotation source");
      const inputs=[this.#q("#annotation-x"),this.#q("#annotation-y")],offset=inputs.map(i=>Number(i.value));
      if(inputs.some(i=>i.value==="")||offset.some(v=>!Number.isFinite(v)))throw new Error("Both annotation offsets must be finite numbers");
      const members=new Set((source.group?.members??[source]).map(m=>m.annotation_id));
      if(this.#request.annotation_anchors?.some(a=>(a.group?.members??[a]).some(m=>members.has(m.annotation_id))))throw new Error("This group already overlaps an anchor in this story");
      this.#request.annotation_anchors??=[];this.#request.annotation_anchors.push({annotation_id:source.annotation_id,paragraph_id:paragraph.id,geometry_sha256:source.geometry_sha256,group:structuredClone(source.group??null),rename_conflicting_names:this.#q("#annotation-rename").checked,offset});
      this.#q("#annotation-approve").checked=false;this.#invalidate();this.#paragraphs();
    }));group.append(attach);
  }
  async #action(action){
    if(!this.#client)throw new Error("Attach a StoryWorkerClient first");
    if(action==="open"){this.#q("#pdf-file").value="";this.#q("#pdf-file").click();return;}
    if(action==="import"){this.#q("#story-file").value="";this.#q("#story-file").click();return;}
    if(action==="cancel"){this.#invalidate(false);this.#renderVersion++;clearTimeout(this.#timer);await this.#client.cancel();this.#busy=false;this.#mutating=false;this.#status("Work cancelled. Last published PDF and undo history retained.");return;}
    if(this.#busy)return;
    if(action==="history"){
      if(!this.#request)throw new Error("Load or create a story draft first");
      await this.#q("wellfriend-story-history-editor").begin(this.#request);return;
    }
    if(action==="structure"){
      if(!this.#request)throw new Error("Load or create a story draft first");if(this.#historySource)throw new Error("Explicitly detach the collaboration draft before structural branch resolution");
      await this.#q("wellfriend-story-structure-editor").begin(this.#request);return;
    }
    if(action==="resume-history"){
      const saved=this.#client.state.stories.find(story=>story.request.story_id===this.#request?.story_id);
      if(!saved||JSON.stringify(saved.request)!==JSON.stringify(this.#request))throw new Error("Load the current saved story first; unsaved changes must not be silently replaced by history resume.");
      await this.#q("wellfriend-story-history-editor").resume(saved.request.story_id,this.#request);return;
    }
    if(action==="detach-history"){
      if(this.#historySource&&globalThis.confirm("Detach this draft from collaborative history? A normal save may leave the saved history detached. Local history operations remain in the history panel.")){this.#historySource=undefined;this.#historyModel=undefined;this.#invalidate(false);}return;
    }
    if(action==="clear-draft"){
      if(this.#request&&!globalThis.confirm("Discard this unsaved story draft? The saved PDF and saved stories will not be changed."))return;
      this.#historySource=undefined;this.#historyModel=undefined;this.#request=undefined;this.#invalidate(false);this.#q("#stories").value="";this.#paragraphs();this.#overlays();this.#status("Draft cleared. Saved PDF unchanged; native occurrence editing is available.");return;
    }
    if(action==="new"){this.#newStory();this.#q("#source-panel").open=true;return;}
    if(action==="preview")return this.#previewLayout();
    if(action==="annotations"){
      const revision=this.#client.state.revision;this.#busy=true;this.#buttons();
      try{const sources=await this.#client.annotations();if(revision!==this.#client.state.revision)return;this.#annotationSources=sources;this.#annotationRevision=revision;const select=this.#q("#annotation-sources");select.replaceChildren();
        for(const [index,source]of sources.entries())select.add(new Option(`Page ${source.page} /${source.subtype}: ${source.name??"unnamed"} [${source.annotation_id}] (${source.group?.members.length??1} members)`,String(index)));
        this.#annotationSelection();this.#paragraphs();this.#status("Review the full group, approve it, then attach it to a paragraph. Preview validates geometry, ownership and every member before save.");
      }finally{this.#busy=false;}return;
    }
    if(action==="images"){
      const revision=this.#client.state.revision,page=this.#page;this.#busy=true;this.#buttons();
      try{const [images,ocr,forms]=await Promise.all([this.#client.images(page),this.#client.imageOcrSources(page),this.#client.formTextSources(page)]);if(revision!==this.#client.state.revision||page!==this.#page)return;this.#imageSources=images;this.#formTextSources={...forms,page_spans:ocr.source_spans.filter(span=>span.text_render_mode===3)};this.#imageRevision=revision;const select=this.#q("#image-sources");select.replaceChildren();
        for(const [index,image]of images.entries())select.add(new Option(`${image.resource_name??"Inline image"} · stream ${image.content_stream_index+1}${image.invocation_path?.length?" (nested Form)":""}`,String(index)));
        select.value=images.length?"0":"";this.#imageSelection();this.#paragraphs();this.#status(images.length?"Choose a page or nested-Form image and only its exact invisible OCR operands, then attach it to a caption. Review geometry, source ownership and native layout before saving.":"No image occurrence is available on this page.");
      }finally{this.#busy=false;}return;
    }
    if(action==="table-values"){
      if(!this.#request?.table_layout)throw new Error("Import or select a table story first");
      this.#busy=true;this.#buttons();const version=this.#version;
      try{const request=await this.#client.synchronizeTableValues(this.#request);if(version!==this.#version)throw new Error("Draft changed during table recalculation; retry on the current draft");this.#request=request;this.#invalidate(false);this.#paragraphs();this.#status("Typed values recalculated. Review row fragments and source grid decisions in a new preview.");}finally{this.#busy=false;}return;
    }
    if(action==="tags"){
      const revision=this.#client.state.revision;this.#busy=true;this.#buttons();
      try{const tags=await this.#client.tags();if(revision!==this.#client.state.revision)return;this.#tagSources=tags;this.#tagRevision=revision;const select=this.#q("#tag-owners");select.replaceChildren();
        for(const [i,tag]of tags.entries()){const option=new Option(`/${tag.role} — object ${tag.reference.object}; pages ${[...new Set(tag.page_mcids.map(v=>v[0]))].join(", ")||"empty"}`,String(i));option.disabled=!tag.text_leaf||!tag.parent;select.add(option);}
        this.#paragraphs();this.#status(tags.length?"Choose complete sibling owners, then bind them to the draft paragraphs. Review any alternate or expanded wording explicitly.":"No existing structure tree was found.");
      }finally{this.#busy=false;}return;
    }
    if(action==="bind-tags"){
      if(this.#request?.figure_removals?.length)throw new Error("Cancel pending Figure deletions before rebinding the selected structural interval");
      if(this.#request?.table_layout)throw new Error("Tables require complete Table/TR/TH/TD ownership in table_layout.tagging, not paragraph sibling bindings");
      if(!this.#request||this.#tagRevision!==this.#client.state.revision)throw new Error("Create a story and load structure owners for the current PDF first");
      const selected=[...this.#q("#tag-owners").selectedOptions].map(option=>this.#tagSources[Number(option.value)]).filter(tag=>tag.text_leaf&&tag.parent);
      if(!selected.length)throw new Error("Select one or more existing paragraph/Figure owners");
      const parent=JSON.stringify(selected[0].parent);if(selected.some(tag=>JSON.stringify(tag.parent)!==parent))throw new Error("Owners must share the same structural parent");
      selected.sort((a,b)=>a.child_position-b.child_position);
      if(selected.some((tag,i)=>tag.child_position===null||i>0&&tag.child_position!==selected[i-1].child_position+1))throw new Error("Select a contiguous sibling interval");
      const paragraphs=selected.filter(tag=>tag.role!=="Figure");
      this.#request.source_tags={parent:structuredClone(selected[0].parent),selected:selected.map(tag=>structuredClone(tag.reference)),insert_at:selected[0].child_position,paragraph_sources:Object.fromEntries(this.#request.paragraphs.map((p,i)=>[p.id,paragraphs[i]?structuredClone(paragraphs[i].reference):null])),new_roles:{},figures:{}};
      this.#invalidate(false);this.#paragraphs();this.#status("Source tag owners bound. Assign each Figure to its own selected owner (or approve a new Figure for an untagged image), then review caption reuse/create choices and descriptions.");return;
    }
    if(action==="source"){const page=this.#page,revision=this.#client.state.revision;const model=await this.#client.source(page);if(page===this.#page&&revision===this.#client.state.revision){this.#model=model;this.#q("#source").value=model.logical_text;}return;}
    if(action==="draw"){if(!this.#geometry)throw new Error("Wait for this page to render first");this.#drawing=true;this.#q(".layer").classList.add("draw");this.#status("Drag the approved text frame on the PDF page.");return;}
    if(action==="frame"){
      const inputs=[...this.shadowRoot.querySelectorAll("#rect-inputs input")];if(inputs.every(i=>i.value!==""&&Number.isFinite(Number(i.value))))this.#rect=inputs.map(i=>Number(i.value));
      if(!this.#model||this.#model.page!==this.#page||!this.#rect)throw new Error("Load source, select text and draw a nonempty frame first");
      const input=this.#q("#source"),[start,end]=sourceSelectionRange(this.#model.logical_text,input.selectionStart,input.selectionEnd);
      if(start===end||this.#rect[0]>=this.#rect[2]||this.#rect[1]>=this.#rect[3])throw new Error("Select a nonempty source range and frame");
      if(!this.#request)this.#newStory();if(this.#request.frames.some(f=>f.page>this.#page))throw new Error("Link frames in approved page order");
      const text=Array.from(this.#model.logical_text).slice(start,end).join("");
      this.#request.frames.push({id:crypto.randomUUID(),page:this.#page,logical_range:[start,end],expected_text:text,rect:[...this.#rect],exclusions:[]});this.#addParagraph(text);
      this.#rect=undefined;inputs.forEach(i=>{i.value="";});this.#invalidate();this.#paragraphs();this.#overlays();return;
    }
    if(action==="paragraph"){if(!this.#request)this.#newStory();this.#addParagraph();this.#invalidate();this.#paragraphs();return;}
    if(action==="download"){
      const url=URL.createObjectURL(new Blob([this.#client.bytes()],{type:"application/pdf"})),link=document.createElement("a");link.href=url;link.download="edited.pdf";link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);return;
    }
    if(["save","undo","redo"].includes(action)){
      if(action==="save"&&(!this.#receipt||!this.#q("#approve").checked))throw new Error("Approve the current preview first");
      this.#busy=true;this.#mutating=true;this.#buttons();const id=this.#request?.story_id,version=this.#version;
      try{if(action==="save"){if(this.#historySource)await this.#client.checkpointHistory(this.#historySource,this.#receipt);else await this.#client.checkpoint(this.#request,this.#receipt);}else await this.#client[action]();
        this.#model=undefined;this.#rect=undefined;this.#q("#source").value="";
        const saved=this.#client.state.stories.find(s=>s.request.story_id===id);
        // Edits typed while checkpointing cannot be silently thrown away.
        if(version!==this.#version)this.dispatchEvent(new CustomEvent("draftconflict",{detail:this.request}));
        this.#historySource=undefined;this.#historyModel=undefined;this.#request=saved?.request;this.#invalidate(false);this.#refresh();await this.#render();this.#status("Native PDF checkpoint loaded. Download exports these exact bytes. Resume saved text history to continue its collaboration identities.");
      }finally{this.#busy=false;this.#mutating=false;}
    }
  }
  disconnectedCallback(){clearTimeout(this.#timer);this.#renderVersion++;if(this.#objectUrl)URL.revokeObjectURL(this.#objectUrl);}
}
if(!customElements.get("wellfriend-story-editor"))customElements.define("wellfriend-story-editor",WellfriendStoryEditor);
