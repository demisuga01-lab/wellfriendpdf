param(
    [string]$Policy = "in_process",
    [string]$Output = ""
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$oxide = Join-Path $repo "target\debug\oxide.exe"
if (!(Test-Path -LiteralPath $oxide)) {
    & cargo build -p oxide-cli
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build -p oxide-cli failed"
    }
}

$cliArgs = @(
    "codec-isolation-report",
    "--filter", "FlateDecode",
    "--sample-text", "hello oxide",
    "--policy", $Policy
)

if ($Output -ne "") {
    $text = (& $oxide @cliArgs | Out-String)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Output, $text, $utf8NoBom)
} else {
    & $oxide @cliArgs
}
if ($LASTEXITCODE -ne 0) {
    throw "codec isolation CLI example failed"
}
