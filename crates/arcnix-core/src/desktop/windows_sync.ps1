$ErrorActionPreference = 'Stop'
$local = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if (!$local -or ![IO.Path]::IsPathRooted($local)) { throw 'Windows profile directory is unavailable' }
$folder = Join-Path $local 'Arcnix'
$target = Join-Path $folder 'desktop.v1'
$lockPath = Join-Path $folder 'desktop.lock'

function Check-Entry($location, [bool]$directory) {
    try { $entry = Get-Item -LiteralPath $location -Force -ErrorAction Stop }
    catch [System.Management.Automation.ItemNotFoundException] { return $false }
    if ($entry.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint) -or $entry.PSIsContainer -ne $directory) {
        throw 'Unsupported entry in the Arcnix profile directory'
    }
    return $true
}
function Read-Bytes($stream, [int]$length) {
    $bytes = [byte[]]::new($length)
    $offset = 0
    while ($offset -lt $length) {
        $count = $stream.Read($bytes, $offset, $length - $offset)
        if ($count -eq 0) { throw 'Incomplete desktop sync data' }
        $offset += $count
    }
    return ,$bytes
}
function Read-Desktop {
    if (!(Check-Entry $target $false)) { return $null }
    try {
        $stream = [IO.File]::Open($target, 'Open', 'Read', ([IO.FileShare]::Read -bor [IO.FileShare]::Delete))
    } catch [IO.FileNotFoundException] { return $null }
    try {
        if ($stream.Length -gt $limit) { throw 'Remote desktop preferences exceed the size limit' }
        return ,(Read-Bytes $stream $stream.Length)
    } finally { $stream.Dispose() }
}
function Revision($bytes) {
    if ($null -eq $bytes) { return 'missing' }
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
function Respond([string]$line, $data = $null) {
    $stream = [Console]::OpenStandardOutput()
    $header = [Text.Encoding]::ASCII.GetBytes("$line" + [char]10)
    $stream.Write($header, 0, $header.Length)
    if ($null -ne $data) { $stream.Write($data, 0, $data.Length) }
    $stream.Flush()
}

if (!(Check-Entry $local $true)) { throw 'Windows profile directory is unavailable' }
if (!$commit) {
    if (!(Check-Entry $folder $true)) { Respond 'M'; return }
    $data = Read-Desktop
    if ($null -eq $data) { Respond 'M' } else { Respond 'F' $data }
    return
}

if ($length -lt 1 -or $length -gt $limit) { throw 'Invalid desktop sync data size' }
$data = Read-Bytes $inputStream $length
if ($inputStream.ReadByte() -ne -1 -or (Revision $data) -cne $next) { throw 'Invalid desktop sync data' }

if (!(Check-Entry $folder $true)) { [void][IO.Directory]::CreateDirectory($folder) }
if (!(Check-Entry $folder $true)) { throw 'Arcnix profile directory is unavailable' }
[void](Check-Entry $lockPath $false)
$lock = $null
$timer = [Diagnostics.Stopwatch]::StartNew()
while (!$lock) {
    try {
        $lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'Write', 'None')
    } catch [IO.IOException] {
        if ($timer.Elapsed.TotalSeconds -ge 5) { throw 'Desktop preferences are locked' }
        Start-Sleep -Milliseconds 50
    }
}
try {
    $current = Revision (Read-Desktop)
    if ($current -ceq $next) { Respond 'OK'; return }
    if ($current -cne $base) { Respond 'STALE'; return }
    $temp = Join-Path $folder ('.desktop-' + [Guid]::NewGuid().ToString('N'))
    $owned = $false
    try {
        $file = [IO.File]::Open($temp, 'CreateNew', 'Write', 'None')
        $owned = $true
        try {
            $file.Write($data, 0, $data.Length)
            $file.Flush($true)
        } finally { $file.Dispose() }
        if (Check-Entry $target $false) {
            [IO.File]::Replace($temp, $target, [NullString]::Value)
        } else {
            [IO.File]::Move($temp, $target)
        }
    } finally { if ($owned -and [IO.File]::Exists($temp)) { [IO.File]::Delete($temp) } }
    Respond 'OK'
} finally { $lock.Dispose() }
