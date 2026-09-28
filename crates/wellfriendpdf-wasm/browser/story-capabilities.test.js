// Unexecuted source regressions; these do not load WASM or establish browser QA.
import test from "node:test";
import assert from "node:assert/strict";
import { assertStorySessionCapabilities, assertFontInstanceCapabilities } from "./story-capabilities.js";
const storyStatus={line_break_policy_version:2,line_shaping_context_version:3,story_pagination_policy_version:1,history_paragraph_style_protocol_version:1,history_paragraph_structure_protocol_version:1,history_inline_style_protocol_version:1,history_compaction_protocol_version:1};

test("matching native capability is checked through read-only status", () => {
  const calls = [];
  assertStorySessionCapabilities({ commandJson(value) {
    calls.push(JSON.parse(value));
    return JSON.stringify(storyStatus);
  } });
  assert.deepEqual(calls, [{ op: "status" }]);
});

test("missing, older, future and mistyped native capabilities are rejected", () => {
  for (const [key, expected] of [["line_break_policy_version",2],["line_shaping_context_version",3],["story_pagination_policy_version",1],["history_paragraph_style_protocol_version",1],["history_paragraph_structure_protocol_version",1],["history_inline_style_protocol_version",1],["history_compaction_protocol_version",1]]) for (const version of [undefined, null, 0, 4, String(expected), true, expected === 2 ? 3 : 2]) {
    assert.throws(() => assertStorySessionCapabilities({ commandJson() {
      return JSON.stringify({ ...storyStatus, [key]: version });
    } }), /matching SDK/);
  }
  assert.throws(() => assertStorySessionCapabilities({}), /matching SDK/);
  assert.throws(() => assertStorySessionCapabilities(null), /matching SDK/);
});

test("malformed or failed native status cannot authorize a session", () => {
  for (const value of ["null", "[]", "false", "{}", "not json"]) {
    assert.throws(() => assertStorySessionCapabilities({ commandJson() { return value; } }));
  }
  const failure = new Error("native failure");
  assert.throws(() => assertStorySessionCapabilities({ commandJson() { throw failure; } }),
    (error) => error === failure);
});

test("static font instancing has an independent native version gate", () => {
  const calls = [];
  assertFontInstanceCapabilities({ commandJson(value) {
    calls.push(JSON.parse(value)); return JSON.stringify({ font_instance_protocol_version: 4 });
  } });
  assert.deepEqual(calls, [{ op: "status" }]);
  for (const version of [undefined, null, 0, 1, 2, 3, 5, "4", true]) {
    assert.throws(() => assertFontInstanceCapabilities({ commandJson() {
      return JSON.stringify({ line_break_policy_version: 2, line_shaping_context_version: 3, story_pagination_policy_version: 1, font_instance_protocol_version: version });
    } }), /matching SDK/);
  }
  for (const value of ["null", "{}", "[]", "not json"]) {
    assert.throws(() => assertFontInstanceCapabilities({ commandJson() { return value; } }));
  }
  assert.throws(() => assertFontInstanceCapabilities(null), /matching SDK/);
});
