# Form JavaScript Policy

Wellfriend does not execute arbitrary PDF JavaScript. `form_js_report` inventories
scripts/actions and the default runtime posture is disabled.

Policy modes:

| Mode | Result |
|---|---|
| `inventory_only` | report only; input bytes unchanged |
| `disable_execution_preserve_source` | source preserved; Wellfriend execution remains disabled |
| `remove_javascript_only` | JavaScript actions and the document JavaScript name-tree slot are removed |
| `remove_all_active_actions` | all action dictionaries/owner slots are removed |
| `preserve_safe_navigation_only` | only internal `GoTo` and bounded page `Named` actions survive |
| `flatten_calculated_values_then_remove` | bounded calculations are written, then actions are removed |
| `custom` | explicit action-type allow/remove sets; anything not allowed fails closed |

Inventory covers catalog `/OpenAction` and `/AA`, document JavaScript name
trees, page `/AA`, annotation/widget/field `/A` and `/AA`, calculate/validate/
format/keystroke and focus/mouse events, submit/import/reset, Launch, URI,
GoTo/GoToR/GoToE, Named, Rendition, `/Next` graphs, malformed actions, and XFA
script/event reports. JavaScript stream decoding is capped at 8 MiB per script
and 64 MiB per document.

Valid example: `event.value = this.getField("A").value * 2;` may be evaluated
only in the opt-in flatten policy. Failure example: `app.launchURL(...)` is
inventoried and rejected by the safe subset.

All full-rewrite policies expose signature impact and obey secure mutation closeout
DocMDP/FieldMDP enforcement. They do not claim cryptographic validity.
