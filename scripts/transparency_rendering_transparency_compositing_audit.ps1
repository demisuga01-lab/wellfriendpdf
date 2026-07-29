param(
  [ValidateSet("baseline", "post", "both")]
  [string]$Phase = "post",
  [string]$WellfriendBin = "",
  [int]$Dpi = 72,
  [int]$TimeoutSeconds = 120
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
Set-Location $repoRoot

$manifest = "target\native_renderer-renderer-native-replay\reference-tool-manifest-reference_renderer.json"
if (-not (Test-Path $manifest)) {
  throw "Missing Reference Renderer reference manifest at $manifest. Run scripts\reference_renderer_multi_reference_audit.ps1 first."
}

$argsList = @(
  "scripts\transparency_rendering_transparency_compositing_audit.py",
  "--phase", $Phase,
  "--dpi", "$Dpi",
  "--timeout", "$TimeoutSeconds"
)
if ($WellfriendBin -ne "") {
  $argsList += @("--wellfriendpdf-bin", $WellfriendBin)
}

python @argsList
