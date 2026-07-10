# XFA event model

All events are inventoried and disabled by default. The opt-in minimal runtime executes only FormCalc `calculate` then `validate`, ordered after initialize rank and before ready/layout ranks. Execution and field mutations are capped and audited.

Initialize, ready, docReady, formReady, layoutReady, pre/post open, enter/exit, change, click, full, pre/post print, pre/post save, submit, and signature events are reported exact but not executed. User-interaction and host lifecycle simulation are not part of the runtime.
