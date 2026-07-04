$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$ArtifactDir = Join-Path $Repo "target/prompt02-binding-parity"
$NativeName = if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
    "oxide_capi.dll"
} elseif ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
    "liboxide_capi.dylib"
} else {
    "liboxide_capi.so"
}
$NativePath = Join-Path $Repo "target/debug/$NativeName"
$JavaClasses = Join-Path $Repo "bindings/java/target/classes"

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

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null

if (!(Test-Path $NativePath)) {
    Invoke-Checked "cargo" @("build", "-p", "oxide-capi") "cargo build -p oxide-capi"
}

$env:OXIDE_NATIVE_LIBRARY = $NativePath

$commands = New-Object System.Collections.Generic.List[string]
Invoke-Checked "cargo" @("test", "-p", "oxide-capi", "capi_repeated_open_report_free_stress") "C ABI repeated open/report/free stress"
$commands.Add("cargo test -p oxide-capi capi_repeated_open_report_free_stress")

Invoke-Checked "dotnet" @("test", (Join-Path $Repo "bindings/dotnet/Oxide.Sdk.Tests/Oxide.Sdk.Tests.csproj"), "--filter", "RepeatedOpenReportAndDisposeStress") ".NET SafeHandle dispose stress"
$commands.Add("dotnet test bindings/dotnet/Oxide.Sdk.Tests --filter RepeatedOpenReportAndDisposeStress")

$javaSources = @(
    (Get-ChildItem -LiteralPath (Join-Path $Repo "bindings/java/src/main/java") -Recurse -Filter "*.java").FullName
    (Get-ChildItem -LiteralPath (Join-Path $Repo "bindings/java/src/test/java") -Recurse -Filter "*.java").FullName
)
New-Item -ItemType Directory -Force -Path $JavaClasses | Out-Null
$javacArgs = @("--enable-preview", "--release", "25", "-d", $JavaClasses) + $javaSources
Invoke-Checked "javac" $javacArgs "javac Java stress smoke"
$commands.Add("javac --enable-preview --release 25 Java sources")
Invoke-Checked "java" @("--enable-preview", "--enable-native-access=ALL-UNNAMED", "-cp", $JavaClasses, "org.oxidepdf.OxideSmokeTest") "Java AutoCloseable stress smoke"
$commands.Add("java --enable-preview --enable-native-access=ALL-UNNAMED org.oxidepdf.OxideSmokeTest")

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

$payload | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $ArtifactDir "memory-smoke.json") -Encoding UTF8
Write-Host "wrote $(Join-Path $ArtifactDir 'memory-smoke.json')"
