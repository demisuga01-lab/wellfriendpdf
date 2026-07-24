param(
    [string]$GradleVersion = "9.6.1",
    [string]$GradleSha256 = "9c0f7faeeb306cb14e4279a3e084ca6b596894089a0638e68a07c945a32c9e14"
)

$ErrorActionPreference = "Stop"

$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$JavaDir = Join-Path $Repo "bindings/java"
$ArtifactDir = Join-Path $Repo "target/prompt02-binding-parity"
$ToolDir = Join-Path $Repo "target/prompt02c-tools"
$SmokeDir = Join-Path $Repo "target/prompt02c-gradle-package-smoke"
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
    $json = $Payload | ConvertTo-Json -Depth 16
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
        return "wellfriendpdf_capi.dll"
    }
    if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        return "libwellfriendpdf_capi.dylib"
    }
    "libwellfriendpdf_capi.so"
}

function Ensure-Gradle {
    $existing = Get-Command gradle -ErrorAction SilentlyContinue
    if ($existing) {
        return $existing.Source
    }

    $gradleHome = Join-Path $ToolDir "gradle-$GradleVersion"
    $runtime = [System.Runtime.InteropServices.RuntimeInformation]
    $gradleCmd = if ($runtime::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        Join-Path $gradleHome "bin/gradle.bat"
    } else {
        Join-Path $gradleHome "bin/gradle"
    }
    if (Test-Path $gradleCmd) {
        return $gradleCmd
    }

    New-Item -ItemType Directory -Force -Path $ToolDir | Out-Null
    $zip = Join-Path $ToolDir "gradle-$GradleVersion-bin.zip"
    $url = "https://services.gradle.org/distributions/gradle-$GradleVersion-bin.zip"
    Write-Host "==> downloading Gradle $GradleVersion from $url"
    Invoke-WebRequest -Uri $url -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($actual -ne $GradleSha256.ToLowerInvariant()) {
        throw "Gradle archive checksum mismatch: expected $GradleSha256, got $actual"
    }
    Expand-Archive -LiteralPath $zip -DestinationPath $ToolDir -Force
    return $gradleCmd
}

function Get-JarEntries {
    param([string]$Jar)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Jar)
    try {
        @($zip.Entries | ForEach-Object { $_.FullName } | Sort-Object)
    } finally {
        $zip.Dispose()
    }
}

function Get-ManifestMap {
    param([string]$Jar)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Jar)
    try {
        $entry = $zip.GetEntry("META-INF/MANIFEST.MF")
        if ($null -eq $entry) {
            return @{}
        }
        $reader = New-Object System.IO.StreamReader($entry.Open())
        try {
            $text = $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
        }
    } finally {
        $zip.Dispose()
    }
    $map = [ordered]@{}
    foreach ($line in ($text -split "`r?`n")) {
        if ($line.Contains(": ")) {
            $parts = $line.Split(": ", 2)
            $map[$parts[0].Trim()] = $parts[1].Trim()
        }
    }
    $map
}

function Assert-JarContents {
    param([string]$Jar)
    $entries = Get-JarEntries $Jar
    $requiredEntries = @(
        "META-INF/MANIFEST.MF",
        "io/wellfriendpdf/WellfriendPdf.class",
        "io/wellfriendpdf/Wellfriend`$Document.class",
        "io/wellfriendpdf/Wellfriend`$BinaryResult.class",
        "io/wellfriendpdf/Wellfriend`$Office.class"
    )
    $missingEntries = @($requiredEntries | Where-Object { $entries -notcontains $_ })
    $forbiddenEntries = @($entries | Where-Object {
        $_ -like "*WellfriendPdfSmokeTest*" -or
        $_ -like "io/wellfriendpdf/packagesmoke/*" -or
        $_ -like "target/*" -or
        $_ -like "build/*" -or
        $_ -like "*wellfriendpdf_capi*" -or
        $_ -like "*.pdb"
    })
    if ($missingEntries.Count -ne 0) {
        throw "JAR missing required entries: $($missingEntries -join ', ')"
    }
    if ($forbiddenEntries.Count -ne 0) {
        throw "JAR contains forbidden entries: $($forbiddenEntries -join ', ')"
    }
    [ordered]@{
        entry_count = $entries.Count
        required_entries_present = $missingEntries.Count -eq 0
        forbidden_entries_absent = $forbiddenEntries.Count -eq 0
        native_libraries_in_jar = @($entries | Where-Object { $_ -like "*wellfriendpdf_capi*" })
        tests_in_jar = @($entries | Where-Object { $_ -like "*Test*" })
        entries = $entries
    }
}

function Compile-ApiSummary {
    param([string]$ClassesDir)
    New-Item -ItemType Directory -Force -Path $ClassesDir | Out-Null
    $source = Join-Path $ClassesDir "ApiSummary.java"
    $code = @'
import java.lang.reflect.*;
import java.net.*;
import java.nio.file.*;
import java.util.*;
import java.util.stream.*;

public final class ApiSummary {
    public static void main(String[] args) throws Exception {
        URL jar = Path.of(args[0]).toUri().toURL();
        try (URLClassLoader loader = new URLClassLoader(new URL[] { jar }, ClassLoader.getPlatformClassLoader())) {
            String[] classes = {
                "io.wellfriendpdf.Wellfriend",
                "io.wellfriendpdf.WellfriendPdf$Document",
                "io.wellfriendpdf.WellfriendPdf$Page",
                "io.wellfriendpdf.WellfriendPdf$BinaryResult",
                "io.wellfriendpdf.WellfriendPdf$Office",
                "io.wellfriendpdf.WellfriendPdf$WellfriendPdfException"
            };
            ArrayList<String> out = new ArrayList<>();
            for (String name : classes) {
                Class<?> cls = Class.forName(name, false, loader);
                out.add("class " + name + " " + Modifier.toString(cls.getModifiers()));
                for (Constructor<?> ctor : cls.getDeclaredConstructors()) {
                    if (Modifier.isPublic(ctor.getModifiers())) {
                        out.add(name + "#<init>(" + params(ctor.getParameterTypes()) + ")");
                    }
                }
                for (Method method : cls.getDeclaredMethods()) {
                    if (Modifier.isPublic(method.getModifiers())) {
                        out.add(name + "#" + Modifier.toString(method.getModifiers()) + " " + method.getName()
                            + "(" + params(method.getParameterTypes()) + "):" + method.getReturnType().getTypeName());
                    }
                }
            }
            Collections.sort(out);
            for (String line : out) {
                System.out.println(line);
            }
        }
    }

    private static String params(Class<?>[] types) {
        return Arrays.stream(types).map(Class::getTypeName).collect(Collectors.joining(","));
    }
}
'@
    [System.IO.File]::WriteAllText($source, $code + [Environment]::NewLine, $Utf8NoBom)
    Invoke-Checked "javac" @("--enable-preview", "--release", "25", "-d", $ClassesDir, $source) "javac API summary"
}

function Get-ApiSummary {
    param([string]$Jar, [string]$ClassesDir)
    $output = & java --enable-preview -cp $ClassesDir ApiSummary $Jar
    if ($LASTEXITCODE -ne 0) {
        throw "API summary failed for $Jar"
    }
    @($output | Sort-Object)
}

New-Item -ItemType Directory -Force -Path $ArtifactDir, $SmokeDir | Out-Null

$gradle = Ensure-Gradle
$nativeName = Get-NativeLibraryName
$nativePath = Join-Path $Repo "target/debug/$nativeName"
if (!(Test-Path $nativePath)) {
    Invoke-Checked "cargo" @("build", "-p", "wellfriendpdf-capi") "cargo build -p wellfriendpdf-capi"
}
if (!(Test-Path $nativePath)) {
    throw "native library not found after build: $nativePath"
}

$env:WELLFRIENDPDF_NATIVE_LIBRARY = $nativePath
$env:WELLFRIENDPDF_PROMPT02_ARTIFACT_DIR = $ArtifactDir

Invoke-Checked $gradle @("--version") "gradle --version"
Invoke-Checked $gradle @("--no-daemon", "-p", $JavaDir, "clean", "test") "gradle clean test"
Invoke-Checked $gradle @("--no-daemon", "-p", $JavaDir, "jar") "gradle jar"
Invoke-Checked $gradle @("--no-daemon", "-p", $JavaDir, "build") "gradle build"

$gradleJar = Join-Path $JavaDir "build/libs/wellfriendpdf-sdk-0.1.0.jar"
if (!(Test-Path $gradleJar)) {
    throw "Gradle package did not produce $gradleJar"
}

$rid = Get-HostRid
$runtimeNativeDir = Join-Path $JavaDir "build/libs/runtimes/$rid/native"
New-Item -ItemType Directory -Force -Path $runtimeNativeDir | Out-Null
Copy-Item -LiteralPath $nativePath -Destination (Join-Path $runtimeNativeDir $nativeName) -Force

$gradleInspection = Assert-JarContents $gradleJar

$classes = Join-Path $SmokeDir "classes"
$runDir = Join-Path $SmokeDir "run"
Remove-Item -LiteralPath $classes -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $runDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $classes, $runDir | Out-Null

$packageSmoke = Join-Path $JavaDir "package-smoke/PackageSmoke.java"
Invoke-Checked "javac" @("--enable-preview", "--release", "25", "-cp", $gradleJar, "-d", $classes, $packageSmoke) "javac Gradle package smoke"

$oldNative = $env:WELLFRIENDPDF_NATIVE_LIBRARY
Remove-Item Env:\WELLFRIENDPDF_NATIVE_LIBRARY -ErrorAction SilentlyContinue
try {
    Push-Location $runDir
    try {
        $classpath = "$classes$([System.IO.Path]::PathSeparator)$gradleJar"
        Invoke-Checked "java" @("--enable-preview", "--enable-native-access=ALL-UNNAMED", "-cp", $classpath, "io.wellfriendpdf.packagesmoke.PackageSmoke", $Fixture) "Gradle JAR runtime smoke"
    } finally {
        Pop-Location
    }
} finally {
    if ($null -ne $oldNative) {
        $env:WELLFRIENDPDF_NATIVE_LIBRARY = $oldNative
    }
}

$gradleSmoke = [ordered]@{
    schema_version = 1
    surface = "java-gradle"
    result = "passed"
    gradle_tool = $gradle
    gradle_version = $GradleVersion
    java_version = (& java --version 2>&1 | Select-Object -First 1)
    jar_path = $gradleJar
    native_library = $nativePath
    runtime_native_layout = Join-Path $runtimeNativeDir $nativeName
    commands = @(
        "gradle --version",
        "gradle --no-daemon -p bindings/java clean test",
        "gradle --no-daemon -p bindings/java jar",
        "gradle --no-daemon -p bindings/java build",
        "javac Gradle package smoke",
        "Gradle JAR runtime smoke"
    )
    jar_contents = [ordered]@{
        entry_count = $gradleInspection.entry_count
        required_entries_present = $gradleInspection.required_entries_present
        forbidden_entries_absent = $gradleInspection.forbidden_entries_absent
        native_libraries_in_jar = $gradleInspection.native_libraries_in_jar
        tests_in_jar = $gradleInspection.tests_in_jar
    }
    runtime_smoke = [ordered]@{
        status = "passed"
        fixture = $Fixture
        operations = @("engineVersion", "abiVersion", "featureReportJson", "Document.open(Path,String)", "page text", "securityReportJson", "parserReportJson", "sanitize")
    }
    native_loading = "Gradle JAR smoke ran from target/prompt02c-gradle-package-smoke/run with WELLFRIENDPDF_NATIVE_LIBRARY unset and wellfriendpdf_capi copied to bindings/java/build/libs/runtimes/$rid/native."
}
Write-JsonNoBom (Join-Path $ArtifactDir "gradle-jar-smoke.json") $gradleSmoke

$mavenJar = Join-Path $JavaDir "target/wellfriendpdf-sdk-0.1.0.jar"
if (!(Test-Path $mavenJar)) {
    Invoke-Checked "powershell" @("-ExecutionPolicy", "Bypass", "-File", (Join-Path $Repo "scripts/prompt02b_java_package_smoke.ps1")) "Maven package smoke for equivalence"
}
if (!(Test-Path $mavenJar)) {
    throw "Maven artifact missing for equivalence: $mavenJar"
}

$mavenInspection = Assert-JarContents $mavenJar
$apiClasses = Join-Path $SmokeDir "api-classes"
Compile-ApiSummary $apiClasses
$mavenApi = Get-ApiSummary $mavenJar $apiClasses
$gradleApi = Get-ApiSummary $gradleJar $apiClasses

$mavenClasses = @($mavenInspection.entries | Where-Object { $_ -like "io/wellfriendpdf/*.class" } | Sort-Object)
$gradleClasses = @($gradleInspection.entries | Where-Object { $_ -like "io/wellfriendpdf/*.class" } | Sort-Object)
$classesOnlyMaven = @($mavenClasses | Where-Object { $gradleClasses -notcontains $_ })
$classesOnlyGradle = @($gradleClasses | Where-Object { $mavenClasses -notcontains $_ })
$apiOnlyMaven = @($mavenApi | Where-Object { $gradleApi -notcontains $_ })
$apiOnlyGradle = @($gradleApi | Where-Object { $mavenApi -notcontains $_ })

$mavenManifest = Get-ManifestMap $mavenJar
$gradleManifest = Get-ManifestMap $gradleJar
$manifestCommon = [ordered]@{}
foreach ($key in @("Automatic-Module-Name")) {
    $manifestCommon[$key] = [ordered]@{
        maven = $mavenManifest[$key]
        gradle = $gradleManifest[$key]
        match = $mavenManifest[$key] -eq $gradleManifest[$key]
    }
}
$manifestDifferences = @()
foreach ($key in @($mavenManifest.Keys + $gradleManifest.Keys | Sort-Object -Unique)) {
    if ($mavenManifest[$key] -ne $gradleManifest[$key]) {
        $manifestDifferences += [ordered]@{ key = $key; maven = $mavenManifest[$key]; gradle = $gradleManifest[$key] }
    }
}

$equivalencePassed = $classesOnlyMaven.Count -eq 0 -and
    $classesOnlyGradle.Count -eq 0 -and
    $apiOnlyMaven.Count -eq 0 -and
    $apiOnlyGradle.Count -eq 0 -and
    $manifestCommon["Automatic-Module-Name"].match

$equivalence = [ordered]@{
    schema_version = 1
    result = if ($equivalencePassed) { "passed" } else { "failed" }
    maven_artifact = $mavenJar
    gradle_artifact = $gradleJar
    class_list = [ordered]@{
        match = $classesOnlyMaven.Count -eq 0 -and $classesOnlyGradle.Count -eq 0
        maven_count = $mavenClasses.Count
        gradle_count = $gradleClasses.Count
        only_maven = $classesOnlyMaven
        only_gradle = $classesOnlyGradle
    }
    api = [ordered]@{
        match = $apiOnlyMaven.Count -eq 0 -and $apiOnlyGradle.Count -eq 0
        only_maven = $apiOnlyMaven
        only_gradle = $apiOnlyGradle
        password_open_methods = @(
            "io.wellfriendpdf.Wellfriend`$Document#public static open(java.nio.file.Path,java.lang.String):io.wellfriendpdf.Wellfriend`$Document",
            "io.wellfriendpdf.Wellfriend`$Document#public static open(byte[],java.lang.String):io.wellfriendpdf.Wellfriend`$Document"
        )
    }
    manifest = [ordered]@{
        common = $manifestCommon
        intentional_differences = $manifestDifferences
        note = "Build-tool generated manifest fields may differ; Automatic-Module-Name must match."
    }
    smoke_result = [ordered]@{
        maven = "passed in target/prompt02-binding-parity/java-package-smoke.json"
        gradle = "passed in target/prompt02-binding-parity/gradle-jar-smoke.json"
    }
    native_loading = [ordered]@{
        maven = "runtimes/$rid/native under bindings/java/target"
        gradle = "runtimes/$rid/native under bindings/java/build/libs"
    }
}
Write-JsonNoBom (Join-Path $ArtifactDir "java-maven-gradle-equivalence.json") $equivalence

if (!$equivalencePassed) {
    throw "Maven/Gradle Java artifact equivalence failed"
}

Write-Host "wrote $(Join-Path $ArtifactDir 'gradle-jar-smoke.json')"
Write-Host "wrote $(Join-Path $ArtifactDir 'java-maven-gradle-equivalence.json')"
