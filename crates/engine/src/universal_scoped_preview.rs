//! Bounded before/candidate rendering for a canonical native scoped-text plan.
//! This is a review aid, not permission to publish candidate PDF bytes and not
//! an independent renderer, conformance, or visual-fidelity certification.
use super::*;
use crate::images::encoder::ImageEncoder;
use crate::render::{PixelBuffer, RenderContract, RenderMode};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const MAX_PAGES: usize = 8;
const MAX_PIXELS: u64 = 32_000_000; // Both images, across all requested pages.
const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
const MIN_RECEIPT_AUTH_KEY_BYTES: usize = 32;
const MAX_RECEIPT_AUTH_KEY_BYTES: usize = 256;
const MAX_RECEIPT_AUTH_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;

fn default_dpi() -> u32 {
    96
}
fn default_pixels() -> u64 {
    16_000_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedPreviewOptions {
    /// One-based pages. Empty means the selected occurrence's page, not all
    /// affected pages. The result discloses the remaining invalidation set.
    #[serde(default)]
    pub pages: Vec<usize>,
    #[serde(default = "default_dpi")]
    pub dpi: u32,
    #[serde(default)]
    pub require_exact: bool,
    #[serde(default = "default_pixels")]
    pub max_total_pixels: u64,
    /// Difference threshold only; it does not change either displayed PNG.
    #[serde(default)]
    pub channel_tolerance: u8,
}

impl Default for ScopedPreviewOptions {
    fn default() -> Self {
        Self {
            pages: vec![],
            dpi: default_dpi(),
            require_exact: false,
            max_total_pixels: default_pixels(),
            channel_tolerance: 0,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ScopedPreviewRaster {
    /// JSON byte array. PNGs are opaque RGB, composited against white by the
    /// render contract. The hash below covers canonical premultiplied RGBA.
    pub png: Vec<u8>,
    pub png_sha256: String,
    pub rgba_sha256: String,
    pub diagnostics: Value,
}

#[derive(Debug, Serialize)]
pub struct ScopedPreviewPage {
    pub page: usize,
    pub width: u32,
    pub height: u32,
    pub before: ScopedPreviewRaster,
    pub candidate: ScopedPreviewRaster,
    pub difference: ScopedPixelDifference,
}

#[derive(Debug, Serialize)]
pub struct ScopedPixelDifference {
    pub channel_tolerance: u8,
    pub changed_pixels: u64,
    pub maximum_channel_delta: u8,
    /// Device pixels, top-left origin, right/bottom exclusive.
    pub bounds: Option<[u32; 4]>,
}

#[derive(Debug, Serialize)]
pub struct ScopedCandidatePreview {
    pub schema_version: &'static str,
    pub plan_id: String,
    pub revision_id: String,
    pub input_sha256: String,
    pub candidate_output_sha256: String,
    pub options: ScopedPreviewOptions,
    pub pages: Vec<ScopedPreviewPage>,
    pub total_pixels: u64,
    pub affected_pages_not_previewed: Vec<usize>,
    pub limitations: Vec<&'static str>,
}

/// Private before/candidate rendering for one exact generated paint-partition
/// approval. The candidate PDF is deliberately not part of this report.
#[derive(Debug, Serialize)]
pub struct PaintPartitionCandidatePreview {
    pub schema_version: &'static str,
    pub proposal_id: String,
    pub revision_id: String,
    pub request_sha256: String,
    pub approval_sha256: String,
    pub font_sha256: Option<String>,
    pub candidate_output_sha256: String,
    pub options: ScopedPreviewOptions,
    pub pages: Vec<ScopedPreviewPage>,
    pub total_pixels: u64,
    pub affected_pages_not_previewed: Vec<usize>,
    pub publication_receipt: PaintPartitionPublicationReceipt,
    pub edit_report: crate::advanced_editing::MultiRunTextEditReport,
    pub limitations: Vec<&'static str>,
}

/// Compact evidence token that a host must retain after showing the private
/// preview. It is a tamper-evident binding, not an authorization signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintPartitionPublicationReceipt {
    pub schema_version: String,
    pub proposal_id: String,
    pub input_sha256: String,
    pub request_sha256: String,
    pub approval_sha256: String,
    pub font_sha256: Option<String>,
    pub candidate_output_sha256: String,
    pub preview_evidence_sha256: String,
    pub receipt_id: String,
}

/// Host-authenticated wrapper for a content-bound publication receipt. The
/// HMAC key never appears in this value. `key_id` supports rotation while the
/// audience prevents a receipt issued for one service boundary being replayed
/// against another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedPaintPartitionPublicationReceipt {
    pub schema_version: String,
    pub key_id: String,
    pub audience: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub publication_receipt: PaintPartitionPublicationReceipt,
    pub hmac_sha256: String,
}

struct RenderedCandidatePages {
    pages: Vec<ScopedPreviewPage>,
    total_pixels: u64,
    affected_pages_not_previewed: Vec<usize>,
}

fn invalid(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("scoped candidate preview: {message}"))
}

fn selected_pages(
    options: &ScopedPreviewOptions,
    selected: usize,
    count: usize,
) -> Result<Vec<usize>> {
    if !(24..=600).contains(&options.dpi) {
        return Err(invalid("dpi must be in 24..=600"));
    }
    if options.max_total_pixels == 0 || options.max_total_pixels > MAX_PIXELS {
        return Err(invalid("max_total_pixels must be in 1..=32000000"));
    }
    let pages = if options.pages.is_empty() {
        vec![selected]
    } else {
        options.pages.clone()
    };
    if pages.len() > MAX_PAGES
        || pages.iter().any(|&page| page == 0 || page > count)
        || pages.iter().copied().collect::<BTreeSet<_>>().len() != pages.len()
    {
        return Err(invalid(
            "expected at most eight distinct, in-range, one-based pages",
        ));
    }
    Ok(pages)
}

fn render_candidate_pages(
    input: &[u8],
    candidate: &[u8],
    selected_page: usize,
    affected_pages: &[usize],
    options: &ScopedPreviewOptions,
) -> Result<RenderedCandidatePages> {
    let before = ContentEngine::open_bytes(input.to_vec())?;
    let after = ContentEngine::open_bytes(candidate.to_vec())?;
    let count = before.page_count()?;
    if count != after.page_count()? {
        return Err(invalid("candidate changed the page count"));
    }
    let pages = selected_pages(options, selected_page, count)?;
    let mode = if options.require_exact {
        RenderMode::HighQuality
    } else {
        RenderMode::Compat
    };
    let cancel = crate::cancel::current_cancel_token();
    // Preflight ALL dimensions before allocating any raster. Geometry equality
    // includes boxes, rotation and UserUnit, not merely rounded pixel sizes.
    let mut contracts = Vec::with_capacity(pages.len());
    let mut total_pixels = 0u64;
    for &page in &pages {
        cancel.check("scoped preview geometry")?;
        let a_page = before.document().get_page(page)?;
        let b_page = after.document().get_page(page)?;
        if a_page.media_box != b_page.media_box
            || a_page.crop_box != b_page.crop_box
            || a_page.rotate != b_page.rotate
            || a_page.user_unit != b_page.user_unit
        {
            return Err(invalid("candidate changed page geometry"));
        }
        let a = before.default_render_contract(page, options.dpi, mode)?;
        let b = after.default_render_contract(page, options.dpi, mode)?;
        if a.width != b.width || a.height != b.height {
            return Err(invalid("candidate has different rendered dimensions"));
        }
        let pair_pixels = u64::from(a.width)
            .checked_mul(u64::from(a.height))
            .and_then(|pixels| pixels.checked_mul(2))
            .ok_or_else(|| invalid("pixel count overflow"))?;
        total_pixels = total_pixels
            .checked_add(pair_pixels)
            .ok_or_else(|| invalid("pixel count overflow"))?;
        if total_pixels > options.max_total_pixels {
            return Err(WellfriendError::ResourceLimit(
                "scoped preview combined pixel budget exceeded; reduce pages or dpi".into(),
            ));
        }
        contracts.push((page, a, b));
    }
    let mut reports = Vec::with_capacity(pages.len());
    let mut encoded_bytes = 0usize;
    for (page, a_contract, b_contract) in contracts {
        cancel.check("scoped preview page")?;
        let (a, a_diagnostics) = render(&before, &a_contract, &cancel)?;
        let (b, b_diagnostics) = render(&after, &b_contract, &cancel)?;
        let difference = compare(
            a.width,
            a.height,
            a.rgba_bytes(),
            b.rgba_bytes(),
            options.channel_tolerance,
        )?;
        reports.push(ScopedPreviewPage {
            page,
            width: a.width,
            height: a.height,
            before: encode(&a, a_diagnostics, &mut encoded_bytes)?,
            candidate: encode(&b, b_diagnostics, &mut encoded_bytes)?,
            difference,
        });
    }
    Ok(RenderedCandidatePages {
        pages: reports,
        total_pixels,
        affected_pages_not_previewed: affected_pages
            .iter()
            .copied()
            .filter(|page| !pages.contains(page))
            .collect(),
    })
}

/// Re-plan from the current bytes, compare the entire immutable plan, then
/// render its private candidate. Never call apply or relax an approval policy.
pub fn preview_scoped_candidate(
    input: &[u8],
    plan: &UniversalEditPlanV2,
    options: &ScopedPreviewOptions,
) -> Result<ScopedCandidatePreview> {
    crate::cancel::check_current_cancel("scoped preview input")?;
    if !matches!(
        &plan.requested_operation,
        UniversalEditOperationV2::ScopedText { .. }
    ) {
        return Err(invalid("only native scoped-text plans are supported"));
    }
    // Reject unreasonable caller options before expensive candidate generation.
    selected_pages(options, 1, usize::MAX)?;
    let (canonical, staged, _) = plan_universal_edit_v2_staged(
        input,
        &UniversalEditRequestV2 {
            operation: plan.requested_operation.clone(),
            policy: plan.policy.clone(),
        },
    )?;
    if serde_json::to_value(&canonical).map_err(json_error)?
        != serde_json::to_value(plan).map_err(json_error)?
    {
        return Err(invalid(
            "plan is stale or was changed; re-plan from the current revision",
        ));
    }
    if !matches!(
        canonical.state,
        UniversalPlanStateV2::Ready | UniversalPlanStateV2::ApprovalRequired
    ) {
        return Err(invalid("a non-applicable plan has no reviewable candidate"));
    }
    let staged = staged.ok_or_else(|| invalid("missing private native candidate"))?;
    let output_hash = digest_hex_cancellable(&staged.bytes, "scoped preview candidate hash")?;
    let UniversalEditOperationV2::ScopedText { request } = &canonical.execution_operation else {
        return Err(invalid("canonical execution operation is not scoped text"));
    };
    if request.planned_output_sha256.as_deref() != Some(output_hash.as_str()) {
        return Err(invalid(
            "candidate differs from the plan's execution receipt",
        ));
    }
    let source_page = canonical
        .candidates
        .first()
        .ok_or_else(|| invalid("missing occurrence"))?
        .page;
    let affected_pages = staged.pages;
    let rendered =
        render_candidate_pages(input, &staged.bytes, source_page, &affected_pages, options)?;
    Ok(ScopedCandidatePreview {
        schema_version:"universal_editing.scoped-candidate-preview.v1",
        plan_id:canonical.plan_id, revision_id:canonical.revision_id,
        input_sha256:digest_hex_cancellable(input, "scoped preview input hash")?,
        candidate_output_sha256:output_hash, options:options.clone(), pages:rendered.pages,
        total_pixels:rendered.total_pixels,
        affected_pages_not_previewed:rendered.affected_pages_not_previewed,
        limitations:vec![
            "Native before/candidate comparison; not an independent renderer or a visual fidelity certificate.",
            "Compatibility mode may use renderer fallbacks; require_exact requests the HighQuality render mode without silent downgrade.",
            "Private native candidate before final rewrite, security and conformance gates; final apply can still refuse.",
            "Only requested pages and the default visible optional-content state are shown; hidden states are not qualified.",
            "Annotations and forms included; opaque white RGB PNGs; differences use canonical premultiplied RGBA.",
            "Review does not authorize mutation; apply still requires the canonical plan and explicit approval.",
        ],
    })
}

/// Recompute and apply an exact paint-partition proposal into a private
/// candidate, then render bounded before/candidate pages. Candidate PDF bytes
/// never leave this function.
pub fn preview_paint_partition_candidate(
    input: &[u8],
    request: &crate::advanced_editing::MultiRunTextRangeRequest,
    proposal: &crate::advanced_editing::GeneratedPaintPartitionProposal,
    approval: &crate::advanced_editing::GeneratedPaintPartitionApproval,
    font_bytes: Option<&[u8]>,
    options: &ScopedPreviewOptions,
) -> Result<PaintPartitionCandidatePreview> {
    crate::cancel::check_current_cancel("paint partition preview input")?;
    // Reject unreasonable caller options before the candidate edit or raster
    // allocation. The real page count is checked after the candidate reopens.
    selected_pages(options, request.page, usize::MAX)?;
    let request_bytes = serde_json::to_vec(request).map_err(json_error)?;
    let approval_bytes = serde_json::to_vec(approval).map_err(json_error)?;
    let (candidate, edit_report) =
        crate::advanced_editing::apply_generated_paint_partition_proposal(
            input, request, proposal, approval, font_bytes,
        )?;
    let candidate_output_sha256 =
        digest_hex_cancellable(&candidate, "paint partition preview candidate hash")?;
    if edit_report.output_sha256 != candidate_output_sha256 {
        return Err(invalid(
            "paint-partition candidate differs from its edit receipt",
        ));
    }
    let rendered =
        render_candidate_pages(input, &candidate, request.page, &[request.page], options)?;
    let input_sha256 = digest_hex_cancellable(input, "paint partition preview input hash")?;
    let request_sha256 =
        digest_hex_cancellable(&request_bytes, "paint partition preview request hash")?;
    let approval_sha256 =
        digest_hex_cancellable(&approval_bytes, "paint partition preview approval hash")?;
    let font_sha256 = font_bytes
        .map(|font| digest_hex_cancellable(font, "paint partition preview font hash"))
        .transpose()?;
    let preview_evidence = serde_json::to_vec(&(
        "advanced_editing.paint-partition-preview-evidence.v1",
        options,
        rendered
            .pages
            .iter()
            .map(|page| {
                (
                    page.page,
                    page.width,
                    page.height,
                    &page.before.png_sha256,
                    &page.before.rgba_sha256,
                    &page.candidate.png_sha256,
                    &page.candidate.rgba_sha256,
                    page.difference.channel_tolerance,
                    page.difference.changed_pixels,
                    page.difference.maximum_channel_delta,
                    page.difference.bounds,
                )
            })
            .collect::<Vec<_>>(),
    ))
    .map_err(json_error)?;
    let preview_evidence_sha256 =
        digest_hex_cancellable(&preview_evidence, "paint partition preview evidence hash")?;
    let receipt_id = paint_partition_receipt_id(
        &proposal.proposal_id,
        &input_sha256,
        &request_sha256,
        &approval_sha256,
        font_sha256.as_deref(),
        &candidate_output_sha256,
        &preview_evidence_sha256,
    )?;
    let publication_receipt = PaintPartitionPublicationReceipt {
        schema_version: "advanced_editing.paint-partition-publication-receipt.v1".into(),
        proposal_id: proposal.proposal_id.clone(),
        input_sha256: input_sha256.clone(),
        request_sha256: request_sha256.clone(),
        approval_sha256: approval_sha256.clone(),
        font_sha256: font_sha256.clone(),
        candidate_output_sha256: candidate_output_sha256.clone(),
        preview_evidence_sha256,
        receipt_id,
    };
    Ok(PaintPartitionCandidatePreview {
        schema_version: "advanced_editing.paint-partition-preview.v1",
        proposal_id: proposal.proposal_id.clone(),
        revision_id: input_sha256,
        request_sha256,
        approval_sha256,
        font_sha256,
        candidate_output_sha256,
        options: options.clone(),
        pages: rendered.pages,
        total_pixels: rendered.total_pixels,
        affected_pages_not_previewed: rendered.affected_pages_not_previewed,
        publication_receipt,
        edit_report,
        limitations: vec![
            "The candidate is rendered by the same native renderer; this is not an independent fidelity qualification.",
            "Preview never publishes candidate PDF bytes and does not authorize the later apply operation.",
            "Only requested pages and the default visible optional-content state are shown.",
            "Apply must recompute the exact candidate and compare its output digest with this reviewed preview.",
        ],
    })
}

fn paint_partition_receipt_id(
    proposal_id: &str,
    input_sha256: &str,
    request_sha256: &str,
    approval_sha256: &str,
    font_sha256: Option<&str>,
    candidate_output_sha256: &str,
    preview_evidence_sha256: &str,
) -> Result<String> {
    let payload = serde_json::to_vec(&(
        "advanced_editing.paint-partition-publication-receipt.v1",
        proposal_id,
        input_sha256,
        request_sha256,
        approval_sha256,
        font_sha256,
        candidate_output_sha256,
        preview_evidence_sha256,
    ))
    .map_err(json_error)?;
    digest_hex_cancellable(&payload, "paint partition publication receipt hash")
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_publication_receipt_shape(receipt: &PaintPartitionPublicationReceipt) -> Result<()> {
    if receipt.schema_version != "advanced_editing.paint-partition-publication-receipt.v1"
        || receipt.proposal_id.is_empty()
        || !is_lower_sha256(&receipt.input_sha256)
        || !is_lower_sha256(&receipt.request_sha256)
        || !is_lower_sha256(&receipt.approval_sha256)
        || receipt
            .font_sha256
            .as_deref()
            .is_some_and(|digest| !is_lower_sha256(digest))
        || !is_lower_sha256(&receipt.candidate_output_sha256)
        || !is_lower_sha256(&receipt.preview_evidence_sha256)
        || !is_lower_sha256(&receipt.receipt_id)
    {
        return Err(invalid(
            "publication receipt has an unsupported schema or malformed identity",
        ));
    }
    let expected = paint_partition_receipt_id(
        &receipt.proposal_id,
        &receipt.input_sha256,
        &receipt.request_sha256,
        &receipt.approval_sha256,
        receipt.font_sha256.as_deref(),
        &receipt.candidate_output_sha256,
        &receipt.preview_evidence_sha256,
    )?;
    if receipt.receipt_id != expected {
        return Err(invalid("publication receipt identity is inconsistent"));
    }
    Ok(())
}

fn validate_receipt_auth_metadata(
    key_id: &str,
    audience: &str,
    issued_at_unix: u64,
    expires_at_unix: u64,
    key: &[u8],
) -> Result<()> {
    if !(MIN_RECEIPT_AUTH_KEY_BYTES..=MAX_RECEIPT_AUTH_KEY_BYTES).contains(&key.len()) {
        return Err(invalid(
            "receipt authentication key must contain 32..=256 bytes",
        ));
    }
    if key_id.is_empty()
        || key_id.len() > 128
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid(
            "receipt authentication key_id must be 1..=128 safe ASCII characters",
        ));
    }
    if audience.is_empty()
        || audience.len() > 256
        || audience.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(invalid(
            "receipt authentication audience must be 1..=256 non-control bytes",
        ));
    }
    let lifetime = expires_at_unix
        .checked_sub(issued_at_unix)
        .ok_or_else(|| invalid("receipt authentication expiry must be later than issuance"))?;
    if lifetime == 0 || lifetime > MAX_RECEIPT_AUTH_LIFETIME_SECS {
        return Err(invalid(
            "receipt authentication lifetime must be 1..=604800 seconds",
        ));
    }
    Ok(())
}

fn receipt_auth_payload(
    key_id: &str,
    audience: &str,
    issued_at_unix: u64,
    expires_at_unix: u64,
    receipt: &PaintPartitionPublicationReceipt,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&(
        "advanced_editing.paint-partition-authenticated-publication-receipt.v1",
        key_id,
        audience,
        issued_at_unix,
        expires_at_unix,
        receipt,
    ))
    .map_err(json_error)
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn decode_hmac_sha256(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(invalid(
            "authenticated receipt HMAC must be 64 lowercase hexadecimal characters",
        ));
    }
    let mut decoded = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte: u8| -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            }
        };
        decoded[index] = nibble(pair[0])
            .and_then(|high| nibble(pair[1]).map(|low| (high << 4) | low))
            .ok_or_else(|| {
                invalid("authenticated receipt HMAC must be 64 lowercase hexadecimal characters")
            })?;
    }
    Ok(decoded)
}

/// Authenticate a content-bound preview receipt with an application-held
/// HMAC-SHA-256 key. This is the host authorization layer that the plain
/// receipt intentionally does not provide.
pub fn authenticate_paint_partition_publication_receipt(
    receipt: PaintPartitionPublicationReceipt,
    key_id: &str,
    audience: &str,
    issued_at_unix: u64,
    expires_at_unix: u64,
    key: &[u8],
) -> Result<AuthenticatedPaintPartitionPublicationReceipt> {
    crate::cancel::check_current_cancel("paint partition receipt authentication")?;
    validate_publication_receipt_shape(&receipt)?;
    validate_receipt_auth_metadata(key_id, audience, issued_at_unix, expires_at_unix, key)?;
    let payload =
        receipt_auth_payload(key_id, audience, issued_at_unix, expires_at_unix, &receipt)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| invalid("could not initialize receipt authentication HMAC"))?;
    mac.update(&payload);
    let hmac_sha256 = encode_hex(&mac.finalize().into_bytes());
    Ok(AuthenticatedPaintPartitionPublicationReceipt {
        schema_version: "advanced_editing.paint-partition-authenticated-publication-receipt.v1"
            .into(),
        key_id: key_id.into(),
        audience: audience.into(),
        issued_at_unix,
        expires_at_unix,
        publication_receipt: receipt,
        hmac_sha256,
    })
}

/// Verify host authenticity, audience, lifetime and key identity before
/// returning the nested content-bound receipt. A small caller-supplied clock
/// skew allowance applies only to issuance, never to expiry.
pub fn verify_authenticated_paint_partition_publication_receipt(
    authenticated: &AuthenticatedPaintPartitionPublicationReceipt,
    expected_key_id: &str,
    expected_audience: &str,
    now_unix: u64,
    allowed_future_skew_secs: u64,
    key: &[u8],
) -> Result<PaintPartitionPublicationReceipt> {
    crate::cancel::check_current_cancel("paint partition receipt verification")?;
    validate_receipt_auth_metadata(
        &authenticated.key_id,
        &authenticated.audience,
        authenticated.issued_at_unix,
        authenticated.expires_at_unix,
        key,
    )?;
    let payload = receipt_auth_payload(
        &authenticated.key_id,
        &authenticated.audience,
        authenticated.issued_at_unix,
        authenticated.expires_at_unix,
        &authenticated.publication_receipt,
    )?;
    let supplied = decode_hmac_sha256(&authenticated.hmac_sha256)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| invalid("could not initialize receipt authentication HMAC"))?;
    mac.update(&payload);
    mac.verify_slice(&supplied)
        .map_err(|_| invalid("authenticated publication receipt HMAC is invalid"))?;
    if authenticated.schema_version
        != "advanced_editing.paint-partition-authenticated-publication-receipt.v1"
        || authenticated.key_id != expected_key_id
        || authenticated.audience != expected_audience
        || authenticated.expires_at_unix < now_unix
        || authenticated.issued_at_unix > now_unix.saturating_add(allowed_future_skew_secs.min(300))
    {
        return Err(invalid(
            "authenticated publication receipt is expired or has the wrong key, audience, or issuance time",
        ));
    }
    validate_publication_receipt_shape(&authenticated.publication_receipt)?;
    Ok(authenticated.publication_receipt.clone())
}

/// Verify that a retained preview receipt authorizes publication of this exact
/// recomputed candidate. The plain receipt is content-bound but unauthenticated;
/// remote hosts can wrap it with
/// [`authenticate_paint_partition_publication_receipt`].
pub fn verify_paint_partition_publication_receipt(
    receipt: &PaintPartitionPublicationReceipt,
    input: &[u8],
    request: &crate::advanced_editing::MultiRunTextRangeRequest,
    proposal_id: &str,
    approval: &crate::advanced_editing::GeneratedPaintPartitionApproval,
    font_bytes: Option<&[u8]>,
    candidate_output_sha256: &str,
) -> Result<()> {
    validate_publication_receipt_shape(receipt)?;
    let request_bytes = serde_json::to_vec(request).map_err(json_error)?;
    let approval_bytes = serde_json::to_vec(approval).map_err(json_error)?;
    let input_sha256 = digest_hex_cancellable(input, "reviewed apply input hash")?;
    let request_sha256 = digest_hex_cancellable(&request_bytes, "reviewed apply request hash")?;
    let approval_sha256 = digest_hex_cancellable(&approval_bytes, "reviewed apply approval hash")?;
    let font_sha256 = font_bytes
        .map(|font| digest_hex_cancellable(font, "reviewed apply font hash"))
        .transpose()?;
    let receipt_id = paint_partition_receipt_id(
        proposal_id,
        &input_sha256,
        &request_sha256,
        &approval_sha256,
        font_sha256.as_deref(),
        candidate_output_sha256,
        &receipt.preview_evidence_sha256,
    )?;
    if receipt.schema_version != "advanced_editing.paint-partition-publication-receipt.v1"
        || receipt.proposal_id != proposal_id
        || receipt.input_sha256 != input_sha256
        || receipt.request_sha256 != request_sha256
        || receipt.approval_sha256 != approval_sha256
        || receipt.font_sha256 != font_sha256
        || receipt.candidate_output_sha256 != candidate_output_sha256
        || receipt.receipt_id != receipt_id
    {
        return Err(invalid(
            "publication receipt is stale, altered, or belongs to different reviewed bytes",
        ));
    }
    Ok(())
}

fn render(
    engine: &ContentEngine,
    contract: &RenderContract,
    cancel: &crate::cancel::CancelToken,
) -> Result<(PixelBuffer, Value)> {
    let (pixels, fonts, telemetry) =
        engine.render_page_with_contract_and_telemetry_report(contract, cancel)?;
    if pixels.width != contract.width || pixels.height != contract.height {
        return Err(invalid(
            "renderer returned dimensions different from the contract",
        ));
    }
    Ok((
        pixels,
        json!({"contract":contract,"font_substitutions":fonts,"telemetry":telemetry,
        "independent_renderer":false,"visual_fidelity_certified":false}),
    ))
}

fn encode(
    pixels: &PixelBuffer,
    diagnostics: Value,
    total: &mut usize,
) -> Result<ScopedPreviewRaster> {
    crate::cancel::check_current_cancel("scoped preview PNG encoding")?;
    let png = ImageEncoder::encode_png_fast(&pixels.to_raw_image())?;
    *total = total
        .checked_add(png.len())
        .ok_or_else(|| invalid("PNG size overflow"))?;
    if *total > MAX_PNG_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "scoped preview combined PNG budget exceeded".into(),
        ));
    }
    Ok(ScopedPreviewRaster {
        png_sha256: digest_hex_cancellable(&png, "scoped preview PNG hash")?,
        png,
        rgba_sha256: digest_hex_cancellable(pixels.rgba_bytes(), "scoped preview raster hash")?,
        diagnostics,
    })
}

fn compare(
    width: u32,
    height: u32,
    a: &[u8],
    b: &[u8],
    tolerance: u8,
) -> Result<ScopedPixelDifference> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| invalid("RGBA dimensions overflow"))?;
    if width == 0 || height == 0 || a.len() as u64 != expected || b.len() != a.len() {
        return Err(invalid("invalid RGBA comparison dimensions"));
    }
    let mut changed = 0;
    let mut maximum = 0;
    let mut bounds = [width, height, 0, 0];
    for (i, (left, right)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if i % 65536 == 0 {
            crate::cancel::check_current_cancel("scoped preview difference")?;
        }
        let delta = left
            .iter()
            .zip(right)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        maximum = maximum.max(delta);
        if delta > tolerance {
            changed += 1;
            let x = (i as u64 % u64::from(width)) as u32;
            let y = (i as u64 / u64::from(width)) as u32;
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x + 1),
                bounds[3].max(y + 1),
            ];
        }
    }
    Ok(ScopedPixelDifference {
        channel_tolerance: tolerance,
        changed_pixels: changed,
        maximum_channel_delta: maximum,
        bounds: (changed > 0).then_some(bounds),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn difference_counts_pixels_not_channels_and_has_exclusive_bounds() {
        let mut after = vec![255; 24];
        after[4] = 10;
        after[23] = 251;
        let result = compare(3, 2, &[255; 24], &after, 3).unwrap();
        assert_eq!(result.changed_pixels, 2);
        assert_eq!(result.bounds, Some([1, 0, 3, 2]));
        assert_eq!(result.maximum_channel_delta, 245);
        assert_eq!(compare(3, 2, &after, &after, 0).unwrap().bounds, None);
        assert_eq!(
            compare(3, 2, &[255; 24], &after, 4).unwrap().changed_pixels,
            1
        );
    }
    #[test]
    fn invalid_dimensions_and_cancel_refuse() {
        assert!(compare(1, 1, &[0; 3], &[0; 3], 0).is_err());
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert!(cancel.scope(|| compare(1, 1, &[0; 4], &[0; 4], 0)).is_err());
    }
    #[test]
    fn page_selection_is_bounded_and_defaults_to_occurrence() {
        let mut options = ScopedPreviewOptions::default();
        assert_eq!(selected_pages(&options, 2, 4).unwrap(), vec![2]);
        options.pages = vec![2, 2];
        assert!(selected_pages(&options, 2, 4).is_err());
        options.pages = vec![0];
        assert!(selected_pages(&options, 2, 4).is_err());
        options.pages = vec![5];
        assert!(selected_pages(&options, 2, 4).is_err());
        options.pages = vec![1];
        options.max_total_pixels = MAX_PIXELS + 1;
        assert!(selected_pages(&options, 2, 4).is_err());
    }

    #[test]
    fn publication_receipt_binds_every_reviewed_identity() {
        let base = paint_partition_receipt_id(
            "proposal",
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            Some(&"4".repeat(64)),
            &"5".repeat(64),
            &"6".repeat(64),
        )
        .unwrap();
        let variants = [
            paint_partition_receipt_id(
                "other",
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
                Some(&"4".repeat(64)),
                &"5".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"0".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
                Some(&"4".repeat(64)),
                &"5".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"1".repeat(64),
                &"0".repeat(64),
                &"3".repeat(64),
                Some(&"4".repeat(64)),
                &"5".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"1".repeat(64),
                &"2".repeat(64),
                &"0".repeat(64),
                Some(&"4".repeat(64)),
                &"5".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
                None,
                &"5".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
                Some(&"4".repeat(64)),
                &"0".repeat(64),
                &"6".repeat(64),
            )
            .unwrap(),
            paint_partition_receipt_id(
                "proposal",
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
                Some(&"4".repeat(64)),
                &"5".repeat(64),
                &"0".repeat(64),
            )
            .unwrap(),
        ];
        assert!(variants.into_iter().all(|candidate| candidate != base));
    }

    fn sample_publication_receipt() -> PaintPartitionPublicationReceipt {
        let mut receipt = PaintPartitionPublicationReceipt {
            schema_version: "advanced_editing.paint-partition-publication-receipt.v1".into(),
            proposal_id: "proposal".into(),
            input_sha256: "1".repeat(64),
            request_sha256: "2".repeat(64),
            approval_sha256: "3".repeat(64),
            font_sha256: Some("4".repeat(64)),
            candidate_output_sha256: "5".repeat(64),
            preview_evidence_sha256: "6".repeat(64),
            receipt_id: String::new(),
        };
        receipt.receipt_id = paint_partition_receipt_id(
            &receipt.proposal_id,
            &receipt.input_sha256,
            &receipt.request_sha256,
            &receipt.approval_sha256,
            receipt.font_sha256.as_deref(),
            &receipt.candidate_output_sha256,
            &receipt.preview_evidence_sha256,
        )
        .unwrap();
        receipt
    }

    #[test]
    fn authenticated_receipt_binds_host_metadata_and_expires() {
        let key = [0x5au8; 32];
        let signed = authenticate_paint_partition_publication_receipt(
            sample_publication_receipt(),
            "key-2026-09",
            "tenant/editor",
            1_000,
            1_900,
            &key,
        )
        .unwrap();
        assert_eq!(
            verify_authenticated_paint_partition_publication_receipt(
                &signed,
                "key-2026-09",
                "tenant/editor",
                1_500,
                0,
                &key,
            )
            .unwrap()
            .proposal_id,
            "proposal"
        );
        assert!(verify_authenticated_paint_partition_publication_receipt(
            &signed,
            "wrong-key",
            "tenant/editor",
            1_500,
            0,
            &key,
        )
        .is_err());
        assert!(verify_authenticated_paint_partition_publication_receipt(
            &signed,
            "key-2026-09",
            "other-audience",
            1_500,
            0,
            &key,
        )
        .is_err());
        assert!(verify_authenticated_paint_partition_publication_receipt(
            &signed,
            "key-2026-09",
            "tenant/editor",
            1_901,
            0,
            &key,
        )
        .is_err());
    }

    #[test]
    fn authenticated_receipt_rejects_tampering_and_short_keys() {
        let key = [0xa5u8; 32];
        let mut malformed = sample_publication_receipt();
        malformed.receipt_id = "0".repeat(64);
        assert!(authenticate_paint_partition_publication_receipt(
            malformed,
            "primary",
            "wellfriendpdf-server",
            10,
            20,
            &key,
        )
        .is_err());
        let mut signed = authenticate_paint_partition_publication_receipt(
            sample_publication_receipt(),
            "primary",
            "wellfriendpdf-server",
            10,
            20,
            &key,
        )
        .unwrap();
        signed.publication_receipt.candidate_output_sha256 = "0".repeat(64);
        assert!(verify_authenticated_paint_partition_publication_receipt(
            &signed,
            "primary",
            "wellfriendpdf-server",
            15,
            0,
            &key,
        )
        .is_err());
        assert!(authenticate_paint_partition_publication_receipt(
            sample_publication_receipt(),
            "primary",
            "wellfriendpdf-server",
            10,
            20,
            &[0u8; 16],
        )
        .is_err());
    }
}
