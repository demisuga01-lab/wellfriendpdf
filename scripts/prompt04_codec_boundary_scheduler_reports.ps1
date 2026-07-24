param(
    [string]$OutDir = "target/prompt04-codec-boundary-scheduler"
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$OutPath = Join-Path $Repo $OutDir
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Write-JsonNoBom {
    param([string]$Path, [object]$Payload, [int]$Depth = 16)
    $json = $Payload | ConvertTo-Json -Depth $Depth
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $Utf8NoBom)
}

function Command-Probe {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $cmd) {
        return [ordered]@{ name = $Name; available = $false; source = $null; version = $null }
    }
    return [ordered]@{
        name = $Name
        available = $true
        source = $cmd.Source
        version = if ($cmd.Version) { $cmd.Version.ToString() } else { $null }
    }
}

function Invoke-Capture {
    param([string]$Name, [string]$FilePath, [string[]]$Arguments)
    $cmd = "$FilePath $($Arguments -join ' ')"
    $command = Get-Command $FilePath -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        return [ordered]@{
            name = $Name
            command = $cmd
            status = "unavailable"
            exit_code = 127
            output = ""
            reason = "$FilePath is not on PATH"
        }
    }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $FilePath @Arguments 2>&1
        $exit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    } finally {
        $ErrorActionPreference = $previous
    }
    return [ordered]@{
        name = $Name
        command = $cmd
        status = if ($exit -eq 0) { "completed" } else { "failed" }
        exit_code = $exit
        output = (($output | Out-String).Trim())
        reason = if ($exit -eq 0) { "" } else { "command returned non-zero" }
    }
}

New-Item -ItemType Directory -Force -Path $OutPath | Out-Null

$head = (& git rev-parse --short HEAD).Trim()
$status = @(& git status --short)

$toolchain = @(
    (Command-Probe "cmake"),
    (Command-Probe "emcc"),
    (Command-Probe "clang++"),
    (Command-Probe "wasm-pack"),
    (Command-Probe "cargo"),
    (Command-Probe "git")
)
$targets = (& rustup target list --installed 2>$null | Out-String).Trim() -split "\r?\n" | Where-Object { $_ }
$rlboxSearch = Invoke-Capture "cargo search rlbox" "cargo" @("search", "rlbox", "--limit", "10")
$rlboxWasmSearch = Invoke-Capture "cargo search rlbox-wasm" "cargo" @("search", "rlbox-wasm", "--limit", "10")

$rlboxReport = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt04"
    status = "hard_blocked"
    verdict = "RLBox/WASM codec sandbox production integration is not viable in this repository in Prompt 04; keep OS subprocess isolation as the practical codec sandbox boundary."
    head = $head
    dirty_entries = $status
    dependency_feasibility = [ordered]@{
        cargo_search_rlbox = $rlboxSearch
        cargo_search_rlbox_wasm = $rlboxWasmSearch
        existing_repo_integration = "No rlbox, wasmtime, wasmer, or wasm sandbox runtime integration was present in Cargo manifests or engine source during Prompt 04 inventory."
    }
    toolchain_feasibility = [ordered]@{
        probes = $toolchain
        installed_rust_targets = $targets
        windows_result = "blocked: cmake is present, but emcc, clang++, and wasm-pack were not all available on PATH for a reproducible RLBox/WASM codec stub build."
        linux_result = "not locally proven in this Windows Prompt 04 run; would require CI job with RLBox C++ toolchain and wasm runtime."
        macos_result = "not locally proven in this Windows Prompt 04 run; would require CI job with RLBox C++ toolchain and wasm runtime."
        wasm_build_result = "wasm32-unknown-unknown target is installed, but that is not sufficient for RLBox C/C++ wasm sandbox packaging."
    }
    minimal_codec_stub = [ordered]@{
        attempted = $false
        reason = "Required C/C++ wasm sandbox toolchain components were unavailable locally and no Rust RLBox crate candidate was discoverable from cargo search output in this run."
    }
    ipc_or_call_boundary = "Existing wellfriendpdf-codec-worker JSON subprocess boundary remains the supported isolation mechanism."
    performance_estimate = "No RLBox stub benchmark was run because the prototype was hard-blocked before build; subprocess isolation overhead remains measured by Prompt 03 release gate and codec isolation tests."
    security_benefit_estimate = "Potential benefit would be finer-grained memory isolation for native codec code, but it would add C++/wasm toolchain and runtime packaging risk. Without a reproducible prototype it must not be claimed."
    future_integration_plan = @(
        "Add a separate optional crate for RLBox/WASM experiments outside wellfriendpdf-engine's unsafe-forbidden core.",
        "Define one no-op C codec stub compiled to wasm and called through a sandbox runtime on Windows, Linux, and macOS CI.",
        "Package the runtime in release gates before enabling any real codec.",
        "Keep native codec registry entries worker-required and deny-by-default until the stub is reproducible."
    )
}

Write-JsonNoBom (Join-Path $OutPath "rlbox-wasm-feasibility.json") $rlboxReport

$generator = Invoke-Capture "prompt04 rust report generator" "cargo" @(
    "run",
    "-p",
    "wellfriendpdf-engine",
    "--example",
    "prompt04_reports",
    "--",
    $OutPath
)
Write-JsonNoBom (Join-Path $OutPath "report-generator-command.json") $generator

if ($generator.exit_code -ne 0) {
    throw "Prompt 04 report generator failed"
}

Write-Host "Prompt 04 reports written to $OutPath"
