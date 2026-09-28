/** Local font discovery and explicit TTC/OTC face selection. Font bytes are
 * passed to the native worker; no font is installed or fetched from a network. */
export class WellfriendFontPicker extends HTMLElement {
  #client; #bytes; #catalog; #epoch = 0; #busy = false; #disabled = false;
  constructor() {
    super();
    this.attachShadow({mode:"open"}).innerHTML = `<style>
      :host{display:block}label{display:block;margin:8px 0}input,select,button{font:inherit;max-width:100%}
      pre{white-space:pre-wrap;overflow-wrap:anywhere}button:focus-visible{outline:3px solid #3668ce}
    </style><details><summary>Import a local font or collection face</summary>
      <p>Up to 4 MiB. Choose the exact face, add it to the draft, then select its name on a paragraph. Preview and approval are still required.</p>
      <label>Font file <input id="file" type="file" accept=".ttf,.otf,.ttc,.otc"></label>
      <label>Face <select id="face"></select></label>
      <label>Unique draft lookup name <input id="name" maxlength="128"></label>
      <label><input id="signature" type="checkbox"> Allow removal of a font signature when extraction/instancing requires it (not verified)</label>
      <label id="instance-choice" hidden><input id="instance" type="checkbox"> Prepare a static editable font from selected TrueType/CFF2 coordinates</label>
      <fieldset id="instance-fields" hidden><legend>Static instance identity and coordinates</legend>
        <p>These explicit names replace localized identity names. Licence/vendor strings remain. No font installation occurs. Unhandled tables or hint semantics can prevent preparation.</p>
        <div id="axes"></div>
        <label>Typographic family <input id="family" maxlength="256"></label>
        <label>Typographic subfamily <input id="subfamily" maxlength="256"></label>
        <label>Legacy family <input id="legacy-family" maxlength="256"></label>
        <label>PostScript name (ASCII, no spaces) <input id="postscript" maxlength="63"></label>
        <label>Legacy style link <select id="style-link"><option value="regular">Regular</option><option value="bold">Bold</option><option value="italic">Italic</option><option value="bold_italic">Bold Italic</option></select></label>
        <label><input id="metric-differences" type="checkbox"> Accept reported redundant-metric differences (declared metrics take precedence)</label>
        <fieldset id="contours" hidden><legend>CFF2 contour compatibility</legend>
          <label><input id="normalize-contours" type="checkbox"> Normalize overlapping contours using curve-preserving numerical union</label>
          <label>Solver tolerance (font design units) <input id="contour-tolerance" type="number" min="0.0000152587890625" max="0.125" step="any" value="0.001"></label>
          <label><input id="hint-loss" type="checkbox"> Allow hint removal only where contour rewriting is needed (small-size appearance can change). Compatible glyphs keep their hints.</label>
          <p>Without normalization, preparation preserves contour/hint bytes only after a compatibility check; outlines needing reconstruction require enabling normalization. Curved checks and union are numerical, not pixel-fidelity guarantees. Review the report and PDF preview.</p>
        </fieldset>
      </fieldset>
      <pre id="info" aria-label="Selected font properties"></pre>
      <button id="add">Prepare and add this font</button>
      <p id="status" role="status" aria-live="polite">Open an editable story draft to import fonts.</p>
    </details>`;
    this.#q("#file").addEventListener("change", () => this.#load());
    this.#q("#face").addEventListener("change", () => this.#select());
    this.#q("#signature").addEventListener("change", () => this.#buttons());
    this.#q("#instance").addEventListener("change", () => { this.#q("#instance-fields").hidden = !this.#q("#instance").checked; this.#buttons(); });
    this.#q("#add").addEventListener("click", () => this.#prepare());
    this.#buttons();
  }
  #q(selector) { return this.shadowRoot.querySelector(selector); }
  #status(value) { this.#q("#status").textContent = value; }
  set client(value) {
    this.#epoch++; this.#client = value; this.#bytes = undefined; this.#catalog = undefined; this.#busy = false;
    this.#q("#file").value = ""; this.#q("#face").replaceChildren(); this.#q("#info").textContent = "";
    this.#select();
  }
  get client() { return this.#client; }
  set disabled(value) { this.#disabled = !!value; this.#buttons(); }
  get disabled() { return this.#disabled; }
  disconnectedCallback() { this.#epoch++; this.#busy = false; }
  connectedCallback() { this.#buttons(); }
  #face() { return this.#catalog?.faces.find(face => String(face.face_index) === this.#q("#face").value); }
  #buttons() {
    const unavailable = this.#disabled || this.#busy || !this.#client;
    for (const control of this.shadowRoot.querySelectorAll("input,select,button")) control.disabled = unavailable;
    const face = this.#face();
    this.#q("#add").disabled ||= !face || !face.permission_bits_allow_editing || !["true_type","cff1","cff2"].includes(face.outline_format)
      || (face.outline_format === "cff2" && !this.#q("#instance").checked)
      || ((this.#catalog.collection || this.#q("#instance").checked) && face.signature_present && !this.#q("#signature").checked);
  }
  #select() {
    this.#q("#signature").checked = false;
    this.#q("#instance").checked = false; this.#q("#metric-differences").checked = false;
    this.#q("#normalize-contours").checked = false; this.#q("#hint-loss").checked = false; this.#q("#contour-tolerance").value = "0.001";
    this.#q("#instance-fields").hidden = true; this.#q("#axes").replaceChildren();
    const face = this.#face();
    this.#q("#contours").hidden = face?.outline_format !== "cff2";
    this.#q("#info").textContent = face ? JSON.stringify(face, null, 2) : "Choose one face explicitly.";
    this.#q("#name").value = face?.postscript_name ?? [face?.family,face?.subfamily].filter(Boolean).join(" ");
    this.#q("#instance-choice").hidden = !(face?.outline_format === "cff2" || (face?.outline_format === "true_type" && face.axes.length));
    this.#q("#family").value = face?.family ?? "Selected Font";
    this.#q("#subfamily").value = "Instance";
    this.#q("#legacy-family").value = `${face?.family ?? "Selected Font"} Instance`;
    this.#q("#postscript").value = `${(face?.postscript_name ?? "SelectedFont").replace(/[^A-Za-z0-9_-]/g, "").slice(0, 40)}-Instance`;
    this.#q("#style-link").value = "regular";
    for (const axis of face?.axes ?? []) {
      const label = document.createElement("label"), input = document.createElement("input");
      label.textContent = `${axis.tag} (${axis.min} to ${axis.max}) `;
      input.type = "number"; input.step = "any"; input.min = String(axis.min); input.max = String(axis.max); input.value = String(axis.default); input.dataset.axis = axis.tag;
      label.append(input); this.#q("#axes").append(label);
    }
    this.#buttons();
  }
  async #load() {
    if (this.#busy || this.#disabled || !this.#client) return;
    const file = this.#q("#file").files?.[0], epoch = ++this.#epoch;
    this.#bytes = undefined; this.#catalog = undefined; this.#q("#face").replaceChildren(); this.#select();
    if (!file) return;
    if (!file.size || file.size > 4 * 1024 * 1024) { this.#status("Font file must be 1..=4 MiB."); return; }
    this.#busy = true; this.#buttons();
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (epoch !== this.#epoch) return;
      const catalog = await this.#client.inspectFont(bytes);
      if (epoch !== this.#epoch) return;
      this.#bytes = bytes; this.#catalog = catalog;
      this.#q("#face").add(new Option("Choose a face", ""));
      for (const face of catalog.faces) this.#q("#face").add(new Option(
        `${face.face_index}: ${face.family ?? "Unnamed"} ${face.subfamily ?? ""} (${face.outline_format})`, String(face.face_index)));
      this.#select(); this.#status("Select the exact face. Variable TrueType and CFF2 faces offer explicit static preparation. CFF2 requires that option and emits CFF1; ordinary extraction does not pin axes.");
    } catch (error) { if (epoch === this.#epoch) this.#status(error.message ?? String(error)); }
    finally { if (epoch === this.#epoch) { this.#busy = false; this.#buttons(); } }
  }
  async #prepare() {
    const face = this.#face();
    if (this.#busy || this.#disabled || !face || !this.#bytes) return;
    const name = this.#q("#name").value.trim();
    if (!name) { this.#status("Enter a unique draft lookup name."); return; }
    const epoch = ++this.#epoch, selection = {source_sha256:this.#catalog.source_sha256, face_index:face.face_index,
      allow_signature_removal:this.#q("#signature").checked};
    let instance;
    if (this.#q("#instance").checked) {
      const coordinates = {};
      for (const input of this.#q("#axes").querySelectorAll("input")) {
        if (!input.value.trim() || !input.checkValidity() || !Number.isFinite(input.valueAsNumber)) { this.#status("Every selected coordinate must be finite and inside its axis range."); return; }
        coordinates[input.dataset.axis] = input.valueAsNumber;
      }
      instance = {selection, coordinates, naming:{family:this.#q("#family").value,subfamily:this.#q("#subfamily").value,
        legacy_family:this.#q("#legacy-family").value,postscript_name:this.#q("#postscript").value,style_link:this.#q("#style-link").value},
        accept_redundant_metric_differences:this.#q("#metric-differences").checked};
      if (face.outline_format === "cff2" && this.#q("#normalize-contours").checked) {
        const tolerance = this.#q("#contour-tolerance");
        if (!tolerance.value.trim() || !tolerance.checkValidity() || !Number.isFinite(tolerance.valueAsNumber)) { this.#status("Contour tolerance must be finite and inside the displayed bounds."); return; }
        instance.cff2_contours = {tolerance_font_units:tolerance.valueAsNumber,allow_hint_loss:this.#q("#hint-loss").checked};
      }
    }
    this.#busy = true; this.#buttons();
    try {
      const prepared = instance ? await this.#client.prepareFontInstance(name, this.#bytes, instance) : await this.#client.prepareFont(name, this.#bytes, selection);
      if (epoch !== this.#epoch) return;
      if (this.#disabled) { this.#status("Draft changed state; prepare again when it is editable."); return; }
      const accepted = this.dispatchEvent(new CustomEvent("fontprepared", {detail:structuredClone(prepared), bubbles:true, composed:true, cancelable:true}));
      this.#status(accepted ? "Preparation emitted fontprepared. In the story editor, select its lookup name on a paragraph and preview." : "Font was not added. Use an editable draft and a unique lookup name.");
      this.#q("#info").textContent = JSON.stringify(prepared.report, null, 2);
    } catch (error) { if (epoch === this.#epoch) this.#status(error.message ?? String(error)); }
    finally { if (epoch === this.#epoch) { this.#busy = false; this.#buttons(); } }
  }
}
if (!customElements.get("wellfriend-font-picker")) customElements.define("wellfriend-font-picker", WellfriendFontPicker);
