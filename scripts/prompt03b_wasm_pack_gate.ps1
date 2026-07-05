param(
    [string]$WasmPackVersion = "0.13.1",
    [string]$OutDir = ""
)

$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ($OutDir -eq "") {
    $OutDir = Join-Path $Repo "target/prompt03-packaging-codec-isolation"
}
$EvidenceDir = Join-Path $OutDir "wasm-pack"
$ToolRoot = Join-Path $Repo "target/prompt03-tools/wasm-pack-$WasmPackVersion"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$Runtime = [System.Runtime.InteropServices.RuntimeInformation]
$ExeSuffix = if ($Runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) { ".exe" } else { "" }
$WasmPack = Join-Path $ToolRoot "bin/wasm-pack$ExeSuffix"
$WebPackage = Join-Path $EvidenceDir "web-pkg"
$NodePackage = Join-Path $EvidenceDir "node-pkg"
$Fixture = Join-Path $Repo "crates/engine/tests/fixtures/minimal.pdf"

function Write-JsonNoBom {
    param([string]$Path, [object]$Payload, [int]$Depth = 16)
    $json = $Payload | ConvertTo-Json -Depth $Depth
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $Utf8NoBom)
}

function Invoke-Logged {
    param([string]$Name, [string]$FilePath, [string[]]$Arguments, [string]$LogPath)
    Write-Host "==> $Name"
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $FilePath @Arguments 2>&1
        $exit = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } else { 0 }
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    [System.IO.File]::WriteAllText($LogPath, (($output | Out-String) + [Environment]::NewLine), $Utf8NoBom)
    if ($exit -ne 0) {
        throw "$Name failed with exit code $exit; see $LogPath"
    }
}

function Clear-TargetDir {
    param([string]$Path)
    $root = (Resolve-Path -LiteralPath $EvidenceDir -ErrorAction SilentlyContinue)
    if ($null -eq $root) {
        New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
        $root = Resolve-Path -LiteralPath $EvidenceDir
    }
    $fullRoot = $root.Path
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $candidate = if (Test-Path -LiteralPath $Path) {
        (Resolve-Path -LiteralPath $Path).Path
    } else {
        [System.IO.Path]::GetFullPath($Path)
    }
    if (!$candidate.StartsWith($fullRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing to remove path outside evidence dir: $candidate"
    }
    Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Get-RelativePath {
    param([string]$Base, [string]$Path)
    $basePath = (Resolve-Path -LiteralPath $Base).Path.TrimEnd("\", "/") + [System.IO.Path]::DirectorySeparatorChar
    $targetPath = (Resolve-Path -LiteralPath $Path).Path
    $baseUri = New-Object System.Uri($basePath)
    $targetUri = New-Object System.Uri($targetPath)
    [System.Uri]::UnescapeDataString($baseUri.MakeRelativeUri($targetUri).ToString()).Replace("\", "/")
}

function Inspect-WasmPackage {
    param([string]$Name, [string]$Path)
    $files = @(Get-ChildItem -LiteralPath $Path -Recurse -File | ForEach-Object {
        Get-RelativePath $Path $_.FullName
    } | Sort-Object)
    $wasm = @($files | Where-Object { $_ -like "*.wasm" })
    $js = @($files | Where-Object { $_ -like "*.js" })
    $dts = @($files | Where-Object { $_ -like "*.d.ts" })
    $expectedMissing = @()
    foreach ($required in @("package.json", "README.md")) {
        if ($files -notcontains $required) {
            $expectedMissing += $required
        }
    }
    if ($wasm.Count -eq 0) { $expectedMissing += "*.wasm" }
    if ($js.Count -eq 0) { $expectedMissing += "*.js" }
    if ($dts.Count -eq 0) { $expectedMissing += "*.d.ts" }

    $forbidden = @($files | Where-Object {
        $_ -match "(^|/)(target|fixtures|tests?)(/|$)" -or
        $_ -match "\.(pdf|pem|key|pfx|dll|so|dylib|pdb|obj|exp)$"
    })

    $absoluteHits = New-Object System.Collections.Generic.List[object]
    foreach ($relative in $files) {
        if ($relative -match "\.(js|json|d\.ts|ts|md)$") {
            $full = Join-Path $Path $relative
            $text = [System.IO.File]::ReadAllText($full)
            if ($text.Contains($Repo) -or $text -match "[A-Za-z]:\\\\") {
                $absoluteHits.Add([ordered]@{ file = $relative }) | Out-Null
            }
        }
    }

    $packageJson = Get-Content -Raw -LiteralPath (Join-Path $Path "package.json") | ConvertFrom-Json
    $payload = [ordered]@{}
    $payload["name"] = $Name
    $payload["package_path"] = $Path
    $payload["package_name"] = $packageJson.name
    $payload["package_version"] = $packageJson.version
    $payload["files"] = $files
    $payload["wasm_files"] = $wasm
    $payload["js_files"] = $js
    $payload["declaration_files"] = $dts
    $payload["expected_files_present"] = ($expectedMissing.Count -eq 0)
    $payload["expected_missing"] = $expectedMissing
    $payload["forbidden_files_absent"] = ($forbidden.Count -eq 0)
    $payload["forbidden_files"] = $forbidden
    $payload["absolute_paths_absent"] = ($absoluteHits.Count -eq 0)
    $payload["absolute_path_hits"] = @($absoluteHits.ToArray())
    $payload["result"] = if ($expectedMissing.Count -eq 0 -and $forbidden.Count -eq 0 -and $absoluteHits.Count -eq 0) { "passed" } else { "failed" }
    $payload
}

New-Item -ItemType Directory -Force -Path $EvidenceDir, $ToolRoot | Out-Null

$bootstrapLog = Join-Path $EvidenceDir "wasm-pack-bootstrap-log.txt"
$installCommand = "cargo install wasm-pack --version $WasmPackVersion --locked --root $ToolRoot"
if (!(Test-Path -LiteralPath $WasmPack)) {
    Invoke-Logged "install wasm-pack $WasmPackVersion" "cargo" @("install", "wasm-pack", "--version", $WasmPackVersion, "--locked", "--root", $ToolRoot) $bootstrapLog
}
$versionOutput = (& $WasmPack --version).Trim()
if ($LASTEXITCODE -ne 0 -or $versionOutput -notmatch [regex]::Escape($WasmPackVersion)) {
    throw "target-local wasm-pack version mismatch: $versionOutput"
}
[System.IO.File]::WriteAllText((Join-Path $EvidenceDir "wasm-pack-version.txt"), $versionOutput + [Environment]::NewLine, $Utf8NoBom)

$platform = [ordered]@{
    os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
    arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
}
Write-JsonNoBom (Join-Path $EvidenceDir "wasm-pack-bootstrap.json") ([ordered]@{
    schema_version = 1
    status = "passed"
    version = $WasmPackVersion
    version_output = $versionOutput
    source = "crates.io:wasm-pack/$WasmPackVersion"
    install_method = "cargo install --locked --root target-local"
    install_path = $WasmPack
    command = $installCommand
    checksum = "not_applicable_cargo_install_from_crates_io"
    platform = $platform
})

Invoke-Logged "rustup target add wasm32-unknown-unknown" "rustup" @("target", "add", "wasm32-unknown-unknown") (Join-Path $EvidenceDir "rustup-target-log.txt")

Clear-TargetDir $WebPackage
Clear-TargetDir $NodePackage

Invoke-Logged "wasm-pack web build" $WasmPack @("build", "crates/oxide-wasm", "--target", "web", "--out-dir", $WebPackage) (Join-Path $EvidenceDir "wasm-pack-web-build-log.txt")
Invoke-Logged "wasm-pack node build" $WasmPack @("build", "crates/oxide-wasm", "--target", "nodejs", "--out-dir", $NodePackage) (Join-Path $EvidenceDir "wasm-pack-node-build-log.txt")

foreach ($pkg in @($WebPackage, $NodePackage)) {
    Copy-Item -LiteralPath (Join-Path $Repo "crates/oxide-wasm/README.md") -Destination (Join-Path $pkg "README.md") -Force
}

$webInspection = Inspect-WasmPackage "web" $WebPackage
$nodeInspection = Inspect-WasmPackage "nodejs" $NodePackage
$inspection = [ordered]@{
    schema_version = 1
    status = if ($webInspection.result -eq "passed" -and $nodeInspection.result -eq "passed") { "passed" } else { "failed" }
    packages = @($webInspection, $nodeInspection)
}
Write-JsonNoBom (Join-Path $EvidenceDir "wasm-package-inspection.json") $inspection 32
if ($inspection.status -ne "passed") {
    throw "WASM package inspection failed"
}

$nodeSmoke = Join-Path $EvidenceDir "wasm-pack-node-smoke.json"
Invoke-Logged "packaged node smoke" "node" @("scripts/prompt03b_wasm_pack_node_smoke.mjs", $NodePackage, $Fixture, $nodeSmoke) (Join-Path $EvidenceDir "wasm-pack-node-smoke-log.txt")
$smoke = Get-Content -Raw -LiteralPath $nodeSmoke | ConvertFrom-Json
if ($smoke.status -ne "passed") {
    throw "packaged Node smoke failed"
}

Write-JsonNoBom (Join-Path $EvidenceDir "wasm-pack-gate.json") ([ordered]@{
    schema_version = 1
    status = "passed"
    wasm_pack = (Join-Path $EvidenceDir "wasm-pack-bootstrap.json")
    web_package = $WebPackage
    node_package = $NodePackage
    inspection = (Join-Path $EvidenceDir "wasm-package-inspection.json")
    node_smoke = $nodeSmoke
})

Write-Host "wasm-pack Prompt 03B gate passed: $EvidenceDir"
