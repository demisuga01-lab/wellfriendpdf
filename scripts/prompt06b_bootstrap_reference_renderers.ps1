param(
    [string]$ToolsDir = "target/prompt06b-tools",
    [string]$ManifestPath = "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json",
    [int]$Dpi = 72,
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = "Stop"

function Resolve-RepoRoot {
    $root = (& git rev-parse --show-toplevel).Trim()
    if (-not $root) {
        throw "Unable to resolve repository root"
    }
    return (Resolve-Path $root).Path
}

function Get-ToolPath {
    param(
        [string]$EnvName,
        [string[]]$Names
    )
    $configured = [Environment]::GetEnvironmentVariable($EnvName)
    if ($configured -and (Test-Path $configured)) {
        return (Resolve-Path $configured).Path
    }
    foreach ($name in $Names) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($cmd) {
            return $cmd.Source
        }
    }
    return $null
}

function Get-Sha256 {
    param([string]$Path)
    if (-not (Test-Path $Path)) {
        return $null
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Quote-ProcessArgument {
    param([string]$Value)
    if ($Value -notmatch '[\s"]') {
        return $Value
    }
    return '"' + ($Value -replace '"', '\"') + '"'
}

function Invoke-VersionCommand {
    param(
        [string]$Path,
        [string[]]$Arguments
    )
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $psi = [System.Diagnostics.ProcessStartInfo]::new()
        if ($Path.ToLowerInvariant().EndsWith(".cmd") -or $Path.ToLowerInvariant().EndsWith(".bat")) {
            $psi.FileName = $env:ComSpec
            $psi.Arguments = "/d /c " + (Quote-ProcessArgument $Path) + " " + (($Arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " ")
        } else {
            $psi.FileName = $Path
            $psi.Arguments = ($Arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join " "
        }
        $psi.RedirectStandardOutput = $true
        $psi.RedirectStandardError = $true
        $psi.UseShellExecute = $false
        $proc = [System.Diagnostics.Process]::Start($psi)
        $stdout = $proc.StandardOutput.ReadToEnd()
        $stderr = $proc.StandardError.ReadToEnd()
        $proc.WaitForExit()
        $text = (($stdout + $stderr) -replace "`r", "").Trim()
        return [ordered]@{
            command = @($Path) + $Arguments
            exit_status = $proc.ExitCode
            stdout = $stdout.Trim()
            stderr = $stderr.Trim()
            output = $text
            elapsed_ms = [int]$watch.ElapsedMilliseconds
        }
    } catch {
        return [ordered]@{
            command = @($Path) + $Arguments
            exit_status = -1
            stdout = ""
            stderr = $_.Exception.Message
            output = $_.Exception.Message
            elapsed_ms = [int]$watch.ElapsedMilliseconds
        }
    }
}

function Bootstrap-MuPdf {
    param(
        [string]$RepoRoot,
        [string]$ToolsRoot
    )
    $existing = Get-ToolPath -EnvName "MUTOOL" -Names @("mutool", "mutool.exe")
    if ($existing) {
        return [ordered]@{
            path = $existing
            strategy = "existing_path_or_env"
            source_url = $null
            checksum_status = "not_applicable_existing_binary"
            checksum_expected = $null
            checksum_actual = Get-Sha256 $existing
        }
    }

    $mupdfDir = Join-Path $ToolsRoot "mupdf"
    $downloadDir = Join-Path $ToolsRoot "downloads"
    $zip = Join-Path $downloadDir "mupdf-1.27.0-windows.zip"
    $sourceUrl = "https://casper.mupdf.com/downloads/archive/mupdf-1.27.0-windows.zip"
    $expected = "f3e60b630453301914e52fb8ec001f6ab56cdb90daf39e533deae3ff214fcff8"
    $stableMutool = Join-Path $mupdfDir "mutool.exe"

    New-Item -ItemType Directory -Force -Path $downloadDir, $mupdfDir | Out-Null
    if (-not (Test-Path $zip)) {
        Invoke-WebRequest -Uri $sourceUrl -OutFile $zip
    }
    $actual = Get-Sha256 $zip
    if ($actual -ne $expected) {
        throw "MuPDF archive checksum mismatch: expected $expected got $actual"
    }
    if (-not (Test-Path $stableMutool)) {
        $extract = Join-Path $mupdfDir "extracted"
        if (Test-Path $extract) {
            Remove-Item -LiteralPath $extract -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path $extract | Out-Null
        Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
        $found = Get-ChildItem -LiteralPath $extract -Recurse -Filter "mutool.exe" | Select-Object -First 1
        if (-not $found) {
            throw "MuPDF archive did not contain mutool.exe"
        }
        Copy-Item -LiteralPath $found.FullName -Destination $stableMutool -Force
    }

    return [ordered]@{
        path = (Resolve-Path $stableMutool).Path
        strategy = "target_local_download_scoop_extras_manifest"
        source_url = $sourceUrl
        checksum_status = "verified_archive_sha256"
        checksum_expected = $expected
        checksum_actual = $actual
    }
}

function Bootstrap-Pdfium {
    param(
        [string]$RepoRoot,
        [string]$ToolsRoot
    )
    $existing = Get-ToolPath -EnvName "PDFIUM_TEST" -Names @("pdfium_test", "pdfium_test.exe")
    if ($existing) {
        return [ordered]@{
            path = $existing
            strategy = "existing_pdfium_test_path_or_env"
            source_url = $null
            checksum_status = "not_applicable_existing_binary"
            package = "pdfium_test"
            package_version = $null
            pdfium_version = $null
            checksums = [ordered]@{ executable = Get-Sha256 $existing }
        }
    }

    $pdfiumDir = Join-Path $ToolsRoot "pdfium"
    $venv = Join-Path $pdfiumDir ".venv"
    $python = Join-Path $venv "Scripts/python.exe"
    $wrapper = Join-Path $pdfiumDir "pdfium_test.cmd"
    $renderScript = Join-Path $RepoRoot "scripts/prompt06b_pdfium_render.py"
    New-Item -ItemType Directory -Force -Path $pdfiumDir | Out-Null

    if (-not (Test-Path $python)) {
        & uv venv --python 3.12 $venv | Out-Null
    }
    & uv pip install --python $python "pypdfium2==4.30.0" "pillow" | Out-Null

    $wrapperBody = @"
@echo off
setlocal
"$python" "$renderScript" %*
"@
    $shouldWriteWrapper = $true
    if (Test-Path $wrapper) {
        $existingWrapper = Get-Content -LiteralPath $wrapper -Raw -ErrorAction SilentlyContinue
        $shouldWriteWrapper = $existingWrapper -ne $wrapperBody
    }
    if ($shouldWriteWrapper) {
        Set-Content -LiteralPath $wrapper -Value $wrapperBody -Encoding ASCII
    }

    $versionJson = & $wrapper "--version-json"
    $version = $versionJson | ConvertFrom-Json
    $pdfiumDll = Get-ChildItem -LiteralPath $venv -Recurse -Filter "pdfium.dll" | Select-Object -First 1
    $checksums = [ordered]@{
        wrapper = Get-Sha256 $wrapper
        render_script = Get-Sha256 $renderScript
        pdfium_dll = if ($pdfiumDll) { Get-Sha256 $pdfiumDll.FullName } else { $null }
    }

    return [ordered]@{
        path = (Resolve-Path $wrapper).Path
        strategy = "target_local_pypdfium2_wrapper_pdfium_test_compatible"
        source_url = "https://pypi.org/project/pypdfium2/4.30.0/"
        checksum_status = "post_install_hashes_recorded_no_preverified_wheel_lock"
        package = "pypdfium2"
        package_version = $version.package_version
        pdfium_version = $version.pdfium_version
        pdfium_build = $version.pdfium_build
        python = $version.python
        module = $version.module
        checksums = $checksums
    }
}

$repoRoot = Resolve-RepoRoot
Set-Location $repoRoot
$toolsRoot = Join-Path $repoRoot $ToolsDir
$manifest = Join-Path $repoRoot $ManifestPath
New-Item -ItemType Directory -Force -Path $toolsRoot, (Split-Path $manifest -Parent) | Out-Null

$popplerPath = Get-ToolPath -EnvName "POPPLER_PDFTOPPM" -Names @("pdftoppm")
$pdfium = Bootstrap-Pdfium -RepoRoot $repoRoot -ToolsRoot $toolsRoot
$mupdf = Bootstrap-MuPdf -RepoRoot $repoRoot -ToolsRoot $toolsRoot

$tools = [ordered]@{}
if ($popplerPath) {
    $popVersion = Invoke-VersionCommand -Path $popplerPath -Arguments @("-v")
    $tools.poppler = [ordered]@{
        name = "poppler"
        executable_path = $popplerPath
        availability = "available"
        detected_version = $popVersion.output
        version_command = $popVersion
        source_bootstrap_strategy = "existing_path_or_env"
        source_url = $null
        checksum_status = "not_applicable_existing_binary"
        checksum = Get-Sha256 $popplerPath
        command_template = @("pdftoppm", "-png", "-r", "$Dpi", "-f", "{page}", "-l", "{page}", "{input}", "{output_prefix}")
        supported_output_format = "png"
        dpi = $Dpi
        timeout_seconds = $TimeoutSeconds
        normalization_quirks = @("opaque white/default Splash background", "page selection explicit", "PNG output explicit")
    }
} else {
    $tools.poppler = [ordered]@{
        name = "poppler"
        executable_path = $null
        availability = "unavailable"
        detected_version = $null
        source_bootstrap_strategy = "not_bootstrapped_prompt06b_requires_existing_pdftoppm"
        checksum_status = "not_available"
        command_template = @()
        supported_output_format = "png"
        dpi = $Dpi
        timeout_seconds = $TimeoutSeconds
        normalization_quirks = @()
    }
}

$pdfVersion = Invoke-VersionCommand -Path $pdfium.path -Arguments @("--version")
$tools.pdfium = [ordered]@{
    name = "pdfium"
    executable_path = $pdfium.path
    availability = "available"
    detected_version = $pdfVersion.output
    version_command = $pdfVersion
    source_bootstrap_strategy = $pdfium.strategy
    source_url = $pdfium.source_url
    checksum_status = $pdfium.checksum_status
    checksums = $pdfium.checksums
    package = $pdfium.package
    package_version = $pdfium.package_version
    pdfium_version = $pdfium.pdfium_version
    pdfium_build = $pdfium.pdfium_build
    command_template = @("pdfium_test", "--png", "--output={output}", "--first-page={page}", "--last-page={page}", "{input}")
    supported_output_format = "png"
    dpi = $Dpi
    timeout_seconds = $TimeoutSeconds
    normalization_quirks = @("target-local pypdfium2 wrapper", "opaque white fill_color", "RGB PNG output", "annotation/form drawing requested when supported")
}

$muVersion = Invoke-VersionCommand -Path $mupdf.path -Arguments @("-v")
$tools.mupdf = [ordered]@{
    name = "mupdf"
    executable_path = $mupdf.path
    availability = "available"
    detected_version = $muVersion.output
    version_command = $muVersion
    source_bootstrap_strategy = $mupdf.strategy
    source_url = $mupdf.source_url
    checksum_status = $mupdf.checksum_status
    checksum_expected = $mupdf.checksum_expected
    checksum_actual = $mupdf.checksum_actual
    executable_checksum = Get-Sha256 $mupdf.path
    command_template = @("mutool", "draw", "-o", "{output}", "-r", "$Dpi", "{input}", "{page}")
    supported_output_format = "png"
    dpi = $Dpi
    timeout_seconds = $TimeoutSeconds
    normalization_quirks = @("default opaque page background", "page selection explicit", "PNG inferred from output extension")
}

$manifestPayload = [ordered]@{
    schema_version = 1
    kind = "prompt06b_reference_tool_manifest"
    host = [ordered]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        repo_root = $repoRoot
    }
    tools = $tools
}

$json = $manifestPayload | ConvertTo-Json -Depth 20
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($manifest, $json + [Environment]::NewLine, $utf8NoBom)

if ($tools.poppler.availability -ne "available" -or $tools.pdfium.availability -ne "available" -or $tools.mupdf.availability -ne "available") {
    Write-Error "Prompt 06B reference bootstrap did not make all required tools available. See $manifest"
    exit 1
}

Write-Host "Prompt 06B reference renderer manifest: $manifest"
Write-Host "Poppler: $($tools.poppler.executable_path)"
Write-Host "PDFium: $($tools.pdfium.executable_path)"
Write-Host "MuPDF: $($tools.mupdf.executable_path)"
