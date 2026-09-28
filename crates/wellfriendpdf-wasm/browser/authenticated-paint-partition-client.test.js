import test from "node:test";
import assert from "node:assert/strict";
import {
  AuthenticatedPaintPartitionHttpClient,
  parsePaintPartitionApplyMultipart,
} from "./authenticated-paint-partition-client.js";

const encode = value => new TextEncoder().encode(value);

test("multipart parser returns exact PDF bytes and metadata", () => {
  const boundary = "wellfriendpdf-universal-v2-test";
  const pdf = encode("%PDF-1.7\nexact\0bytes");
  const prefix = encode(`--${boundary}\r\nContent-Type: application/json\r\nContent-Disposition: form-data; name="metadata"\r\n\r\n{"kind":"ok"}\r\n--${boundary}\r\nContent-Type: application/pdf\r\nContent-Disposition: form-data; name="document"; filename="edited.pdf"\r\n\r\n`);
  const suffix = encode(`\r\n--${boundary}--\r\n`);
  const body = new Uint8Array(prefix.length + pdf.length + suffix.length);
  body.set(prefix); body.set(pdf, prefix.length); body.set(suffix, prefix.length + pdf.length);
  const parsed = parsePaintPartitionApplyMultipart(body, `multipart/mixed; boundary=${boundary}`);
  assert.deepEqual(parsed.report, { kind: "ok" });
  assert.deepEqual(parsed.document, pdf);
});

test("authenticated client never sends an HMAC key and consumes the signed receipt", async () => {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify({ report: { authenticated_publication_receipt: { hmac_sha256: "a".repeat(64) } } }), {
      status: 200, headers: { "content-type": "application/json" },
    });
  };
  const client = new AuthenticatedPaintPartitionHttpClient({ baseUrl: "https://example.test/", apiKey: "api", fetchImpl });
  const result = await client.previewAuthenticated({
    pdf: encode("%PDF-1.7"), request: {}, proposal: {}, approval: {},
  });
  assert.equal(result.report.authenticated_publication_receipt.hmac_sha256.length, 64);
  assert.equal(calls[0].init.headers["X-API-Key"], "api");
  const names = [...calls[0].init.body.keys()];
  assert.ok(!names.some(name => /hmac|key/i.test(name)));
});
