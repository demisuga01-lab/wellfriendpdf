param(
    [string]$WellfriendBin = "",
    [int]$Dpi = 72,
    [int]$TimeoutSeconds = 120
)

$ErrorActionPreference = "Stop"

$repoRoot = (& git rev-parse --show-toplevel).Trim()
if (-not $repoRoot) {
    throw "Unable to resolve repository root"
}
Set-Location $repoRoot

& powershell -NoProfile -ExecutionPolicy Bypass -File scripts\prompt06b_bootstrap_reference_renderers.ps1 -Dpi $Dpi -TimeoutSeconds $TimeoutSeconds
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$manifest = "target/prompt06-renderer-native-replay/reference-tool-manifest-prompt06b.json"
$argsList = @(
    "scripts\prompt06b_render_compare.py",
    "--manifest", $manifest,
    "--dpi", "$Dpi",
    "--timeout", "$TimeoutSeconds"
)
if (-not [string]::IsNullOrWhiteSpace($WellfriendBin)) {
    $argsList += @("--wellfriendpdf-bin", $WellfriendBin)
}

& python @argsList
exit $LASTEXITCODE
