param(
    [string]$Policy = "in_process",
    [string]$Output = ""
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$wellfriendpdf = Join-Path $repo "target\debug\wellfriendpdf.exe"
if (!(Test-Path -LiteralPath $wellfriendpdf)) {
    & cargo build -p wellfriendpdf-cli
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build -p wellfriendpdf-cli failed"
    }
}

$cliArgs = @(
    "codec-isolation-report",
    "--filter", "FlateDecode",
    "--sample-text", "hello wellfriendpdf",
    "--policy", $Policy
)

if ($Output -ne "") {
    $text = (& $wellfriendpdf @cliArgs | Out-String)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Output, $text, $utf8NoBom)
} else {
    & $wellfriendpdf @cliArgs
}
if ($LASTEXITCODE -ne 0) {
    throw "codec isolation CLI example failed"
}
