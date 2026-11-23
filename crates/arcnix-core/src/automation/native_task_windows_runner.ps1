param([string]$Dir, [string]$Name)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
try {
    $claim = [IO.File]::Open((Join-Path $Dir 'started'), 'CreateNew', 'Write', 'None')
    $claim.Dispose()
} catch [IO.IOException] { return }

$output = Join-Path $Dir 'output'
function Log([string]$Line) {
    [IO.File]::AppendAllText($output, $Line + "`n", $utf8)
}
$state = 'failed'
try {
    $expected, $source = [IO.File]::ReadAllLines((Join-Path $Dir 'request'), $utf8)
    Log "File: $source"
    $entry = Get-Item -LiteralPath $source -Force
    if ($entry.PSIsContainer -or $entry.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
        throw 'Expected a regular file.'
    }
    $file = [IO.File]::Open($source, 'Open', 'Read', 'Read')
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $actual = [BitConverter]::ToString($sha.ComputeHash($file)).Replace('-', '').ToLowerInvariant() }
        finally { $sha.Dispose() }
    } finally { $file.Dispose() }
    Log "SHA-256: $actual"
    if ($expected) { Log "Expected SHA-256: $expected" }
    if ($expected -and $actual -cne $expected) { throw 'The checksum differs from the expected value.' }
    $state = 'succeeded'
} catch { Log $_.Exception.Message }

$temp = Join-Path $Dir 'runner-result'
$final = Join-Path $Dir 'result'
[IO.File]::WriteAllText($temp, $state, $utf8)
try { [IO.File]::Move($temp, $final) }
catch [IO.IOException] { if (![IO.File]::Exists($final)) { throw } }
finally { [IO.File]::Delete($temp) }
# The result is durable before removing the completed single-use scheduler entry.
Unregister-ScheduledTask -TaskName $Name -TaskPath '\' -Confirm:$false -ErrorAction SilentlyContinue
