param(
    [switch]$ContinueOnFailure
)

$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$OutDir = Join-Path $Repo "target/prompt03-packaging-codec-isolation"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$Steps = New-Object System.Collections.Generic.List[object]
$Artifacts = New-Object System.Collections.Generic.List[object]
$HadFailure = $false

function Write-JsonNoBom {
    param([string]$Path, [object]$Payload, [int]$Depth = 16)
    $json = $Payload | ConvertTo-Json -Depth $Depth
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $Utf8NoBom)
}

function Add-Step {
    param(
        [string]$Name,
        [string]$Command,
        [string]$Status,
        [int]$ExitCode = 0,
        [string]$Log = "",
        [string]$Reason = ""
    )
    $Steps.Add([ordered]@{
        name = $Name
        command = $Command
        status = $Status
        exit_code = $ExitCode
        log = $Log
        reason = $Reason
    }) | Out-Null
}

function Invoke-GateStep {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$Arguments,
        [bool]$Required = $true
    )
    $cmd = "$FilePath $($Arguments -join ' ')"
    $command = Get-Command $FilePath -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        Add-Step $Name $cmd "unavailable" 127 "" "$FilePath is not on PATH"
        if ($Required) {
            $script:HadFailure = $true
        }
        return
    }

    Write-Host "==> $Name"
    $logPath = Join-Path $OutDir (($Name -replace "[^A-Za-z0-9_.-]", "_") + ".log")
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $FilePath @Arguments 2>&1
        $exit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    [System.IO.File]::WriteAllText($logPath, (($output | Out-String) + [Environment]::NewLine), $Utf8NoBom)
    if ($exit -eq 0) {
        Add-Step $Name $cmd "passed" 0 $logPath ""
    } else {
        Add-Step $Name $cmd "failed" $exit $logPath "command returned non-zero"
        if ($Required) {
            $script:HadFailure = $true
        }
    }
}

function Add-Artifact {
    param([string]$Name, [string]$Path, [string]$Surface)
    $exists = Test-Path -LiteralPath $Path
    $entry = [ordered]@{
        name = $Name
        surface = $Surface
        path = $Path
        exists = $exists
    }
    if ($exists) {
        $file = Get-Item -LiteralPath $Path
        $entry["bytes"] = $file.Length
        if (!$file.PSIsContainer) {
            $entry["sha256"] = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
        }
    }
    $Artifacts.Add($entry) | Out-Null
}

function Get-NativeLibraryName {
    $runtime = [System.Runtime.InteropServices.RuntimeInformation]
    if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        return "oxide_capi.dll"
    }
    if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        return "liboxide_capi.dylib"
    }
    return "liboxide_capi.so"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$head = (& git rev-parse --short HEAD).Trim()
$status = @(& git status --short)
$nativeName = Get-NativeLibraryName
$nativePath = Join-Path $Repo "target/debug/$nativeName"
$runtime = [System.Runtime.InteropServices.RuntimeInformation]
$exeSuffix = if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) { ".exe" } else { "" }
$worker = Join-Path $Repo "target/debug/oxide-codec-worker$exeSuffix"
$oxide = Join-Path $Repo "target/debug/oxide$exeSuffix"

Invoke-GateStep "cargo fmt check" "cargo" @("fmt", "--check")
Invoke-GateStep "cargo package oxide-engine" "cargo" @("package", "-p", "oxide-engine", "--allow-dirty")
Invoke-GateStep "cargo build cli capi" "cargo" @("build", "-p", "oxide-cli", "-p", "oxide-capi")
Invoke-GateStep "cargo build codec worker" "cargo" @("build", "-p", "oxide-engine", "--bin", "oxide-codec-worker")
Invoke-GateStep "rust example codec isolation" "cargo" @("run", "-p", "oxide-engine", "--example", "prompt03_codec_isolation", "--", "in_process")
Invoke-GateStep "cli codec isolation in-process" $oxide @("codec-isolation-report", "--filter", "FlateDecode", "--sample-text", "hello oxide", "--policy", "in_process")
Invoke-GateStep "cli codec isolation isolated" $oxide @("codec-isolation-report", "--filter", "FlateDecode", "--sample-text", "hello oxide", "--policy", "isolated_required", "--worker", $worker)
Invoke-GateStep "capi tests" "cargo" @("test", "-p", "oxide-capi")
Invoke-GateStep "engine isolation tests" "cargo" @("test", "-p", "oxide-engine", "--test", "codec_isolation")
Invoke-GateStep "wasm target check" "cargo" @("check", "-p", "oxide-wasm", "--target", "wasm32-unknown-unknown")

if (Get-Command python -ErrorAction SilentlyContinue) {
    $maturinVersion = & python -m maturin --version 2>$null
    if ($LASTEXITCODE -eq 0) {
        Invoke-GateStep "python wheel build" "python" @("-m", "maturin", "build", "--manifest-path", "crates/oxide-py/Cargo.toml", "--out", $OutDir)
    } else {
        Add-Step "python wheel build" "python -m maturin build" "unavailable" 127 "" "maturin is not installed"
    }
} else {
    Add-Step "python wheel build" "python -m maturin build" "unavailable" 127 "" "python is not on PATH"
}

if (Get-Command dotnet -ErrorAction SilentlyContinue) {
    $env:OXIDE_NATIVE_LIBRARY = $nativePath
    Invoke-GateStep "dotnet test" "dotnet" @("test", "bindings/dotnet/Oxide.Sdk.Tests/Oxide.Sdk.Tests.csproj")
    Invoke-GateStep "dotnet pack" "dotnet" @("pack", "bindings/dotnet/Oxide.Sdk/Oxide.Sdk.csproj", "-o", $OutDir)
} else {
    Add-Step "dotnet test" "dotnet test bindings/dotnet/Oxide.Sdk.Tests" "unavailable" 127 "" "dotnet is not on PATH"
    Add-Step "dotnet pack" "dotnet pack bindings/dotnet/Oxide.Sdk" "unavailable" 127 "" "dotnet is not on PATH"
}

if ((Get-Command java -ErrorAction SilentlyContinue) -and (Get-Command javac -ErrorAction SilentlyContinue)) {
    Invoke-GateStep "java maven package smoke" "powershell" @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/prompt02b_java_package_smoke.ps1") $false
    Invoke-GateStep "java gradle package smoke" "powershell" @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/prompt02c_gradle_package_smoke.ps1") $false
} else {
    Add-Step "java maven package smoke" "scripts/prompt02b_java_package_smoke.ps1" "unavailable" 127 "" "java or javac is not on PATH"
    Add-Step "java gradle package smoke" "scripts/prompt02c_gradle_package_smoke.ps1" "unavailable" 127 "" "java or javac is not on PATH"
}

if (Get-Command wasm-pack -ErrorAction SilentlyContinue) {
    Invoke-GateStep "wasm-pack package" "wasm-pack" @("build", "crates/oxide-wasm", "--target", "web", "--out-dir", "examples/browser/pkg") $false
} else {
    Add-Step "wasm-pack package" "wasm-pack build crates/oxide-wasm --target web" "unavailable" 127 "" "wasm-pack is not on PATH"
}

Add-Artifact "oxide cli debug binary" $oxide "cli"
Add-Artifact "codec worker debug binary" $worker "codec_worker"
Add-Artifact "c abi native library" $nativePath "c_abi"
Add-Artifact "c abi header" (Join-Path $Repo "crates/oxide-capi/include/oxide.h") "c_abi"
Add-Artifact "python codec isolation example" (Join-Path $Repo "crates/oxide-py/examples/codec_isolation_report.py") "python"
Add-Artifact "rust codec isolation example" (Join-Path $Repo "crates/engine/examples/prompt03_codec_isolation.rs") "rust"
Add-Artifact "wasm codec isolation example" (Join-Path $Repo "crates/oxide-wasm/examples/browser/codec_isolation_report.mjs") "wasm"
Add-Artifact "dotnet codec isolation example" (Join-Path $Repo "bindings/dotnet/examples/Prompt03CodecIsolation.cs") "dotnet"
Add-Artifact "java codec isolation example" (Join-Path $Repo "bindings/java/examples/Prompt03CodecIsolation.java") "java"

$examplesMatrix = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt03"
    surfaces = @(
        @{ surface = "rust"; example = "crates/engine/examples/sdk_reports.rs"; codec_isolation = "crates/engine/examples/prompt03_codec_isolation.rs"; package_command = "cargo package -p oxide-engine --allow-dirty" },
        @{ surface = "cli"; example = "examples/cli/codec_isolation_report.ps1"; command = "oxide codec-isolation-report --filter FlateDecode --sample-text 'hello oxide' --policy in_process" },
        @{ surface = "python"; example = "crates/oxide-py/examples/sdk_reports.py"; codec_isolation = "crates/oxide-py/examples/codec_isolation_report.py"; package_command = "python -m maturin build --manifest-path crates/oxide-py/Cargo.toml" },
        @{ surface = "c_abi"; example = "crates/oxide-capi/examples/sdk_reports.c"; codec_isolation = "crates/oxide-capi/examples/codec_isolation_report.c"; package_command = "cargo build -p oxide-capi" },
        @{ surface = "wasm"; example = "crates/oxide-wasm/examples/browser"; codec_isolation = "crates/oxide-wasm/examples/browser/codec_isolation_report.mjs"; package_command = "wasm-pack build crates/oxide-wasm --target web" },
        @{ surface = "dotnet"; example = "bindings/dotnet/examples/Prompt02Reports.cs"; codec_isolation = "bindings/dotnet/examples/Prompt03CodecIsolation.cs"; package_command = "dotnet pack bindings/dotnet/Oxide.Sdk/Oxide.Sdk.csproj" },
        @{ surface = "java_maven"; example = "bindings/java/examples/Prompt02Reports.java"; codec_isolation = "bindings/java/examples/Prompt03CodecIsolation.java"; package_command = "scripts/prompt02b_java_package_smoke.ps1" },
        @{ surface = "java_gradle"; example = "bindings/java/examples/Prompt02Reports.java"; codec_isolation = "bindings/java/examples/Prompt03CodecIsolation.java"; package_command = "scripts/prompt02c_gradle_package_smoke.ps1" }
    )
    workflows = @(
        "open from file", "open from bytes", "password open", "page count", "page boxes",
        "document status", "repair diagnostics", "plain text", "spans", "reading order",
        "semantic blocks", "tables", "rag chunks", "json export", "render page",
        "dpi option", "forms", "annotations", "redaction", "sanitizer", "docx export",
        "pptx export", "xlsx export", "html export", "markdown export", "security report",
        "signature report", "pdfa validation", "pdfua validation", "pdfx validation",
        "canonicalize output", "package build", "package install smoke", "codec isolation"
    )
}
Write-JsonNoBom (Join-Path $OutDir "examples-matrix.json") $examplesMatrix

$threatMatrix = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt03"
    codec_families = @(
        @{ codec = "FlateDecode and predictors"; worker = "implemented"; primary_risks = @("decompression bomb", "predictor row amplification", "malformed stream") },
        @{ codec = "RunLengthDecode"; worker = "implemented"; primary_risks = @("output amplification", "malformed packet") },
        @{ codec = "ASCIIHexDecode and ASCII85Decode"; worker = "implemented"; primary_risks = @("malformed terminator", "oversized decoded output") },
        @{ codec = "LZWDecode"; worker = "implemented"; primary_risks = @("dictionary growth", "malformed code stream") },
        @{ codec = "DCTDecode"; worker = "reported_unsupported"; primary_risks = @("native-like decoder complexity", "large image allocation") },
        @{ codec = "JPXDecode"; worker = "reported_unsupported"; primary_risks = @("complex codestream parser", "tile memory amplification") },
        @{ codec = "JBIG2Decode"; worker = "reported_unsupported"; primary_risks = @("symbol dictionary hazards", "history of exploited decoders") },
        @{ codec = "CCITTFaxDecode"; worker = "reported_unsupported"; primary_risks = @("bitstream parser bugs", "dimension mismatch") }
    )
    policies = @("in_process", "isolated_preferred", "isolated_required", "report_only", "disabled")
}
Write-JsonNoBom (Join-Path $OutDir "codec-threat-model-matrix.json") $threatMatrix

$manifest = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt03"
    head = $head
    dirty_entries = $status
    generated_at_utc = [DateTime]::UtcNow.ToString("o")
    result = if ($HadFailure) { "failed" } else { "passed_or_unavailable_optional" }
    steps = $Steps
    artifacts = $Artifacts
    docs = @(
        "docs/combined_prompt03_audit.md",
        "docs/multi_language_examples_prompt03.md",
        "docs/binding_release_packaging_gate_prompt03.md",
        "docs/package_artifact_manifest_prompt03.md",
        "docs/codec_threat_model_prompt03.md",
        "docs/codec_isolation_design_prompt03.md",
        "docs/codec_isolation_user_guide_prompt03.md",
        "docs/security_policy.md"
    )
}
Write-JsonNoBom (Join-Path $OutDir "release-manifest.json") $manifest

$smoke = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt03"
    focused_tests = @(
        "cargo test -p oxide-engine --test codec_isolation",
        "cargo test -p oxide-capi",
        "oxide codec-isolation-report --policy in_process",
        "oxide codec-isolation-report --policy isolated_required --worker target/debug/oxide-codec-worker"
    )
    steps = @($Steps | Where-Object { $_.name -like "*codec isolation*" -or $_.name -like "*isolation tests*" })
}
Write-JsonNoBom (Join-Path $OutDir "isolation-smoke-report.json") $smoke

$failures = [ordered]@{
    schema_version = 1
    prompt = "combined_prompt03"
    failures = @($Steps | Where-Object { $_.status -eq "failed" })
    unavailable = @($Steps | Where-Object { $_.status -eq "unavailable" })
}
Write-JsonNoBom (Join-Path $OutDir "isolation-failure-report.json") $failures

Write-Host "wrote $OutDir"

if ($HadFailure -and !$ContinueOnFailure) {
    throw "Prompt 03 release gate had required failures; inspect target/prompt03-packaging-codec-isolation/release-manifest.json"
}
