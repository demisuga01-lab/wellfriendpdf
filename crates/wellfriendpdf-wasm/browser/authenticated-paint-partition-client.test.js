import test from "node:test";
import assert from "node:assert/strict";
import {
  AuthenticatedPaintPartitionHttpClient,
  parsePaintPartitionApplyMultipart,
} from "./authenticated-paint-partition-client.js";

const encode = value => new TextEncoder().encode(value);

// Fixtures mirror the server envelopes (crates/server/src/routes/universal_editing.rs,
// crates/engine/src/universal_scoped_preview.rs) that the client validates strictly.
const applyReport = {
  schema_version: 1,
  kind: "advanced_editing_closeout_multi_run_text_edit_report",
  report: { status: "applied" },
};
const publicationReceipt = {
  schema_version: "advanced_editing.paint-partition-publication-receipt.v1",
  proposal_id: "proposal",
  input_sha256: "1".repeat(64),
  request_sha256: "2".repeat(64),
  approval_sha256: "3".repeat(64),
  font_sha256: null,
  candidate_output_sha256: "4".repeat(64),
  preview_evidence_sha256: "5".repeat(64),
  receipt_id: "6".repeat(64),
};
const authenticatedReceipt = {
  schema_version: "advanced_editing.paint-partition-authenticated-publication-receipt.v1",
  key_id: "primary",
  audience: "wellfriendpdf-server",
  issued_at_unix: 1_700_000_000,
  expires_at_unix: 1_700_000_900,
  publication_receipt: publicationReceipt,
  hmac_sha256: "a".repeat(64),
};
const previewEnvelope = {
  schema_version: 1,
  kind: "advanced_editing_closeout_paint_partition_preview",
  report: { publication_receipt: publicationReceipt, authenticated_publication_receipt: authenticatedReceipt },
};

test("multipart parser returns exact PDF bytes and metadata", () => {
  const boundary = "wellfriendpdf-universal-v2-test";
  const pdf = encode("%PDF-1.7\nexact\0bytes");
  const prefix = encode(`--${boundary}\r\nContent-Type: application/json\r\nContent-Disposition: form-data; name="metadata"\r\n\r\n${JSON.stringify(applyReport)}\r\n--${boundary}\r\nContent-Type: application/pdf\r\nContent-Disposition: form-data; name="document"; filename="edited.pdf"\r\n\r\n`);
  const suffix = encode(`\r\n--${boundary}--\r\n`);
  const body = new Uint8Array(prefix.length + pdf.length + suffix.length);
  body.set(prefix); body.set(pdf, prefix.length); body.set(suffix, prefix.length + pdf.length);
  const parsed = parsePaintPartitionApplyMultipart(body, `multipart/mixed; boundary=${boundary}`);
  assert.deepEqual(parsed.report, applyReport);
  assert.deepEqual(parsed.document, pdf);
});

test("authenticated client never sends an HMAC key and consumes the signed receipt", async () => {
  const calls = [];
  const fetchImpl = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(previewEnvelope), {
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
