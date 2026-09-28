// Maintenance-only source transcription. Prints the Rust override table; does
// not edit files, invoke Cargo, run SDK code, download data or process PDFs.
// Run with the path to Rustybuzz 0.20.1's hb/ot_shaper_arabic_table.rs.
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";

const expectedHash = "5b5e3c33bd8743caadc5b3cd72f7608982ce071277569da3322118f5f5001a02";
if (process.argv.length !== 3) throw new Error("Expected exactly one upstream table path");
const bytes = readFileSync(process.argv[2]);
if (createHash("sha256").update(bytes).digest("hex") !== expectedHash) {
  throw new Error("Upstream table is not the pinned source; review dependency/data changes first");
}
const source = bytes.toString("utf8");
const tableSource = /pub const JOINING_TABLE:[\s\S]*?=\s*&\[([\s\S]*?)\];/.exec(source)?.[1];
if (!tableSource) throw new Error("Missing joining table");
const table = tableSource.replace(/\/\*[\s\S]*?\*\//g, "").split(",").map(s => s.trim()).filter(Boolean);
if (table.length !== 1434 || table.some(type => !/^(U|L|R|D|A|DR|T|X)$/.test(type))) {
  throw new Error("Unexpected joining table shape");
}
const offsets = new Map([...source.matchAll(/const (JOINING_OFFSET_0X[\dA-F]+): usize = (\d+);/g)]
  .map(([, name, offset]) => [name, Number(offset)]));
const dispatch = [...source.matchAll(/if \((0x[\dA-F]+)\.\.=(0x[\dA-F]+)\)\.contains\(&u\)\s*\{\s*return JOINING_TABLE\[u as usize - (0x[\dA-F]+) \+ (JOINING_OFFSET_0X[\dA-F]+)\];/g)];
if (offsets.size !== 11 || dispatch.length !== 11) throw new Error("Unexpected dispatch count");
const overrides = [];
let consumed = 0;
let lastCodepoint = -1;
for (const [, first, last, base, name] of dispatch) {
  const start = Number(first), end = Number(last), offset = offsets.get(name);
  if (Number(base) !== start || offset !== consumed || start <= lastCodepoint || end < start) {
    throw new Error("Dispatch/index mismatch");
  }
  for (let code = start; code <= end; code++) {
    const type = table[consumed++];
    if (type === undefined) throw new Error("Dispatch exceeds table");
    if (type === "X") continue;
    const transparent = type === "T";
    const previous = overrides.at(-1);
    if (previous && previous[1] + 1 === code && previous[2] === transparent) previous[1] = code;
    else overrides.push([code, code, transparent]);
  }
  lastCodepoint = end;
}
if (consumed !== table.length || overrides.length !== 49) throw new Error("Incomplete transcription");
const hex = value => `0x${value.toString(16).toUpperCase()}`;
process.stdout.write("#[rustfmt::skip]\nconst OVERRIDES: &[(u32, u32, bool)] = &[\n" +
  overrides.map(([start, end, transparent]) => `    (${hex(start)}, ${hex(end)}, ${transparent}),\n`).join("") + "];\n");
