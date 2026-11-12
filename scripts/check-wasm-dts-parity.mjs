#!/usr/bin/env node
// Keeps the hand-written WASM declarations (crates/wellfriendpdf-wasm/
// wellfriendpdf.d.ts, the package's published "types") in step with the
// declarations wasm-bindgen generates from the Rust exports.
//
//   node scripts/check-wasm-dts-parity.mjs <generated.d.ts> [--write] [--hand <file>]
//
// <generated.d.ts> is wasm-bindgen's output, e.g. pkg/wellfriendpdf_wasm.d.ts
// after `wasm-pack build crates/wellfriendpdf-wasm --target web --out-dir pkg`.
// Every exported class, constructor, method, static method and property and
// every exported function in it must be declared in the hand-written file,
// with the same number of parameters. Without --write the script lists what
// is missing or differs and exits 1. With --write it copies each missing
// declaration, with its doc comment, from the generated file into the
// hand-written one (new members at the end of their class, new classes and
// functions at the end of the file) and reports parameter-count differences
// for a person to resolve; existing hand-written declarations, which may use
// narrower types such as ReportJson, are never changed.
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wasmDir = join(root, "crates", "wellfriendpdf-wasm");
const args = process.argv.slice(2);
const write = args.includes("--write");
const handIndex = args.indexOf("--hand");
const handPath = resolve(handIndex >= 0 ? args[handIndex + 1] : join(wasmDir, "wellfriendpdf.d.ts"));
const generatedPath = args.find((arg, index) => !arg.startsWith("--") && args[index - 1] !== "--hand");
if (!generatedPath) {
  console.error("usage: node scripts/check-wasm-dts-parity.mjs <generated.d.ts> [--write] [--hand <file>]");
  process.exit(2);
}

// TypeScript is the WASM package's dev dependency (`npm ci` in crates/wellfriendpdf-wasm).
const ts = createRequire(join(wasmDir, "package.json"))("typescript");

// wasm-bindgen's loader exports, which the hand-written file declares in its
// own shape (`export default function init(...)`).
const LOADER_EXPORTS = new Set(["initSync", "__wbg_init", "default"]);

function parse(path) {
  const text = readFileSync(path, "utf8");
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const classes = new Map();
  const functions = new Map();
  for (const statement of source.statements) {
    const exported = ts.getModifiers?.(statement)?.some((m) => m.kind === ts.SyntaxKind.ExportKeyword);
    if (!exported) continue;
    if (ts.isClassDeclaration(statement) && statement.name) {
      const members = new Map();
      for (const member of statement.members) {
        const key = memberKey(member);
        if (key) members.set(key, { node: member, params: paramCount(member) });
      }
      classes.set(statement.name.text, { node: statement, members });
    } else if (ts.isFunctionDeclaration(statement) && statement.name) {
      const name = statement.name.text;
      if (!LOADER_EXPORTS.has(name)) functions.set(name, { node: statement, params: paramCount(statement) });
    }
  }
  return { text, source, classes, functions };
}

function memberKey(member) {
  if (ts.isConstructorDeclaration(member)) return "constructor";
  // `[Symbol.dispose]()` needs the esnext.disposable lib; the package targets ES2022.
  if (!member.name || ts.isComputedPropertyName(member.name)) return null;
  const isStatic = ts.getModifiers?.(member)?.some((m) => m.kind === ts.SyntaxKind.StaticKeyword);
  const name = member.name.getText();
  if (ts.isMethodDeclaration(member)) return `${isStatic ? "static " : ""}${name}()`;
  if (ts.isPropertyDeclaration(member) || ts.isGetAccessorDeclaration(member) || ts.isSetAccessorDeclaration(member)) {
    return `${isStatic ? "static " : ""}${name}`;
  }
  return null;
}

function paramCount(node) {
  return node.parameters ? node.parameters.length : null;
}

// The declaration's text with its leading doc comment, re-indented.
function declarationText(node, sourceText, indent) {
  const start = node.getFullStart();
  const leading = ts.getLeadingCommentRanges(sourceText, start) ?? [];
  const docs = leading.filter((range) => sourceText.slice(range.pos, range.pos + 3) === "/**");
  const from = docs.length ? docs[0].pos : node.getStart();
  const raw = sourceText.slice(from, node.getEnd());
  return raw
    .split("\n")
    .map((line) => indent + line.replace(/^\s+/, ""))
    .join("\n")
    .replace(/^(\s*)\*/gm, "$1 *");
}

const generated = parse(resolve(generatedPath));
let hand = parse(handPath);
const missing = [];
const mismatched = [];
const insertions = [];

for (const [name, cls] of generated.classes) {
  const handClass = hand.classes.get(name);
  if (!handClass) {
    missing.push(`class ${name}`);
    // Rebuild the class from the members compared above, which leaves out
    // `[Symbol.dispose]()` (it needs the esnext.disposable lib).
    const header = declarationText(cls.node, generated.text, "");
    const opening = header.slice(0, header.indexOf("{", header.indexOf(`class ${name}`)) + 1);
    const body = [...cls.members.values()].map((member) => declarationText(member.node, generated.text, "  "));
    insertions.push({ at: hand.text.length, text: `\n${opening}\n${body.join("\n")}\n}\n` });
    continue;
  }
  const added = [];
  for (const [key, member] of cls.members) {
    const handMember = handClass.members.get(key);
    if (!handMember) {
      missing.push(`${name}.${key}`);
      added.push(declarationText(member.node, generated.text, "  "));
    } else if (member.params !== null && handMember.params !== member.params) {
      mismatched.push(`${name}.${key}: ${handMember.params} parameter(s), generated ${member.params}`);
    }
  }
  if (added.length) {
    const close = handClass.node.getEnd() - 1; // the class's closing brace
    insertions.push({ at: close, text: `${added.join("\n")}\n` });
  }
}
for (const [name, fn] of generated.functions) {
  const handFn = hand.functions.get(name);
  if (!handFn) {
    missing.push(`function ${name}`);
    insertions.push({ at: hand.text.length, text: `\n${declarationText(fn.node, generated.text, "")}\n` });
  } else if (handFn.params !== fn.params) {
    mismatched.push(`${name}(): ${handFn.params} parameter(s), generated ${fn.params}`);
  }
}

if (write && insertions.length) {
  let text = hand.text;
  for (const { at, text: insert } of insertions.sort((a, b) => b.at - a.at)) {
    text = text.slice(0, at) + insert + text.slice(at);
  }
  writeFileSync(handPath, text.endsWith("\n") ? text : `${text}\n`, "utf8");
  console.log(`added ${missing.length} declaration(s) to ${handPath}`);
}

const summary = `${generated.classes.size} classes, ${[...generated.classes.values()].reduce((n, c) => n + c.members.size, 0)} members, ${generated.functions.size} functions generated`;
const problems = [
  ...(write ? [] : missing.map((item) => `missing: ${item}`)),
  ...mismatched.map((item) => `parameter count differs: ${item}`),
];
if (problems.length) {
  console.error(`${handPath} is behind the wasm-bindgen declarations (${summary}):\n${problems.map((p) => `  ${p}`).join("\n")}`);
  process.exitCode = 1;
} else {
  console.log(`hand-written declarations cover the wasm-bindgen output (${summary})`);
}
