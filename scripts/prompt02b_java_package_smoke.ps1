param(
    [string]$MavenVersion = "3.9.9"
)

$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$JavaDir = Join-Path $Repo "bindings/java"
$Pom = Join-Path $JavaDir "pom.xml"
$ArtifactDir = Join-Path $Repo "target/prompt02-binding-parity"
$ToolDir = Join-Path $Repo "target/prompt02b-tools"
$SmokeDir = Join-Path $Repo "target/prompt02b-package-smoke"
$Fixture = Join-Path $Repo "crates/engine/tests/fixtures/tracemonkey.pdf"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Invoke-Checked {
    param([string]$FilePath, [string[]]$Arguments, [string]$Display)
    Write-Host "==> $Display"
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Display failed with exit code $LASTEXITCODE"
    }
}

function Write-JsonNoBom {
    param([string]$Path, [object]$Payload)
    $json = $Payload | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText($Path, $json + [Environment]::NewLine, $Utf8NoBom)
}

function Get-HostRid {
    $runtime = [System.Runtime.InteropServices.RuntimeInformation]
    $os = if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        "win"
    } elseif ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        "osx"
    } else {
        "linux"
    }
    $archInfo = $runtime::ProcessArchitecture
    $arch = switch ($archInfo) {
        "X64" { "x64" }
        "Arm64" { "arm64" }
        "X86" { "x86" }
        "Arm" { "arm" }
        default { $archInfo.ToString().ToLowerInvariant() }
    }
    "$os-$arch"
}

function Get-NativeLibraryName {
    $runtime = [System.Runtime.InteropServices.RuntimeInformation]
    if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        return "oxide_capi.dll"
    }
    if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        return "liboxide_capi.dylib"
    }
    "liboxide_capi.so"
}

function Ensure-Maven {
    $existing = Get-Command mvn -ErrorAction SilentlyContinue
    if ($existing) {
        return $existing.Source
    }

    $mavenHome = Join-Path $ToolDir "apache-maven-$MavenVersion"
    $runtime = [System.Runtime.InteropServices.RuntimeInformation]
    $mavenCmd = if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        Join-Path $mavenHome "bin/mvn.cmd"
    } else {
        Join-Path $mavenHome "bin/mvn"
    }
    if (Test-Path $mavenCmd) {
        return $mavenCmd
    }

    New-Item -ItemType Directory -Force -Path $ToolDir | Out-Null
    $zip = Join-Path $ToolDir "apache-maven-$MavenVersion-bin.zip"
    $url = "https://archive.apache.org/dist/maven/maven-3/$MavenVersion/binaries/apache-maven-$MavenVersion-bin.zip"
    Write-Host "==> downloading Maven $MavenVersion from $url"
    Invoke-WebRequest -Uri $url -OutFile $zip
    Expand-Archive -LiteralPath $zip -DestinationPath $ToolDir -Force
    return $mavenCmd
}

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
New-Item -ItemType Directory -Force -Path $SmokeDir | Out-Null

$mvn = Ensure-Maven
$nativeName = Get-NativeLibraryName
$nativePath = Join-Path $Repo "target/debug/$nativeName"
if (!(Test-Path $nativePath)) {
    Invoke-Checked "cargo" @("build", "-p", "oxide-capi") "cargo build -p oxide-capi"
}
if (!(Test-Path $nativePath)) {
    throw "native library not found after build: $nativePath"
}

$env:OXIDE_NATIVE_LIBRARY = $nativePath
$env:OXIDE_PROMPT02_ARTIFACT_DIR = $ArtifactDir

Invoke-Checked $mvn @("-f", $Pom, "-version") "mvn -version"
Invoke-Checked $mvn @("-f", $Pom, "clean", "test") "mvn clean test"
Invoke-Checked $mvn @("-f", $Pom, "package") "mvn package"

$jar = Join-Path $JavaDir "target/oxide-sdk-0.1.0.jar"
if (!(Test-Path $jar)) {
    throw "Maven package did not produce $jar"
}

$rid = Get-HostRid
$runtimeNativeDir = Join-Path $JavaDir "target/runtimes/$rid/native"
New-Item -ItemType Directory -Force -Path $runtimeNativeDir | Out-Null
Copy-Item -LiteralPath $nativePath -Destination (Join-Path $runtimeNativeDir $nativeName) -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
try {
    $entries = @($zip.Entries | ForEach-Object { $_.FullName } | Sort-Object)
} finally {
    $zip.Dispose()
}

$requiredEntries = @(
    "META-INF/MANIFEST.MF",
    "org/oxidepdf/Oxide.class",
    "org/oxidepdf/Oxide`$Document.class",
    "org/oxidepdf/Oxide`$BinaryResult.class",
    "org/oxidepdf/Oxide`$Office.class"
)
$missingEntries = @($requiredEntries | Where-Object { $entries -notcontains $_ })
$forbiddenEntries = @($entries | Where-Object {
    $_ -like "*OxideSmokeTest*" -or
    $_ -like "org/oxidepdf/packagesmoke/*" -or
    $_ -like "target/*" -or
    $_ -like "*oxide_capi*" -or
    $_ -like "*.pdb"
})
if ($missingEntries.Count -ne 0) {
    throw "JAR missing required entries: $($missingEntries -join ', ')"
}
if ($forbiddenEntries.Count -ne 0) {
    throw "JAR contains forbidden entries: $($forbiddenEntries -join ', ')"
}

$classes = Join-Path $SmokeDir "classes"
$runDir = Join-Path $SmokeDir "run"
Remove-Item -LiteralPath $classes -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $runDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $classes, $runDir | Out-Null

$packageSmoke = Join-Path $JavaDir "package-smoke/PackageSmoke.java"
Invoke-Checked "javac" @("--enable-preview", "--release", "25", "-cp", $jar, "-d", $classes, $packageSmoke) "javac package smoke"

$oldNative = $env:OXIDE_NATIVE_LIBRARY
Remove-Item Env:\OXIDE_NATIVE_LIBRARY -ErrorAction SilentlyContinue
try {
    Push-Location $runDir
    try {
        $classpath = "$classes$([System.IO.Path]::PathSeparator)$jar"
        Invoke-Checked "java" @("--enable-preview", "--enable-native-access=ALL-UNNAMED", "-cp", $classpath, "org.oxidepdf.packagesmoke.PackageSmoke", $Fixture) "JAR runtime smoke"
    } finally {
        Pop-Location
    }
} finally {
    if ($null -ne $oldNative) {
        $env:OXIDE_NATIVE_LIBRARY = $oldNative
    }
}

$payload = [ordered]@{
    schema_version = 1
    surface = "java"
    maven_tool = $mvn
    maven_version = $MavenVersion
    jar_path = $jar
    native_library = $nativePath
    runtime_native_layout = Join-Path $runtimeNativeDir $nativeName
    commands = @(
        "mvn -version",
        "mvn clean test",
        "mvn package",
        "javac package smoke",
        "JAR runtime smoke"
    )
    jar_contents = [ordered]@{
        entry_count = $entries.Count
        required_entries_present = $missingEntries.Count -eq 0
        forbidden_entries_absent = $forbiddenEntries.Count -eq 0
        native_libraries_in_jar = @($entries | Where-Object { $_ -like "*oxide_capi*" })
        tests_in_jar = @($entries | Where-Object { $_ -like "*Test*" })
    }
    native_loading = "JAR smoke ran from target/prompt02b-package-smoke/run with OXIDE_NATIVE_LIBRARY unset and oxide_capi copied to bindings/java/target/runtimes/$rid/native."
    gradle_policy = "Prompt 02B Maven package smoke preserved; Prompt 02C adds authoritative Gradle build/package support."
    result = "passed"
}

Write-JsonNoBom (Join-Path $ArtifactDir "java-package-smoke.json") $payload
Write-Host "wrote $(Join-Path $ArtifactDir 'java-package-smoke.json')"
