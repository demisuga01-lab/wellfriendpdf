# Form JavaScript Sanitizer

The form action policy sanitizer uses the canonical object rewriter. It removes both
action objects and the owning `/A`, `/AA`, `/OpenAction`, `/Next`, or
`/Names/JavaScript` reachability slot required by the selected policy. Sibling
name trees such as `/Dests` and `/EmbeddedFiles` are preserved.

Every saved PDF is reopened and rescanned. `rescan_passed` is true only when no
action forbidden by that policy remains reachable. Running the same sanitizer
twice is covered by the metamorphic gate.

Malformed, cyclic, unresolved, encrypted/undecodable, oversized, and excessive-
depth actions are removed fail-closed by active-action policies. Inventory-only
modes keep source bytes unchanged and therefore do not claim content removal.

Safe-navigation mode preserves internal `GoTo` destinations plus `Named`
`FirstPage`, `LastPage`, `NextPage`, and `PrevPage`. External URLs, Launch,
GoToR, GoToE, submit/import, media, and JavaScript are removed.

Example:

```text
wellfriendpdf form-js-sanitize input.pdf --policy preserve_safe_navigation_only \
  --output sanitized.pdf --report sanitizer.json
```

On certified/signed inputs a prohibited full rewrite returns the stable
`unsupported_feature` error unless the caller explicitly requests the Roadmap task
18B signature-policy override.
