param(
  [ValidateSet("baseline", "post", "both")]
  [string]$Phase = "post",
  [string]$OxideBin = "",
  [int]$Dpi = 72,
  [int]$TimeoutSeconds = 120
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
Set-Location $repoRoot

$manifest = "target\prompt06-renderer-native-replay\reference-tool-manifest-prompt06b.json"
if (-not (Test-Path $manifest)) {
  throw "Missing Prompt 06B reference manifest at $manifest. Run scripts\prompt06b_multi_reference_audit.ps1 first."
}

$argsList = @(
  "scripts\prompt07_transparency_compositing_audit.py",
  "--phase", $Phase,
  "--dpi", "$Dpi",
  "--timeout", "$TimeoutSeconds"
)
if ($OxideBin -ne "") {
  $argsList += @("--oxide-bin", $OxideBin)
}

python @argsList
