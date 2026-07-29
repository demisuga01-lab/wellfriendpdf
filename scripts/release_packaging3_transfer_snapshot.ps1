param(
    [Parameter(Mandatory = $true)]
    [string]$RemoteResultDir,

    [Parameter(Mandatory = $true)]
    [string]$RemoteTempDir
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$stamp = Get-Date -Format 'yyyyMMddTHHmmssZ'
$archive = Join-Path $repo "target/text_reflow-geometric-semantic-reflow/text_reflow-source-$stamp.tar.gz"
$manifest = Join-Path $repo "target/text_reflow-geometric-semantic-reflow/text_reflow-source-$stamp.manifest.txt"

Push-Location $repo
try {
    # Transfer only source-controlled inputs.  Historical release artifacts
    # under target/ may themselves be tracked in this long-lived repository;
    # including them turns an exact source transfer into a multi-gigabyte
    # result-cache transfer and can exhaust the VPS temporary quota.  Roadmap task
    # 33 validation regenerates its own evidence under the remote result
    # directory, so target/ is intentionally excluded from the source manifest.
    $files = git ls-files -co --exclude-standard | Where-Object {
        $_ -notmatch '^(target|fuzz/target|bindings/java/build|bindings/dotnet/.*/(?:bin|obj))/'
    }
    if (-not $files) {
        throw 'text reflow transfer source manifest is empty.'
    }
    $files | Set-Content -LiteralPath $manifest -Encoding UTF8
    $files | & tar.exe -czf $archive -T -
    if ($LASTEXITCODE -ne 0) {
        throw "tar failed with exit $LASTEXITCODE"
    }
    $localHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    $remoteArchive = "$RemoteTempDir/text_reflow-source-$stamp.tar.gz"
    # The archive destination must exist before SCP.  Keep this as a separate
    # remote operation so a transfer failure cannot be mistaken for a source
    # validation failure.
    & ssh.exe 'demisuga01@35.185.176.47' "mkdir -p $RemoteResultDir/source $RemoteTempDir/source"
    if ($LASTEXITCODE -ne 0) {
        throw "remote directory preparation failed with exit $LASTEXITCODE"
    }
    & scp.exe $archive "demisuga01@35.185.176.47:$remoteArchive"
    if ($LASTEXITCODE -ne 0) {
        throw "scp failed with exit $LASTEXITCODE"
    }
    & ssh.exe 'demisuga01@35.185.176.47' "sha256sum $remoteArchive; tar -xzf $remoteArchive -C $RemoteTempDir/source; find $RemoteTempDir/source -type f -print0 | sort -z | xargs -0 sha256sum > $RemoteResultDir/source/source-manifest.sha256"
    if ($LASTEXITCODE -ne 0) {
        throw "remote extraction failed with exit $LASTEXITCODE"
    }
    [pscustomobject]@{
        archive = $archive
        archive_sha256 = $localHash
        source_manifest = $manifest
        remote_archive = $remoteArchive
        remote_source = "$RemoteTempDir/source"
    } | ConvertTo-Json -Depth 3
}
finally {
    Pop-Location
}
