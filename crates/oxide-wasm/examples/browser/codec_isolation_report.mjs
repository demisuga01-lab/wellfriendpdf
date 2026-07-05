import init, { OxidePdf } from "./pkg/oxide_wasm.js";

await init();

const encoded = new Uint8Array([
  0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0xaf,
  0xc8, 0x4c, 0x49, 0x05, 0x00, 0x19, 0xdd, 0x04, 0x4e,
]);

const envelope = JSON.parse(
  OxidePdf.codecIsolationReportJson("FlateDecode", encoded, "in_process"),
);

console.log(JSON.stringify(envelope, null, 2));
if (envelope.report.status !== "success") {
  throw new Error(`codec isolation status: ${envelope.report.status}`);
}
