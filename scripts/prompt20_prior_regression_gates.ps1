param(
    [switch]$ContinueOnFailure,
    [string]$Only = "",
    [switch]$MergeExisting
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Out = Join-Path $Repo "target/prompt20-advanced-editing/prior-gates"
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Utf8 = New-Object System.Text.UTF8Encoding($false)
$Results = New-Object System.Collections.Generic.List[object]
$HadFailure = $false

$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_INCREMENTAL = "0"
$env:RUST_TEST_THREADS = "1"
$env:RAYON_NUM_THREADS = "1"

$Gates = @(
    @{ id = "prompt04"; kind = "powershell"; path = "scripts/prompt04_codec_boundary_scheduler_reports.ps1" },
    @{ id = "prompt05"; kind = "python"; path = "scripts/prompt05_codec_closeout.py" },
    @{ id = "prompt06"; kind = "python"; path = "scripts/prompt06_renderer_parity_audit.py" },
    @{ id = "prompt06b"; kind = "powershell"; path = "scripts/prompt06b_multi_reference_audit.ps1" },
    @{ id = "prompt07"; kind = "python"; path = "scripts/prompt07_transparency_compositing_audit.py" },
    @{ id = "prompt07b"; kind = "python"; path = "scripts/prompt07b_transparency_closure_audit.py" },
    @{ id = "prompt08"; kind = "python"; path = "scripts/prompt08_text_shading_patterns_audit.py" },
    @{ id = "prompt08b"; kind = "python"; path = "scripts/prompt08b_type3_cid_tensor_audit.py" },
    @{ id = "prompt09"; kind = "python"; path = "scripts/prompt09_annotation_ocg_progressive_cache_audit.py" },
    @{ id = "prompt09b"; kind = "python"; path = "scripts/prompt09b_validation_closure_audit.py" },
    @{ id = "prompt10"; kind = "python"; path = "scripts/prompt10_cjk_rtl_color_glyph_reference_harness.py" },
    @{ id = "prompt10b"; kind = "python"; path = "scripts/prompt10b_color_glyph_cjk_rtl_closure.py" },
    @{ id = "prompt10c"; kind = "python"; path = "scripts/prompt10c_color_glyph_hinting_cff_closure.py" },
    @{ id = "prompt10d"; kind = "python"; path = "scripts/prompt10d_full_colrv1_svg_color_glyph_closure.py" },
    @{ id = "prompt10e"; kind = "python"; path = "scripts/prompt10e_colrv1_gradient_clip_composite_closure.py" },
    @{ id = "prompt10f"; kind = "python"; path = "scripts/prompt10f_colrv1_porterduff_radial_closure.py" },
    @{ id = "prompt11"; kind = "python"; path = "scripts/prompt11_renderer_fuzz_cmm_closeout.py" },
    @{ id = "prompt11b"; kind = "python"; path = "scripts/prompt11b_native_cmm_audit.py" },
    @{ id = "prompt12"; kind = "python"; path = "scripts/prompt12_prepress_cmm_audit.py" },
    @{ id = "prompt12b"; kind = "python"; path = "scripts/prompt12b_prepress_nchannel_plate_closure.py" },
    @{ id = "prompt13"; kind = "python"; path = "scripts/prompt13_prepress_benchmark.py" },
    @{ id = "prompt14"; kind = "python"; path = "scripts/prompt14_semantic_intelligence_audit.py" },
    @{ id = "prompt14b"; kind = "python"; path = "scripts/prompt14b_cjk_dictionary_layout_backend_closure.py" },
    @{ id = "prompt15"; kind = "python"; path = "scripts/prompt15_semantic_intelligence_benchmark.py" },
    @{ id = "prompt16"; kind = "python"; path = "scripts/prompt16_xfa_runtime_audit.py" },
    @{ id = "prompt17"; kind = "python"; path = "scripts/prompt17_interactive_redaction_audit.py" },
    @{ id = "prompt18"; kind = "python"; path = "scripts/prompt18_secure_mutation_audit.py" },
    @{ id = "prompt18b"; kind = "python"; path = "scripts/prompt18b_advanced_secure_mutation_audit.py" },
    @{ id = "prompt19"; kind = "cargo"; path = "cargo"; args = @("test", "-p", "wellfriendpdf-engine", "--test", "prompt19_interactive_docx", "--jobs", "1") }
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
$ManifestPath = Join-Path $Out "prompt20-prior-gates.json"
if ($MergeExisting -and (Test-Path -LiteralPath $ManifestPath)) {
    $Existing = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
    $NewIds = @($ResultArray | ForEach-Object { $_.id })
    $Retained = @($Existing.gates | Where-Object { $_.id -notin $NewIds })
    $ResultArray = @($Retained) + @($ResultArray)
}
$Manifest = [ordered]@{
    schema_version = "prompt20.prior-regression-gates.v1"
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
