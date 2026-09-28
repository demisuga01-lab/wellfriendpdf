const DEFAULT_MAX_DOCUMENT_BYTES = 256 * 1024 * 1024;
const DEFAULT_MAX_JSON_BYTES = 16 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES = 384 * 1024 * 1024;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

function requireBytes(value, name, maxBytes, allowEmpty = false) {
  if (!(value instanceof Uint8Array)) throw new TypeError(`${name} must be a Uint8Array`);
  if ((!allowEmpty && value.byteLength === 0) || value.byteLength > maxBytes) {
    throw new RangeError(`${name} must contain ${allowEmpty ? "0..=" : "1..="}${maxBytes} bytes`);
  }
  return value;
}

function canonicalJson(value, name, maxBytes) {
  if (typeof value === "string") {
    if (!value.trim()) throw new TypeError(`${name} must not be empty`);
    try { JSON.parse(value); } catch (error) { throw new TypeError(`${name} is not valid JSON`, { cause: error }); }
    if (encoder.encode(value).byteLength > maxBytes) throw new RangeError(`${name} exceeds ${maxBytes} UTF-8 bytes`);
    return value;
  }
  if (value == null || typeof value !== "object") throw new TypeError(`${name} must be a JSON object or JSON string`);
  let json;
  try { json = JSON.stringify(value); }
  catch (error) { throw new TypeError(`${name} is not JSON-serializable`, { cause: error }); }
  if (encoder.encode(json).byteLength > maxBytes) throw new RangeError(`${name} exceeds ${maxBytes} UTF-8 bytes`);
  return json;
}

function requireRecord(value, name) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${name} must be a JSON object`);
  }
  return value;
}

function requireEnvelope(value, kind, name) {
  const envelope = requireRecord(value, name);
  if (envelope.schema_version !== 1 || envelope.kind !== kind) {
    throw new TypeError(`${name} has an unsupported schema or kind`);
  }
  requireRecord(envelope.report, `${name}.report`);
  return envelope;
}

function requireHexDigest(value, name) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    throw new TypeError(`${name} must be a lowercase SHA-256 hex digest`);
  }
}

function validatePublicationReceipt(value, name = "publication receipt") {
  const receipt = requireRecord(value, name);
  if (receipt.schema_version !== "advanced_editing.paint-partition-publication-receipt.v1"
      || typeof receipt.proposal_id !== "string" || receipt.proposal_id.length === 0) {
    throw new TypeError(`${name} has an unsupported schema or missing proposal id`);
  }
  for (const field of ["input_sha256", "request_sha256", "approval_sha256", "candidate_output_sha256", "preview_evidence_sha256", "receipt_id"]) {
    requireHexDigest(receipt[field], `${name}.${field}`);
  }
  if (receipt.font_sha256 != null) requireHexDigest(receipt.font_sha256, `${name}.font_sha256`);
  return receipt;
}

function samePublicationReceipt(left, right) {
  return ["schema_version", "proposal_id", "input_sha256", "request_sha256", "approval_sha256",
    "font_sha256", "candidate_output_sha256", "preview_evidence_sha256", "receipt_id"]
    .every((field) => left[field] === right[field]);
}

function validateAuthenticatedReceipt(value, expectedPublicationReceipt) {
  const receipt = requireRecord(value, "authenticated publication receipt");
  if (receipt.schema_version !== "advanced_editing.paint-partition-authenticated-publication-receipt.v1"
      || typeof receipt.key_id !== "string" || receipt.key_id.length === 0
      || typeof receipt.audience !== "string" || receipt.audience.length === 0
      || !Number.isSafeInteger(receipt.issued_at_unix) || receipt.issued_at_unix < 0
      || !Number.isSafeInteger(receipt.expires_at_unix) || receipt.expires_at_unix <= receipt.issued_at_unix) {
    throw new TypeError("authenticated publication receipt has invalid claims");
  }
  requireHexDigest(receipt.hmac_sha256, "authenticated publication receipt.hmac_sha256");
  const nested = validatePublicationReceipt(receipt.publication_receipt, "authenticated publication receipt.publication_receipt");
  if (expectedPublicationReceipt && !samePublicationReceipt(nested, expectedPublicationReceipt)) {
    throw new TypeError("authenticated receipt does not wrap the preview publication receipt");
  }
  return receipt;
}

function optionalBytes(value, name, maxBytes) {
  return value == null ? undefined : requireBytes(value, name, maxBytes);
}

function appendBytes(form, name, bytes, filename) {
  if (bytes == null) return;
  form.append(name, new Blob([bytes]), filename);
}

function appendJson(form, name, value, maxBytes) {
  form.append(name, canonicalJson(value, name, maxBytes));
}

function parseBoundary(contentType) {
  if (typeof contentType !== "string" || !/^multipart\/mixed\b/i.test(contentType)) {
    throw new TypeError("apply response is not multipart/mixed");
  }
  const match = /(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
  const boundary = match?.[1] ?? match?.[2];
  if (!boundary || boundary.length > 200 || /[\r\n]/.test(boundary)) {
    throw new TypeError("apply response has an invalid multipart boundary");
  }
  return boundary;
}

function findBytes(haystack, needle, from = 0) {
  outer: for (let index = from; index + needle.length <= haystack.length; index++) {
    for (let offset = 0; offset < needle.length; offset++) {
      if (haystack[index + offset] !== needle[offset]) continue outer;
    }
    return index;
  }
  return -1;
}

function decodeHeaders(bytes) {
  const text = decoder.decode(bytes);
  const headers = new Map();
  for (const line of text.split("\r\n")) {
    const separator = line.indexOf(":");
    if (separator <= 0) throw new TypeError("multipart response contains a malformed header");
    const name = line.slice(0, separator).trim().toLowerCase();
    const value = line.slice(separator + 1).trim();
    if (!name || headers.has(name)) throw new TypeError("multipart response contains duplicate or empty headers");
    headers.set(name, value);
  }
  return headers;
}

function parseDisposition(value) {
  if (!value) return undefined;
  const match = /(?:^|;)\s*name="([^"]+)"/i.exec(value);
  return match?.[1];
}

export function parsePaintPartitionApplyMultipart(bytes, contentType, limits = {}) {
  const maxResponseBytes = limits.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES;
  const maxJsonBytes = limits.maxJsonBytes ?? DEFAULT_MAX_JSON_BYTES;
  if (!Number.isSafeInteger(maxResponseBytes) || maxResponseBytes <= 0
      || !Number.isSafeInteger(maxJsonBytes) || maxJsonBytes <= 0) {
    throw new RangeError("multipart limits must be positive safe integers");
  }
  requireBytes(bytes, "multipart response", maxResponseBytes);
  const boundary = parseBoundary(contentType);
  const opening = encoder.encode(`--${boundary}\r\n`);
  const separator = encoder.encode(`\r\n--${boundary}`);
  const headerEnd = encoder.encode("\r\n\r\n");
  if (findBytes(bytes, opening, 0) !== 0) throw new TypeError("multipart response does not start with its boundary");

  let cursor = opening.length;
  const parts = new Map();
  for (let count = 0; count < 4; count++) {
    const headerStop = findBytes(bytes, headerEnd, cursor);
    if (headerStop < 0 || headerStop - cursor > 16 * 1024) throw new TypeError("multipart response headers are missing or oversized");
    const headers = decodeHeaders(bytes.subarray(cursor, headerStop));
    const bodyStart = headerStop + headerEnd.length;
    const boundaryAt = findBytes(bytes, separator, bodyStart);
    if (boundaryAt < 0) throw new TypeError("multipart response part is unterminated");
    const name = parseDisposition(headers.get("content-disposition"));
    if (!name || parts.has(name)) throw new TypeError("multipart response has an unnamed or duplicate part");
    parts.set(name, { headers, body: bytes.slice(bodyStart, boundaryAt) });
    cursor = boundaryAt + separator.length;
    if (bytes[cursor] === 45 && bytes[cursor + 1] === 45) {
      cursor += 2;
      if (bytes[cursor] === 13 && bytes[cursor + 1] === 10) cursor += 2;
      if (cursor !== bytes.length) throw new TypeError("multipart response contains bytes after its closing boundary");
      break;
    }
    if (bytes[cursor] !== 13 || bytes[cursor + 1] !== 10) throw new TypeError("multipart response boundary delimiter is malformed");
    cursor += 2;
  }

  if (parts.size !== 2 || !parts.has("metadata") || !parts.has("document")) {
    throw new TypeError("multipart response must contain exactly metadata and document parts");
  }
  const metadata = parts.get("metadata");
  const document = parts.get("document");
  if (!/^application\/json\b/i.test(metadata.headers.get("content-type") ?? "")) {
    throw new TypeError("metadata part is not application/json");
  }
  if (!/^application\/pdf\b/i.test(document.headers.get("content-type") ?? "")) {
    throw new TypeError("document part is not application/pdf");
  }
  if (metadata.body.byteLength > maxJsonBytes) throw new RangeError("metadata part exceeds the JSON budget");
  let report;
  try { report = JSON.parse(decoder.decode(metadata.body)); }
  catch (error) { throw new TypeError("metadata part is not valid UTF-8 JSON", { cause: error }); }
  requireEnvelope(report, "advanced_editing_closeout_multi_run_text_edit_report", "apply metadata");
  if (document.body.byteLength < 5 || decoder.decode(document.body.subarray(0, 5)) !== "%PDF-") {
    throw new TypeError("document part does not start with a PDF header");
  }
  return { report, document: document.body };
}

async function boundedResponseBytes(response, maxBytes) {
  const declared = response.headers.get("content-length");
  if (declared != null) {
    const length = Number(declared);
    if (!Number.isSafeInteger(length) || length < 0 || length > maxBytes) {
      throw new RangeError(`response Content-Length exceeds ${maxBytes} bytes`);
    }
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > maxBytes) throw new RangeError(`response exceeds ${maxBytes} bytes`);
  return bytes;
}

async function errorFromResponse(response, maxBytes) {
  let detail = `${response.status} ${response.statusText}`.trim();
  try {
    const bytes = await boundedResponseBytes(response, Math.min(maxBytes, 1024 * 1024));
    const text = decoder.decode(bytes).trim();
    if (text) {
      try {
        const parsed = JSON.parse(text);
        detail = parsed?.error?.message ?? parsed?.message ?? text;
      } catch { detail = text; }
    }
  } catch { /* Preserve status-only error when the response is malformed. */ }
  const error = new Error(`Wellfriend PDF server rejected the request: ${detail}`);
  error.status = response.status;
  return error;
}

function requireJsonResponse(response, name) {
  if (!/^application\/json\b/i.test(response.headers.get("content-type") ?? "")) {
    throw new TypeError(`${name} is not application/json`);
  }
}

/** Browser-safe remote client for server-authenticated paint-partition review.
 * It never accepts or receives the server HMAC key. The application remains
 * responsible for user authorization and for displaying every preview image.
 */
export class AuthenticatedPaintPartitionHttpClient {
  #baseUrl; #apiKey; #fetch; #credentials; #maxDocumentBytes; #maxJsonBytes; #maxResponseBytes;
  constructor(options) {
    if (!options || typeof options !== "object") throw new TypeError("client options are required");
    const baseUrl = new URL(options.baseUrl, globalThis.location?.href);
    if (!/^https?:$/.test(baseUrl.protocol)) throw new TypeError("baseUrl must use http or https");
    if (baseUrl.username || baseUrl.password || baseUrl.search || baseUrl.hash) {
      throw new TypeError("baseUrl must not contain credentials, a query, or a fragment");
    }
    if (!baseUrl.pathname.endsWith("/")) baseUrl.pathname += "/";
    this.#baseUrl = baseUrl;
    this.#apiKey = options.apiKey;
    if (this.#apiKey != null && (typeof this.#apiKey !== "string" || !this.#apiKey)) throw new TypeError("apiKey must be a non-empty string");
    this.#fetch = options.fetchImpl ?? globalThis.fetch;
    if (typeof this.#fetch !== "function") throw new TypeError("a fetch implementation is required");
    this.#credentials = options.credentials ?? "same-origin";
    this.#maxDocumentBytes = options.maxDocumentBytes ?? DEFAULT_MAX_DOCUMENT_BYTES;
    this.#maxJsonBytes = options.maxJsonBytes ?? DEFAULT_MAX_JSON_BYTES;
    this.#maxResponseBytes = options.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES;
    for (const [name, value] of [["maxDocumentBytes", this.#maxDocumentBytes], ["maxJsonBytes", this.#maxJsonBytes], ["maxResponseBytes", this.#maxResponseBytes]]) {
      if (!Number.isSafeInteger(value) || value <= 0) throw new RangeError(`${name} must be a positive safe integer`);
    }
  }

  #url(path) { return new URL(path.replace(/^\//, ""), this.#baseUrl).toString(); }
  #headers() { return this.#apiKey == null ? {} : { "X-API-Key": this.#apiKey }; }
  async #post(path, form, signal) {
    const response = await this.#fetch(this.#url(path), {
      method: "POST", body: form, headers: this.#headers(), credentials: this.#credentials, signal,
    });
    if (!response.ok) throw await errorFromResponse(response, this.#maxJsonBytes);
    return response;
  }
  #baseForm(pdf, password) {
    const form = new FormData();
    appendBytes(form, "file", requireBytes(pdf, "pdf", this.#maxDocumentBytes), "input.pdf");
    appendBytes(form, "password", optionalBytes(password, "password", 4096), "password.bin");
    return form;
  }

  async propose({ pdf, request, password, signal } = {}) {
    const form = this.#baseForm(pdf, password);
    appendJson(form, "request_json", request, this.#maxJsonBytes);
    const response = await this.#post("api/v2/universal-editing/paint-partition/propose", form, signal);
    requireJsonResponse(response, "proposal response");
    const bytes = await boundedResponseBytes(response, this.#maxJsonBytes);
    try {
      const envelope = requireEnvelope(
        JSON.parse(decoder.decode(bytes)),
        "advanced_editing_closeout_paint_partition_proposal",
        "proposal response",
      );
      if (typeof envelope.report.proposal_id !== "string" || !Array.isArray(envelope.report.candidates)) {
        throw new TypeError("proposal response has an invalid report shape");
      }
      return envelope;
    }
    catch (error) { throw new TypeError("proposal response is not valid UTF-8 JSON", { cause: error }); }
  }

  async previewAuthenticated({ pdf, request, proposal, approval, fontBytes, options, password, signal } = {}) {
    const form = this.#baseForm(pdf, password);
    appendJson(form, "request_json", request, this.#maxJsonBytes);
    appendJson(form, "proposal_json", proposal, this.#maxJsonBytes);
    appendJson(form, "approval_json", approval, this.#maxJsonBytes);
    if (options != null) appendJson(form, "options_json", options, this.#maxJsonBytes);
    appendBytes(form, "font", optionalBytes(fontBytes, "fontBytes", 4 * 1024 * 1024), "approved-font.bin");
    const response = await this.#post("api/v2/universal-editing/paint-partition/preview-authenticated", form, signal);
    requireJsonResponse(response, "authenticated preview response");
    const bytes = await boundedResponseBytes(response, this.#maxJsonBytes);
    let envelope;
    try {
      envelope = requireEnvelope(
        JSON.parse(decoder.decode(bytes)),
        "advanced_editing_closeout_paint_partition_preview",
        "authenticated preview response",
      );
    }
    catch (error) { throw new TypeError("preview response is not valid UTF-8 JSON", { cause: error }); }
    const publicationReceipt = validatePublicationReceipt(envelope.report.publication_receipt);
    validateAuthenticatedReceipt(envelope.report.authenticated_publication_receipt, publicationReceipt);
    return envelope;
  }

  async applyAuthenticated({ pdf, request, proposal, approval, authenticatedReceipt, fontBytes, password, signal } = {}) {
    const form = this.#baseForm(pdf, password);
    appendJson(form, "request_json", request, this.#maxJsonBytes);
    appendJson(form, "proposal_json", proposal, this.#maxJsonBytes);
    appendJson(form, "approval_json", approval, this.#maxJsonBytes);
    const authenticatedReceiptJson = canonicalJson(
      authenticatedReceipt,
      "authenticated_publication_receipt_json",
      this.#maxJsonBytes,
    );
    const authenticatedReceiptValue = JSON.parse(authenticatedReceiptJson);
    validateAuthenticatedReceipt(authenticatedReceiptValue);
    form.append("authenticated_publication_receipt_json", authenticatedReceiptJson);
    appendBytes(form, "font", optionalBytes(fontBytes, "fontBytes", 4 * 1024 * 1024), "approved-font.bin");
    const response = await this.#post("api/v2/universal-editing/paint-partition/apply-authenticated", form, signal);
    const bytes = await boundedResponseBytes(response, this.#maxResponseBytes);
    return parsePaintPartitionApplyMultipart(bytes, response.headers.get("content-type"), {
      maxResponseBytes: this.#maxResponseBytes,
      maxJsonBytes: this.#maxJsonBytes,
    });
  }
}
