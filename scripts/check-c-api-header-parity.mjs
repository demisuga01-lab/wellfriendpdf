#!/usr/bin/env node
// Source-level ABI guard. This deliberately does not load or execute the
// native library; it ensures every explicitly declared Rust C export is also
// declared in the shipped public header.
import { readdirSync, readFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(process.argv[2] ?? fileURLToPath(new URL("..", import.meta.url)));
const sourceRoot = join(root, "crates", "wellfriendpdf-capi", "src");
const headerPath = join(root, "crates", "wellfriendpdf-capi", "include", "wellfriendpdf.h");
const dotnetRoot = join(root, "bindings", "dotnet", "WellfriendPdf");
const javaPath = join(root, "bindings", "java", "src", "main", "java", "io", "wellfriendpdf", "WellfriendPdf.java");

function rustFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...rustFiles(path));
    else if (extname(entry.name) === ".rs" && !entry.name.endsWith("_tests.rs")) files.push(path);
  }
  return files;
}

function filesWithExtension(directory, extension) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...filesWithExtension(path, extension));
    else if (extname(entry.name) === extension) files.push(path);
  }
  return files;
}

function names(text, pattern) {
  return new Set(Array.from(text.matchAll(pattern), (match) => match[1]));
}

const rustExports = new Set();
for (const path of rustFiles(sourceRoot)) {
  const source = readFileSync(path, "utf8");
  for (const name of names(
    source,
    /\bpub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(wellfriendpdf_[A-Za-z0-9_]+)\s*\(/g,
  )) rustExports.add(name);
}

const header = readFileSync(headerPath, "utf8");
const headerDeclarations = names(
  header,
  /\b(wellfriendpdf_[A-Za-z0-9_]+)\s*\(/g,
);
const missing = [...rustExports].filter((name) => !headerDeclarations.has(name)).sort();
const dotnetImports = new Set();
for (const path of filesWithExtension(dotnetRoot, ".cs")) {
  const source = readFileSync(path, "utf8");
  for (const name of names(
    source,
    /\b(?:internal|private|public)\s+static\s+extern\s+[A-Za-z0-9_<>,.\[\]?]+\s+(wellfriendpdf_[A-Za-z0-9_]+)\s*\(/g,
  )) dotnetImports.add(name);
}
const javaImports = names(
  readFileSync(javaPath, "utf8"),
  /"(wellfriendpdf_[A-Za-z0-9_]+)"/g,
);
const missingDotnet = [...dotnetImports].filter((name) => !headerDeclarations.has(name)).sort();
const missingJava = [...javaImports].filter((name) => !headerDeclarations.has(name)).sort();

if (missing.length || missingDotnet.length || missingJava.length) {
  process.stderr.write(
    [
      missing.length ? `Public C header is missing ${missing.length} explicit Rust export(s):\n${missing.map((name) => `  ${name}`).join("\n")}` : "",
      missingDotnet.length ? `Public C header is missing ${missingDotnet.length} .NET import(s):\n${missingDotnet.map((name) => `  ${name}`).join("\n")}` : "",
      missingJava.length ? `Public C header is missing ${missingJava.length} Java import(s):\n${missingJava.map((name) => `  ${name}`).join("\n")}` : "",
    ].filter(Boolean).join("\n") + "\n",
  );
  process.exitCode = 1;
} else {
  process.stdout.write(
    `C header covers ${rustExports.size} explicit Rust exports, ${dotnetImports.size} .NET imports, and ${javaImports.size} Java imports (${headerDeclarations.size} total declarations).\n`,
  );
}
