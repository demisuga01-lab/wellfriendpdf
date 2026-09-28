# Coordinated text-field and widget source editing

Source implementation only, on the uncommitted candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. This is not runtime qualification or
completion of the full editor roadmap.

## Ownership and transaction

The implementation follows the terminal-field/widget relationship and default
appearance resource rules in sections 12.7.3 and 12.7.4.3 of the
[PDF specification](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).
It reuses the canonical reachable field index rather than matching field names.
A revision-bound field target names an indirect terminal text field and a review
page containing one of its widgets. Every widget must have unique field/page
ownership. Merged field/widget dictionaries are composed, not overwritten.

The public native operation is
`advanced_editing::form_text::appearance::widgets::edit_widget_text`. Universal
plans use `operation.kind = scoped_text` and `source.scope = widget_field`.
`WidgetTextEditRequest` carries expected/replacement field values, one source
appearance request and complete expected/replacement display per widget, and
explicit default-value/default-appearance/rich-text/action decisions.

All source targets, field value, complete widget displays and structural
ActualText decisions are bound before mutation. Field and display text may
differ, but their mappings must be supplied explicitly. Duplicate/missing
widgets and competing logical-text decisions fail. Shared ActualText decisions
are rebased against private staged revisions only after checking the same
approved replacement; stale original byte identities are not reused.

The writer stages native source edits, sets the terminal field's Unicode V,
optionally sets DV, and verifies reopened value, ownership, complete displays,
default dictionaries and supported tag ownership. Inherited values are shadowed
on the selected terminal rather than changing sibling fields through ancestors.
Rich text can be discarded only by explicit plain-text conversion. No action or
calculation executes; retained actions require a separate affirmative decision.

Private intermediate edits are squashed into one canonical incremental update
against the original reader. The final Prev points directly to the input xref.
No partially updated field/widget revision is published. Original historical
bytes remain: this is ordinary editing, not sanitizing redaction. Native nested
edit metrics describe staging; output targets are rebound to the published PDF.
Physical `widget_pages` are reported separately from `affected_pages`: tagged
owner changes conservatively invalidate the document because shared logical
ancestors can extend beyond the widgets' pages.

## Future viewer defaults

- `preserve_source_defaults` leaves source DA/DR/Q and widget overrides alone.
  A generated replacement font does not silently replace viewer editing defaults.
- `from_edited_appearance` binds an explicitly selected edited font into AcroForm
  DR/Font with a collision-safe name, sets the field DA, and removes per-widget
  DA overrides. Size zero explicitly requests viewer auto-sizing. This composes
  with the current staged catalog, including tagged updates. It does not promise
  that a subset contains all future characters a user may type.
- Reset value DV changes only when `update_default_value` is true.

## Source integration and limits

Universal optional source discovery includes `widget_fields`; the shared JSON
plan/preview/approve/apply routes expose the operation to existing HTTP/native
bindings. The browser native panel has an explicit field-wide plain-text choice:
it changes all widgets, uses their respective source rectangles/styles, and
previews every widget page (up to eight in this panel). Formatted, nested,
tagged, rich or scripted mappings need typed advanced requests. No second
document store or automatic approval is introduced.

The implementation bounds transactions to 256 widgets, discovery to 256 fields,
individual text to 4 MiB and aggregate text/cache data to 16 MiB. It checks
MaxLen, single-line constraints and read-only consent. Password, file-selection,
comb, XFA, missing/stateful/alternate appearances and document-wide
NeedAppearances regeneration remain separate work. Source discovery is not a
promise that every discovered field is editable. Revision-local field/page
indexes and per-page appearance-target/display caches avoid rebuilding the full
source graph for each field/widget. Input caches are dropped before mutation.
Private serialization and the complete workflow still need scaling qualification.

## Evidence

Eleven Rust regression functions and one fake-worker JavaScript regression were
added, **not executed**. They cover cross-page/shared appearances, repeated edits,
single-revision publication, exact field/widget coverage, merged dictionaries,
inherited values, defaults/reset values, read-only/rich/action decisions,
MaxLen/NeedAppearances, atomic failure, shared structural ActualText, and the
governed plan/preview/apply/undo route. The Rust preview fixture contains rendering
assertions, but no rendering was run. The JavaScript fixture is not a WASM test.

No compiler, build, typecheck, tests, PDF workloads, rendering, browser QA,
benchmarks, binding execution, deployment, commit or push were run. Source
formatting/whitespace checks do not establish correctness or visual fidelity.
Universal editing and superiority to Acrobat remain unestablished.
