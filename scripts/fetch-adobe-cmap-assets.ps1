param(
    [Parameter(Mandatory=$true)][ValidateSet('cmap-resources','mapping-resources-pdf')][string]$Repository,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{40}$')][string]$Revision,
    [Parameter(Mandatory=$true)][string]$PathsJson
)
# Source-data acquisition only. No local writes, PDF execution, or downloaded
# code execution. The caller installs returned text with apply_patch. Original
# bytes are losslessly compressed; source SHA-256 and Git blob IDs are retained.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Net.Http
$taskClient = New-Object System.Net.Http.HttpClient
$taskClient.Timeout = [TimeSpan]::FromSeconds(45)
$taskPaths = ConvertFrom-Json -InputObject $PathsJson
$taskOutput = @()
try {
    foreach ($taskPath in $taskPaths) {
        if ($taskPath -notmatch '^[A-Za-z0-9_./-]+$' -or $taskPath.Contains('..')) { throw 'Invalid resource path' }
        $taskUri = "https://raw.githubusercontent.com/adobe-type-tools/$Repository/$Revision/$taskPath"
        $taskBytes = $null
        for ($taskAttempt = 0; $taskAttempt -lt 3; $taskAttempt++) {
            try { $taskBytes = $taskClient.GetByteArrayAsync($taskUri).GetAwaiter().GetResult(); break }
            catch { if ($taskAttempt -eq 2) { throw "Resource acquisition failed: $taskUri : $_" } }
        }
        $taskSha = [Security.Cryptography.SHA256]::Create()
        $taskHash = [BitConverter]::ToString($taskSha.ComputeHash($taskBytes)).Replace('-','').ToLowerInvariant()
        $taskSha.Dispose()
        $taskSha1 = [Security.Cryptography.SHA1]::Create()
        $taskPrefix = [Text.Encoding]::ASCII.GetBytes("blob $($taskBytes.Length)`0")
        $taskBlob = [byte[]]($taskPrefix + $taskBytes)
        $taskBlobHash = [BitConverter]::ToString($taskSha1.ComputeHash($taskBlob)).Replace('-','').ToLowerInvariant()
        $taskSha1.Dispose()
        $taskText = [Text.Encoding]::UTF8.GetString($taskBytes)
        if ($taskPath -match '^LICENSE') {
            $taskOutput += [pscustomobject]@{ path=$taskPath; size=$taskBytes.Length; sha256=$taskHash; blob=$taskBlobHash; text=$taskText }
            continue
        }
        $taskBuffer = New-Object IO.MemoryStream
        $taskZip = New-Object IO.Compression.GZipStream($taskBuffer,[IO.Compression.CompressionLevel]::Optimal,$true)
        $taskZip.Write($taskBytes,0,$taskBytes.Length)
        $taskZip.Dispose()
        $taskHex = [BitConverter]::ToString($taskBuffer.ToArray()).Replace('-','').ToLowerInvariant()
        $taskBuffer.Dispose()
        $taskName = [regex]::Match($taskText,'(?m)^/CMapName\s+/([^\s]+)\s+def').Groups[1].Value
        $taskRegistry = [regex]::Match($taskText,'/Registry\s+\(([^)]+)\)').Groups[1].Value
        $taskOrdering = [regex]::Match($taskText,'/Ordering\s+\(([^)]+)\)').Groups[1].Value
        $taskSupplement = [regex]::Match($taskText,'/Supplement\s+(\d+)').Groups[1].Value
        $taskMode = [regex]::Match($taskText,'(?m)^/WMode\s+(\d+)\s+def').Groups[1].Value
        $taskBase = [regex]::Match($taskText,'(?m)^/([^\s]+)\s+usecmap').Groups[1].Value
        $taskSpaces = [regex]::Matches($taskText,'(?s)begincodespacerange\s*(.*?)endcodespacerange')
        $taskLengths = @($taskSpaces | ForEach-Object { [regex]::Matches($_.Groups[1].Value,'<([a-fA-F0-9]+)>') | ForEach-Object { $_.Groups[1].Value.Length / 2 } } | Sort-Object -Unique)
        $taskLength = if ($taskLengths.Count -eq 1) { [int]$taskLengths[0] } else { 0 }
        $taskOutput += [pscustomobject]@{ path=$taskPath; size=$taskBytes.Length; sha256=$taskHash; blob=$taskBlobHash; hex=$taskHex; name=$taskName; registry=$taskRegistry; ordering=$taskOrdering; supplement=$taskSupplement; mode=$taskMode; base=$taskBase; code_size=$taskLength }
    }
    ConvertTo-Json -InputObject $taskOutput -Depth 5 -Compress
} finally { $taskClient.Dispose() }
