$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ArtifactDir = Join-Path $Repo "target/prompt02-binding-parity"
$NativeName = if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
    "wellfriendpdf_capi.dll"
} elseif ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
    "libwellfriendpdf_capi.dylib"
} else {
    "libwellfriendpdf_capi.so"
}
$NativePath = Join-Path $Repo "target/debug/$NativeName"
$JavaClasses = Join-Path $Repo "bindings/java/target/classes"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Invoke-Checked {
    param([string]$FilePath, [string[]]$Arguments, [string]$Display)
    Write-Host "==> $Display"
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Display failed with exit code $LASTEXITCODE"
    }
}

function Tool-Status {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return "<missing>"
}

function Write-JsonNoBom {
    param([string]$Path, [object]$Payload)
    $json = $Payload | ConvertTo-Json -Depth 6
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $Utf8NoBom)
}

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null

if (!(Test-Path $NativePath)) {
    Invoke-Checked "cargo" @("build", "-p", "wellfriendpdf-capi") "cargo build -p wellfriendpdf-capi"
}

$env:WELLFRIENDPDF_NATIVE_LIBRARY = $NativePath

$commands = New-Object System.Collections.Generic.List[string]
Invoke-Checked "cargo" @("test", "-p", "wellfriendpdf-capi", "capi_repeated_open_report_free_stress") "C ABI repeated open/report/free stress"
$commands.Add("cargo test -p wellfriendpdf-capi capi_repeated_open_report_free_stress")

Invoke-Checked "dotnet" @("test", (Join-Path $Repo "bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj"), "--filter", "RepeatedOpenReportAndDisposeStress") ".NET SafeHandle dispose stress"
$commands.Add("dotnet test bindings/dotnet/WellfriendPdf.Tests --filter RepeatedOpenReportAndDisposeStress")

$javaSources = @(
    (Get-ChildItem -LiteralPath (Join-Path $Repo "bindings/java/src/main/java") -Recurse -Filter "*.java").FullName
    (Get-ChildItem -LiteralPath (Join-Path $Repo "bindings/java/src/test/java") -Recurse -Filter "*.java" |
        Where-Object { $_.Name -ne "WellfriendPdfJUnitTest.java" }).FullName
)
New-Item -ItemType Directory -Force -Path $JavaClasses | Out-Null
$javacArgs = @("--enable-preview", "--release", "25", "-d", $JavaClasses) + $javaSources
Invoke-Checked "javac" $javacArgs "javac Java stress smoke"
$commands.Add("javac --enable-preview --release 25 Java sources")
Invoke-Checked "java" @("--enable-preview", "--enable-native-access=ALL-UNNAMED", "-cp", $JavaClasses, "io.wellfriendpdf.WellfriendPdfSmokeTest") "Java AutoCloseable stress smoke"
$commands.Add("java --enable-preview --enable-native-access=ALL-UNNAMED io.wellfriendpdf.WellfriendPdfSmokeTest")

$payload = [ordered]@{
    schema_version = 1
    result = "passed"
    tools = [ordered]@{
        valgrind = Tool-Status "valgrind"
        cargo_llvm_cov = Tool-Status "cargo-llvm-cov"
        dotnet = Tool-Status "dotnet"
        java = Tool-Status "java"
        javac = Tool-Status "javac"
        rustup = Tool-Status "rustup"
    }
    local_limit = "Valgrind is unavailable on this Windows host; ASan/TSan are covered by .github/workflows/sanitizers.yml on Linux nightly/toolchain CI."
    ci_gate = ".github/workflows/sanitizers.yml"
    commands = $commands
    native_library = $NativePath
}

Write-JsonNoBom (Join-Path $ArtifactDir "memory-smoke.json") $payload
Write-Host "wrote $(Join-Path $ArtifactDir 'memory-smoke.json')"
