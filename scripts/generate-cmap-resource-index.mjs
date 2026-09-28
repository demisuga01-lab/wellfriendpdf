// Emit source (or an apply_patch update) from the vendored manifest. No file
// writes, network requests, engine execution or PDF workloads.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(readFileSync(resolve(root, "crates/engine/src/fonts/cmaps/manifest.json"), "utf8"));
if (manifest.format !== "wellpdfsdk.adobe-cmap-resources.v1") throw new Error("Unknown CMap manifest format");
const assets = manifest.resources;
const seen = new Set();
for (const entry of assets) {
  const key = `${entry.kind}:${entry.name}`;
  if (seen.has(key) || !["Cid", "Unicode"].includes(entry.kind) ||
      !/^[A-Za-z0-9_-]+$/.test(entry.name) || !/^[A-Za-z0-9_/-]+\.gz\.hex$/.test(entry.file) ||
      !/^[a-f0-9]{64}$/.test(entry.sha256) || !Number.isSafeInteger(entry.size) || entry.size < 1 || entry.size > 1048576 ||
      ![entry.registry,entry.ordering,entry.collection].every(value => typeof value === "string" && /^[A-Za-z0-9_-]+$/.test(value)) ||
      !Number.isSafeInteger(Number(entry.supplement)) || Number(entry.supplement) < 0 || Number(entry.supplement) > 4294967295 || !["", "0", "1"].includes(entry.mode) ||
      ![0,1,2,3,4].includes(entry.code_size) || ![0,2,8,16,32].includes(entry.unicode_encoding)) {
    throw new Error(`Invalid or duplicate CMap manifest entry: ${key}`);
  }
  seen.add(key);
}
const quoted = JSON.stringify;
let output = "// Generated from pinned Adobe resources. See cmaps/manifest.json and licences.\nuse super::{Asset,Kind};\nuse crate::fonts::predefined_cmap::PredefinedCMapInfo;\n";
output += "pub(in crate::fonts::predefined_cmap) static ASSETS:&[Asset]=&[\n";
for (const a of assets) output += `Asset {name:${quoted(a.name)},kind:Kind::${a.kind},collection:${quoted(a.collection)},registry:${quoted(a.registry)},ordering:${quoted(a.ordering)},supplement:${Number(a.supplement)},vertical:${a.mode === "1"},code_size:${a.code_size},unicode_encoding:${a.unicode_encoding},source_sha256:${quoted(a.sha256)},decoded_len:${a.size},gzip_hex:include_str!(${quoted("cmaps/" + a.file)})},\n`;
output += "];\npub(in crate::fonts::predefined_cmap) static METADATA:&[PredefinedCMapInfo]=&[\n";
for (const a of assets.filter(a => a.kind === "Cid")) output += `PredefinedCMapInfo {name:${quoted(a.name)},collection:${quoted(a.collection)},vertical:${a.mode === "1"},code_size:${a.code_size},unicode_preserving:${a.unicode_encoding !== 0}},\n`;
output += "];\n";
if (process.argv.includes("--apply-patch")) {
  const target = "crates/engine/src/fonts/predefined_resource_data.rs";
  const before = readFileSync(resolve(root, target), "utf8").replaceAll("\r", "").trimEnd();
  output = `*** Begin Patch\n*** Update File: ${target}\n@@\n${before.split("\n").map(line => "-" + line).join("\n")}\n${output.trimEnd().split("\n").map(line => "+" + line).join("\n")}\n*** End Patch\n`;
}
process.stdout.write(output);
