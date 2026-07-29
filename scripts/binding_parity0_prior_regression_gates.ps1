param(
    [switch]$ContinueOnFailure,
    [string]$Only = "",
    [switch]$MergeExisting
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Out = Join-Path $Repo "target/advanced_editing-advanced-editing/prior-gates"
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Utf8 = New-Object System.Text.UTF8Encoding($false)
$Results = New-Object System.Collections.Generic.List[object]
$HadFailure = $false

$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_INCREMENTAL = "0"
$env:RUST_TEST_THREADS = "1"
$env:RAYON_NUM_THREADS = "1"

$Gates = @(
    @{ id = "codec_boundary"; kind = "powershell"; path = "scripts/codec_boundary_codec_boundary_scheduler_reports.ps1" },
    @{ id = "decode_scheduler"; kind = "python"; path = "scripts/decode_scheduler_codec_closeout.py" },
    @{ id = "native_renderer"; kind = "python"; path = "scripts/native_renderer_renderer_parity_audit.py" },
    @{ id = "reference_renderer"; kind = "powershell"; path = "scripts/reference_renderer_multi_reference_audit.ps1" },
    @{ id = "transparency_rendering"; kind = "python"; path = "scripts/transparency_rendering_transparency_compositing_audit.py" },
    @{ id = "transparency_closeout"; kind = "python"; path = "scripts/transparency_closeout_transparency_closure_audit.py" },
    @{ id = "advanced_rendering"; kind = "python"; path = "scripts/advanced_rendering_text_shading_patterns_audit.py" },
    @{ id = "type3_cid_rendering"; kind = "python"; path = "scripts/type3_cid_rendering_type3_cid_tensor_audit.py" },
    @{ id = "annotation_ocg_rendering"; kind = "python"; path = "scripts/annotation_ocg_rendering_annotation_ocg_progressive_cache_audit.py" },
    @{ id = "renderer_validation"; kind = "python"; path = "scripts/renderer_validation_validation_closure_audit.py" },
    @{ id = "multilingual_color_glyphs"; kind = "python"; path = "scripts/multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness.py" },
    @{ id = "cjk_rtl_color_glyph_closeout"; kind = "python"; path = "scripts/cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_closure.py" },
    @{ id = "color_glyph_hinting"; kind = "python"; path = "scripts/color_glyph_hinting_color_glyph_hinting_cff_closure.py" },
    @{ id = "colrv_svg_bitmap"; kind = "python"; path = "scripts/colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure.py" },
    @{ id = "colrv_gradient_composite"; kind = "python"; path = "scripts/colrv_gradient_composite_colrv1_gradient_clip_composite_closure.py" },
    @{ id = "porterduff_radial_color_glyph"; kind = "python"; path = "scripts/porterduff_radial_color_glyph_colrv1_porterduff_radial_closure.py" },
    @{ id = "renderer_fuzz_cmm"; kind = "python"; path = "scripts/renderer_fuzz_cmm_renderer_fuzz_cmm_closeout.py" },
    @{ id = "native_cmm_backend"; kind = "python"; path = "scripts/native_cmm_backend_native_cmm_audit.py" },
    @{ id = "prepress_cmm"; kind = "python"; path = "scripts/prepress_cmm_prepress_cmm_audit.py" },
    @{ id = "nchannel_plate_prepress"; kind = "python"; path = "scripts/nchannel_plate_prepress_prepress_nchannel_plate_closure.py" },
    @{ id = "prepress_proofing"; kind = "python"; path = "scripts/prepress_proofing_prepress_benchmark.py" },
    @{ id = "semantic_intelligence"; kind = "python"; path = "scripts/semantic_intelligence_semantic_intelligence_audit.py" },
    @{ id = "cjk_dictionary_layout"; kind = "python"; path = "scripts/cjk_dictionary_layout_cjk_dictionary_layout_backend_closure.py" },
    @{ id = "semantic_closeout"; kind = "python"; path = "scripts/semantic_closeout_semantic_intelligence_benchmark.py" },
    @{ id = "xfa_runtime"; kind = "python"; path = "scripts/xfa_runtime_xfa_runtime_audit.py" },
    @{ id = "annotation_media_redaction"; kind = "python"; path = "scripts/annotation_media_redaction_interactive_redaction_audit.py" },
    @{ id = "secure_mutation"; kind = "python"; path = "scripts/secure_mutation_secure_mutation_audit.py" },
    @{ id = "secure_mutation_closeout"; kind = "python"; path = "scripts/secure_mutation_closeout_advanced_secure_mutation_audit.py" },
    @{ id = "form_action_policy"; kind = "cargo"; path = "cargo"; args = @("test", "-p", "wellfriendpdf-engine", "--test", "form_action_policy_interactive_docx", "--jobs", "1") }
)

if ($Only -ne "") {
    $Gates = @($Gates | Where-Object { $_.id -eq $Only })
    if ($Gates.Count -eq 0) { throw "unknown gate id: $Only" }
}

foreach ($Gate in $Gates) {
    $started = Get-Date
    $log = Join-Path $Out ($Gate.id + ".log")
    Write-Host "==> $($Gate.id): $($Gate.path)"
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        if ($Gate.kind -eq "powershell") {
            $output = & powershell -NoProfile -ExecutionPolicy Bypass -File $Gate.path 2>&1
        } elseif ($Gate.kind -eq "cargo") {
            $output = & cargo @($Gate.args) 2>&1
        } else {
            $output = & python $Gate.path 2>&1
        }
        $exit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    } finally {
        $ErrorActionPreference = $previous
    }
    [IO.File]::WriteAllText($log, (($output | Out-String) + [Environment]::NewLine), $Utf8)
    $status = if ($exit -eq 0) { "passed" } else { "failed" }
    if ($exit -ne 0) { $HadFailure = $true }
    $Results.Add([ordered]@{
        id = $Gate.id
        command = if ($Gate.kind -eq "powershell") {
            "powershell -File $($Gate.path)"
        } elseif ($Gate.kind -eq "cargo") {
            "cargo $($Gate.args -join ' ')"
        } else {
            "python $($Gate.path)"
        }
        status = $status
        exit_code = $exit
        elapsed_ms = [math]::Round(((Get-Date) - $started).TotalMilliseconds, 3)
        log = $log
    }) | Out-Null
    if ($exit -ne 0 -and -not $ContinueOnFailure) { break }
}

$ResultArray = @($Results.ToArray())
$ManifestPath = Join-Path $Out "advanced_editing-prior-gates.json"
if ($MergeExisting -and (Test-Path -LiteralPath $ManifestPath)) {
    $Existing = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
    $NewIds = @($ResultArray | ForEach-Object { $_.id })
    $Retained = @($Existing.gates | Where-Object { $_.id -notin $NewIds })
    $ResultArray = @($Retained) + @($ResultArray)
}
$Manifest = [ordered]@{
    schema_version = "advanced_editing.prior-regression-gates.v1"
    memory_cap_mib = 4096
    process_tree_cap_enforced_by_parent_job_object = $true
    cargo_build_jobs = 1
    rust_test_threads = 1
    rayon_threads = 1
    result = if ($HadFailure) { "failed" } else { "passed" }
    passed = @($ResultArray | Where-Object { $_.status -eq "passed" }).Count
    failed = @($ResultArray | Where-Object { $_.status -eq "failed" }).Count
    gates = $ResultArray
}
[IO.File]::WriteAllText($ManifestPath, (($Manifest | ConvertTo-Json -Depth 8) + [Environment]::NewLine), $Utf8)
$Manifest | ConvertTo-Json -Depth 8
if ($HadFailure) { exit 1 }
