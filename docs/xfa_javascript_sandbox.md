# XFA JavaScript policy

XFA Runtime does not execute JavaScript. Scripts are inventoried with language, event, target SOM, size, SHA-256, provenance, and blocked capabilities, then receive `unsupported_reported_security_policy`.

There is no browser, WebView, Node, system script host, or embedded JavaScript interpreter. Consequently eval/function constructors, prototypes, global escape, timers, network, files, process, native calls, imports, DOM APIs, Acrobat privileged APIs, allocations, and loops cannot run. This fail-closed inventory posture is intentional and is not a JavaScript-subset claim.
