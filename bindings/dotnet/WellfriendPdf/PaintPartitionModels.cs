using System.Text.Json;
using System.Text.Json.Serialization;

namespace WellfriendPdf;

public enum PaintPartitionTextMode
{
    SafePatch,
    ParagraphReflowHorizontal,
    ParagraphReflowRtl,
    ParagraphReflowVertical,
    OverlayFallback,
    Unsupported,
}

public enum PaintPartitionStylePolicy
{
    InheritLeading,
    InheritTrailing,
    PreservePerSegment,
    ExplicitSupplied,
}

public enum PaintPartitionOverflowPolicy
{
    Error,
    Clip,
    ExpandRegion,
}

public enum PaintPartitionAlignment
{
    Left,
    Right,
    Center,
    Start,
    End,
    Justify,
}

public enum PaintPartitionOrderPolicy
{
    RequireSingleSourceTextObject,
    AnchorAfterFirstSourceTextObject,
    AnchorAfterLastSourceTextObject,
}

public enum AdvancedEditingSupportStatus
{
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    NotInAdvancedEditingScope,
    Blocked,
}

public sealed record PaintPartitionLineBidi
{
    public required byte[] Levels { get; init; }
    public required bool Rtl { get; init; }
    public Dictionary<string, JsonElement>? Context { get; init; }
}

public sealed record PaintPartitionExplicitLayoutLine
{
    public required string LogicalText { get; init; }
    public required string VisualText { get; init; }
    public PaintPartitionLineBidi? Bidi { get; init; }
    public bool InsertedVisualHyphen { get; init; }
}

public sealed record PaintPartitionPlacement
{
    public required ulong SourceTextObject { get; init; }
    public required ulong[] ReplacementScalarRange { get; init; }
    public required double[] Region { get; init; }
    public IReadOnlyList<PaintPartitionExplicitLayoutLine>? FinalLines { get; init; }

    internal void Validate(string name)
    {
        PaintPartitionJson.RequirePair(ReplacementScalarRange, $"{name}.replacement_scalar_range");
        PaintPartitionJson.RequireRegion(Region, $"{name}.region");
    }
}

public sealed record PaintPartitionTextEditOptions
{
    public double[] Region { get; init; } = [36.0, 36.0, 576.0, 756.0];
    public double FontSize { get; init; } = 12.0;
    public double LineSpacing { get; init; } = 1.2;
    public ulong MaxLinesOrColumns { get; init; } = 4096;
    public PaintPartitionOverflowPolicy OverflowPolicy { get; init; } = PaintPartitionOverflowPolicy.Error;
    public bool SignaturePolicyOverride { get; init; }
    public bool Deterministic { get; init; } = true;
    public PaintPartitionAlignment Alignment { get; init; } = PaintPartitionAlignment.Left;
    public bool JustifyLastLine { get; init; }
    public double MaxWordSpacing { get; init; } = 0.5;
    public double MaxCharacterSpacing { get; init; } = 0.05;
    public uint? TargetStreamObject { get; init; }
    public ushort? TargetStreamGeneration { get; init; }
    public ulong[]? TargetDecodedByteRange { get; init; }
    public PaintPartitionOrderPolicy PaintOrderPolicy { get; init; } = PaintPartitionOrderPolicy.RequireSingleSourceTextObject;
    public IReadOnlyList<PaintPartitionPlacement> PaintPartitions { get; init; } = [];

    internal void Validate()
    {
        PaintPartitionJson.RequireRegion(Region, "options.region");
        if (!double.IsFinite(FontSize) || FontSize <= 0) throw new ArgumentOutOfRangeException(nameof(FontSize));
        if (!double.IsFinite(LineSpacing) || LineSpacing <= 0) throw new ArgumentOutOfRangeException(nameof(LineSpacing));
        if (MaxLinesOrColumns == 0) throw new ArgumentOutOfRangeException(nameof(MaxLinesOrColumns));
        if (!double.IsFinite(MaxWordSpacing) || MaxWordSpacing < 0) throw new ArgumentOutOfRangeException(nameof(MaxWordSpacing));
        if (!double.IsFinite(MaxCharacterSpacing) || MaxCharacterSpacing < 0) throw new ArgumentOutOfRangeException(nameof(MaxCharacterSpacing));
        if (TargetDecodedByteRange is not null) PaintPartitionJson.RequirePair(TargetDecodedByteRange, nameof(TargetDecodedByteRange));
        for (var index = 0; index < PaintPartitions.Count; index++) PaintPartitions[index].Validate($"paint_partitions[{index}]");
    }
}

public sealed record PaintPartitionTextRangeRequest
{
    public required ulong Page { get; init; }
    public required ulong LogicalStart { get; init; }
    public required ulong LogicalEnd { get; init; }
    public required string ReplacementText { get; init; }
    public required PaintPartitionTextMode Mode { get; init; }
    public PaintPartitionStylePolicy StylePolicy { get; init; } = PaintPartitionStylePolicy.InheritLeading;
    public PaintPartitionTextEditOptions? Options { get; init; }
    public IReadOnlyList<PaintPartitionExplicitLayoutLine>? FinalLines { get; init; }

    public string ToJson()
    {
        if (Page == 0) throw new ArgumentOutOfRangeException(nameof(Page));
        if (LogicalStart > LogicalEnd) throw new ArgumentException("LogicalStart must not exceed LogicalEnd.");
        ArgumentNullException.ThrowIfNull(ReplacementText);
        Options?.Validate();
        return PaintPartitionJson.Serialize(this);
    }
}

public sealed record PaintPartitionProposalCandidate
{
    public required ulong SourceTextObject { get; init; }
    public required ulong SelectedSourceScalarCount { get; init; }
    public required ulong[] ReplacementScalarRange { get; init; }
    public required IReadOnlyList<string> SelectedSpanIds { get; init; }
    public double[]? SuggestedRegion { get; init; }
    public required string StartBoundaryClass { get; init; }
    public required string EndBoundaryClass { get; init; }
}

public sealed record PaintPartitionProposal
{
    public required string SchemaVersion { get; init; }
    public required AdvancedEditingSupportStatus Status { get; init; }
    public required string InputSha256 { get; init; }
    public required string RequestSha256 { get; init; }
    public required string ProposalId { get; init; }
    public required ulong Page { get; init; }
    public required ulong[] LogicalRange { get; init; }
    public required string ReplacementSha256 { get; init; }
    public required IReadOnlyList<PaintPartitionProposalCandidate> Candidates { get; init; }
    public required bool Deterministic { get; init; }
    public required IReadOnlyList<string> ExactLimits { get; init; }

    public string ToJson()
    {
        Validate();
        return PaintPartitionJson.Serialize(this);
    }

    internal void Validate()
    {
        if (SchemaVersion != "advanced_editing.paint-partition-proposal.v1")
            throw new ArgumentException("Unsupported paint-partition proposal schema.", nameof(SchemaVersion));
        ArgumentException.ThrowIfNullOrWhiteSpace(ProposalId);
        PaintPartitionJson.RequireDigest(ProposalId, nameof(ProposalId));
        PaintPartitionJson.RequireDigest(InputSha256, nameof(InputSha256));
        PaintPartitionJson.RequireDigest(RequestSha256, nameof(RequestSha256));
        PaintPartitionJson.RequireDigest(ReplacementSha256, nameof(ReplacementSha256));
        PaintPartitionJson.RequirePair(LogicalRange, nameof(LogicalRange));
        if (Page == 0 || Candidates.Count == 0) throw new ArgumentException("Proposal page and candidates must be non-empty.");
        foreach (var candidate in Candidates)
        {
            PaintPartitionJson.RequirePair(candidate.ReplacementScalarRange, nameof(candidate.ReplacementScalarRange));
            if (candidate.SuggestedRegion is not null)
                PaintPartitionJson.RequireRegion(candidate.SuggestedRegion, nameof(candidate.SuggestedRegion));
        }
    }
}

public sealed record PaintPartitionApprovalEntry
{
    public required ulong SourceTextObject { get; init; }
    public required double[] Region { get; init; }
    public IReadOnlyList<PaintPartitionExplicitLayoutLine>? FinalLines { get; init; }
}

public sealed record PaintPartitionApproval
{
    public required string ProposalId { get; init; }
    public string? FontSha256 { get; init; }
    public required IReadOnlyList<PaintPartitionApprovalEntry> Partitions { get; init; }

    public string ToJson()
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(ProposalId);
        if (FontSha256 is not null) PaintPartitionJson.RequireDigest(FontSha256, nameof(FontSha256));
        if (Partitions.Count == 0) throw new ArgumentException("At least one approved partition is required.", nameof(Partitions));
        for (var index = 0; index < Partitions.Count; index++)
        {
            PaintPartitionJson.RequireRegion(Partitions[index].Region, $"partitions[{index}].region");
        }
        return PaintPartitionJson.Serialize(this);
    }
}

public sealed record PaintPartitionPublicationReceipt
{
    public required string SchemaVersion { get; init; }
    public required string ProposalId { get; init; }
    public required string InputSha256 { get; init; }
    public required string RequestSha256 { get; init; }
    public required string ApprovalSha256 { get; init; }
    public string? FontSha256 { get; init; }
    public required string CandidateOutputSha256 { get; init; }
    public required string PreviewEvidenceSha256 { get; init; }
    public required string ReceiptId { get; init; }

    public string ToJson()
    {
        Validate();
        return PaintPartitionJson.Serialize(this);
    }

    internal void Validate()
    {
        if (SchemaVersion != "advanced_editing.paint-partition-publication-receipt.v1")
            throw new ArgumentException("Unsupported publication receipt schema.", nameof(SchemaVersion));
        ArgumentException.ThrowIfNullOrWhiteSpace(ProposalId);
        foreach (var (value, name) in new[]
        {
            (InputSha256, nameof(InputSha256)), (RequestSha256, nameof(RequestSha256)),
            (ApprovalSha256, nameof(ApprovalSha256)), (CandidateOutputSha256, nameof(CandidateOutputSha256)),
            (PreviewEvidenceSha256, nameof(PreviewEvidenceSha256)), (ReceiptId, nameof(ReceiptId)),
        }) PaintPartitionJson.RequireDigest(value, name);
        if (FontSha256 is not null) PaintPartitionJson.RequireDigest(FontSha256, nameof(FontSha256));
    }
}

public sealed record AuthenticatedPaintPartitionPublicationReceipt
{
    public required string SchemaVersion { get; init; }
    public required string KeyId { get; init; }
    public required string Audience { get; init; }
    public required ulong IssuedAtUnix { get; init; }
    public required ulong ExpiresAtUnix { get; init; }
    public required PaintPartitionPublicationReceipt PublicationReceipt { get; init; }
    public required string HmacSha256 { get; init; }

    public string ToJson()
    {
        Validate();
        return PaintPartitionJson.Serialize(this);
    }

    internal void Validate()
    {
        if (SchemaVersion != "advanced_editing.paint-partition-authenticated-publication-receipt.v1")
            throw new ArgumentException("Unsupported authenticated receipt schema.", nameof(SchemaVersion));
        ArgumentException.ThrowIfNullOrWhiteSpace(KeyId);
        ArgumentException.ThrowIfNullOrWhiteSpace(Audience);
        if (ExpiresAtUnix <= IssuedAtUnix) throw new ArgumentOutOfRangeException(nameof(ExpiresAtUnix));
        PaintPartitionJson.RequireDigest(HmacSha256, nameof(HmacSha256));
        PublicationReceipt.Validate();
    }
}

public sealed record PaintPartitionEnvelope<T>
{
    public required uint SchemaVersion { get; init; }
    public required string Kind { get; init; }
    public required T Report { get; init; }
}

public sealed record PaintPartitionPreviewReport
{
    public required PaintPartitionPublicationReceipt PublicationReceipt { get; init; }

    [JsonExtensionData]
    public Dictionary<string, JsonElement>? AdditionalProperties { get; init; }
}

public sealed record PaintPartitionPreviewOptions
{
    public IReadOnlyList<ulong> Pages { get; init; } = [];
    public uint Dpi { get; init; } = 96;
    public bool RequireExact { get; init; }
    public ulong MaxTotalPixels { get; init; } = 16_000_000;
    public byte ChannelTolerance { get; init; }

    public string ToJson()
    {
        if (Pages.Count > 8 || Pages.Any(page => page == 0)) throw new ArgumentOutOfRangeException(nameof(Pages));
        if (Dpi is < 24 or > 600) throw new ArgumentOutOfRangeException(nameof(Dpi));
        if (MaxTotalPixels is 0 or > 32_000_000) throw new ArgumentOutOfRangeException(nameof(MaxTotalPixels));
        return PaintPartitionJson.Serialize(this);
    }
}

internal static class PaintPartitionJson
{
    internal static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DictionaryKeyPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },
    };

    internal static string Serialize<T>(T value) => JsonSerializer.Serialize(value, Options);

    internal static PaintPartitionEnvelope<T> DeserializeEnvelope<T>(string json, string expectedKind)
    {
        var envelope = JsonSerializer.Deserialize<PaintPartitionEnvelope<T>>(json, Options)
            ?? throw new JsonException("Native response did not contain an envelope object.");
        if (envelope.SchemaVersion != 1 || envelope.Kind != expectedKind || envelope.Report is null)
            throw new JsonException($"Native response did not match envelope kind '{expectedKind}'.");
        return envelope;
    }

    internal static void RequireDigest(string value, string name)
    {
        if (value.Length != 64 || value.Any(character => !(character is >= '0' and <= '9' or >= 'a' and <= 'f')))
            throw new ArgumentException("Expected a lowercase SHA-256 hex digest.", name);
    }

    internal static void RequirePair(ulong[] value, string name)
    {
        if (value.Length != 2 || value[0] > value[1]) throw new ArgumentException("Expected an ordered two-value range.", name);
    }

    internal static void RequireRegion(double[] value, string name)
    {
        if (value.Length != 4 || value.Any(coordinate => !double.IsFinite(coordinate))
            || value[2] <= value[0] || value[3] <= value[1])
            throw new ArgumentException("Expected a finite non-empty [x0,y0,x1,y1] region.", name);
    }
}

public sealed partial class WellfriendDocument
{
    public PaintPartitionEnvelope<PaintPartitionProposal> ProposeTextRangePaintPartitions(
        PaintPartitionTextRangeRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        var envelope = PaintPartitionJson.DeserializeEnvelope<PaintPartitionProposal>(
            ProposeTextRangePaintPartitions(request.ToJson()),
            "advanced_editing_closeout_paint_partition_proposal");
        envelope.Report.Validate();
        return envelope;
    }

    public PaintPartitionEnvelope<PaintPartitionPreviewReport> PreviewTextRangePaintPartitions(
        PaintPartitionTextRangeRequest request,
        PaintPartitionProposal proposal,
        PaintPartitionApproval approval,
        byte[]? fontBytes = null,
        PaintPartitionPreviewOptions? options = null)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(proposal);
        ArgumentNullException.ThrowIfNull(approval);
        RequireSameProposal(proposal, approval);
        var envelope = PaintPartitionJson.DeserializeEnvelope<PaintPartitionPreviewReport>(
            PreviewTextRangePaintPartitions(
                request.ToJson(), proposal.ToJson(), approval.ToJson(), fontBytes, options?.ToJson()),
            "advanced_editing_closeout_paint_partition_preview");
        envelope.Report.PublicationReceipt.Validate();
        return envelope;
    }

    public WellfriendBinaryResult ApplyTextRangePaintPartitions(
        PaintPartitionTextRangeRequest request,
        PaintPartitionProposal proposal,
        PaintPartitionApproval approval,
        byte[]? fontBytes = null)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(proposal);
        ArgumentNullException.ThrowIfNull(approval);
        RequireSameProposal(proposal, approval);
        return ApplyTextRangePaintPartitions(request.ToJson(), proposal.ToJson(), approval.ToJson(), fontBytes);
    }

    public WellfriendBinaryResult ApplyReviewedTextRangePaintPartitions(
        PaintPartitionTextRangeRequest request,
        PaintPartitionProposal proposal,
        PaintPartitionApproval approval,
        PaintPartitionPublicationReceipt publicationReceipt,
        byte[]? fontBytes = null)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(proposal);
        ArgumentNullException.ThrowIfNull(approval);
        ArgumentNullException.ThrowIfNull(publicationReceipt);
        RequireSameProposal(proposal, approval);
        if (publicationReceipt.ProposalId != proposal.ProposalId)
            throw new ArgumentException("Publication receipt belongs to a different proposal.", nameof(publicationReceipt));
        return ApplyReviewedTextRangePaintPartitions(
            request.ToJson(), proposal.ToJson(), approval.ToJson(), publicationReceipt.ToJson(), fontBytes);
    }

    public static PaintPartitionEnvelope<AuthenticatedPaintPartitionPublicationReceipt> AuthenticateTextRangePaintPartitionReceipt(
        PaintPartitionPublicationReceipt publicationReceipt,
        string keyId,
        string audience,
        ulong issuedAtUnix,
        ulong expiresAtUnix,
        byte[] hmacKey)
    {
        ArgumentNullException.ThrowIfNull(publicationReceipt);
        var envelope = PaintPartitionJson.DeserializeEnvelope<AuthenticatedPaintPartitionPublicationReceipt>(
            AuthenticateTextRangePaintPartitionReceipt(
                publicationReceipt.ToJson(), keyId, audience, issuedAtUnix, expiresAtUnix, hmacKey),
            "advanced_editing_closeout_authenticated_paint_partition_publication_receipt");
        envelope.Report.Validate();
        return envelope;
    }

    public static PaintPartitionEnvelope<PaintPartitionPublicationReceipt> VerifyAuthenticatedTextRangePaintPartitionReceipt(
        AuthenticatedPaintPartitionPublicationReceipt authenticatedReceipt,
        string expectedKeyId,
        string expectedAudience,
        ulong nowUnix,
        ulong allowedFutureSkewSecs,
        byte[] hmacKey)
    {
        ArgumentNullException.ThrowIfNull(authenticatedReceipt);
        var envelope = PaintPartitionJson.DeserializeEnvelope<PaintPartitionPublicationReceipt>(
            VerifyAuthenticatedTextRangePaintPartitionReceipt(
                authenticatedReceipt.ToJson(), expectedKeyId, expectedAudience, nowUnix,
                allowedFutureSkewSecs, hmacKey),
            "advanced_editing_closeout_verified_paint_partition_publication_receipt");
        envelope.Report.Validate();
        return envelope;
    }

    private static void RequireSameProposal(
        PaintPartitionProposal proposal,
        PaintPartitionApproval approval)
    {
        if (approval.ProposalId != proposal.ProposalId)
            throw new ArgumentException("Approval belongs to a different proposal.", nameof(approval));
    }
}
