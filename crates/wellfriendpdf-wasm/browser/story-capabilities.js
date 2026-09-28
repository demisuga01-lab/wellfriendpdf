// A new UI must not send policy fields to an older SDK that silently ignores
// unknown paragraph fields. Saved metadata has its own version gate as well.
export function assertStorySessionCapabilities(session) {
  const mismatch = "This editor requires a matching SDK with paragraph composition, hard-line shaping, physical-page pagination and causal paragraph style/structure history. Update the worker and WASM assets together.";
  if (typeof session?.commandJson !== "function") throw new Error(mismatch);
  const status = JSON.parse(session.commandJson(JSON.stringify({ op: "status" })));
  if (status?.line_break_policy_version !== 2 || status?.line_shaping_context_version !== 3 || status?.story_pagination_policy_version !== 1 || status?.history_paragraph_style_protocol_version !== 1 || status?.history_paragraph_structure_protocol_version !== 1 || status?.history_inline_style_protocol_version !== 1 || status?.history_compaction_protocol_version !== 1) throw new Error(mismatch);
}

// Optional commands get their own gate: old binaries may still perform ordinary
// font extraction, but must never ignore an explicit static-axis request.
export function assertFontInstanceCapabilities(session) {
  const mismatch = "Static font instancing requires a matching SDK. Update the worker and WASM assets together.";
  if (typeof session?.commandJson !== "function") throw new Error(mismatch);
  const status = JSON.parse(session.commandJson(JSON.stringify({ op: "status" })));
  if (status?.font_instance_protocol_version !== 4) throw new Error(mismatch);
}
