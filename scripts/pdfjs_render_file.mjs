#!/usr/bin/env node
import { readFileSync } from "node:fs";
import process from "node:process";
import { createCanvas } from "@napi-rs/canvas";
import * as pdfjsLib from "pdfjs-dist/legacy/build/pdf.mjs";

function parseArgs(argv) {
  const args = { pages: "first", dpi: 72 };
  for (let i = 2; i < argv.length; i += 1) {
    const key = argv[i];
    const value = argv[i + 1];
    if (key === "--input") {
      args.input = value;
      i += 1;
    } else if (key === "--pages") {
      args.pages = value;
      i += 1;
    } else if (key === "--dpi") {
      args.dpi = Number(value);
      i += 1;
    }
  }
  if (!args.input) {
    throw new Error("missing --input");
  }
  if (args.pages !== "first" && args.pages !== "all") {
    throw new Error("invalid --pages");
  }
  if (!Number.isFinite(args.dpi) || args.dpi <= 0) {
    throw new Error("invalid --dpi");
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv);
  const data = new Uint8Array(readFileSync(args.input));
  const doc = await pdfjsLib.getDocument({
    data,
    disableWorker: true,
    useSystemFonts: true,
  }).promise;
  const pageCount = doc.numPages;
  const maxPage = args.pages === "all" ? pageCount : Math.min(1, pageCount);
  const scale = args.dpi / 72.0;
  let pagesRendered = 0;
  let checksum = 0xcbf29ce484222325n;
  for (let pageNumber = 1; pageNumber <= maxPage; pageNumber += 1) {
    const page = await doc.getPage(pageNumber);
    const viewport = page.getViewport({ scale });
    const canvas = createCanvas(Math.ceil(viewport.width), Math.ceil(viewport.height));
    const context = canvas.getContext("2d");
    await page.render({ canvasContext: context, viewport }).promise;
    const imageData = context.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let i = 0; i < imageData.length; i += 4096) {
      checksum ^= BigInt(imageData[i]);
      checksum = (checksum * 0x100000001b3n) & 0xffffffffffffffffn;
    }
    pagesRendered += 1;
    page.cleanup();
  }
  if (typeof doc.destroy === "function") {
    await doc.destroy();
  } else if (typeof doc.cleanup === "function") {
    doc.cleanup();
  }
  console.log(JSON.stringify({
    page_count: pageCount,
    pages_rendered: pagesRendered,
    sampled_fnv1a64: checksum.toString(16).padStart(16, "0"),
  }));
}

main().catch((error) => {
  console.error(error && error.stack ? error.stack : String(error));
  process.exit(1);
});
