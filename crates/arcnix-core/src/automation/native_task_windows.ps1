$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = [Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
$local = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if (!$local -or ![IO.Path]::IsPathRooted($local)) { throw 'Windows profile directory is unavailable' }
$app = Join-Path $local 'Arcnix'
$root = Join-Path $app 'native-tasks-v1'
if ($expectedRoot -and $root -cne $expectedRoot) { throw 'Server task root changed' }
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()

function Entry([string]$Path, [bool]$Directory) {
    try { $item = Get-Item -LiteralPath $Path -Force }
    catch [System.Management.Automation.ItemNotFoundException] { return $null }
    if ($item.PSIsContainer -ne $Directory -or $item.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
        throw 'Unexpected entry in the server task directory'
    }
    return $item
}
function B64([string]$Value) { [Convert]::ToBase64String($utf8.GetBytes($Value)) }
function Header { [Console]::WriteLine("ARCNIX_NATIVE_TASKS_V1`t$now`t$(B64 $root)") }
function PowerShellExe { Join-Path $PSHOME 'powershell.exe' }
function JobName([string]$Id) { "Arcnix-NativeTask-$Id" }
function JobArgs([string]$Dir, [string]$Name) {
    $body = $utf8.GetString([Convert]::FromBase64String($runner))
    $path = B64 $Dir
    $script = "& {`n$body`n} -Dir ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$path'))) -Name '$Name'"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    return '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + $encoded
}
function OwnedJob([string]$Dir, [string]$Id) {
    $name = JobName $Id
    $job = Get-ScheduledTask -TaskName $name -TaskPath '\' -ErrorAction SilentlyContinue
    if (!$job) { return $null }
    $argsFile = Entry (Join-Path $Dir 'jobargs') $false
    if (!$argsFile -or $argsFile.Length -gt 16384) { throw 'Missing server task action identity' }
    $arguments = [IO.File]::ReadAllText($argsFile.FullName, $utf8)
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = $job.Principal.UserId
    if (@($job.Actions).Count -ne 1 -or $job.Actions.Execute -ine (PowerShellExe) -or
        $job.Actions.Arguments -cne $arguments -or
        ($principal -ine $identity.Name -and $principal -ine $identity.User.Value)) {
        throw 'Unexpected scheduler identity for the server task'
    }
    return $job
}
function ValidRecord([string]$Dir) {
    (Entry $Dir $true) -and (Entry (Join-Path $Dir 'request') $false) -and (Entry (Join-Path $Dir 'output') $false)
}
function ConfirmExisting([string]$Dir, [string]$Wanted) {
    if (!(ValidRecord $Dir)) { throw 'The task record is still being initialized' }
    $existing = Entry (Join-Path $Dir 'request') $false
    if ($existing.Length -gt 16384 -or [IO.File]::ReadAllText($existing.FullName, $utf8) -cne $Wanted) {
        throw 'The task ID already belongs to another request'
    }
    [Console]::WriteLine('OK')
}
function Row([string]$Dir, [string]$Id) {
    $request = Entry (Join-Path $Dir 'request') $false
    $created = ([DateTimeOffset]$request.CreationTimeUtc).ToUnixTimeSeconds()
    $output = Entry (Join-Path $Dir 'output') $false
    $bytes = $output.Length
    $final = Entry (Join-Path $Dir 'result') $false
    if ($final) {
        $state = 'unknown'
        if ($final.Length -le 16) {
            $value = [IO.File]::ReadAllText($final.FullName, $utf8)
            if ($value -cin @('succeeded', 'failed', 'cancelled')) { $state = $value }
        }
    } else {
        $state = 'unknown'
        try {
            $job = OwnedJob $Dir $Id
            if ($job -and ($job.State -in @('Running', 'Queued') -or
                (!(Entry (Join-Path $Dir 'started') $false) -and ($now - $created) -lt 30))) {
                $state = 'running'
            } else { $state = 'interrupted' }
        } catch {}
    }
    [Console]::WriteLine("$Id`t$created`t$bytes`t$state")
}

[void](Entry $local $true)
[void](Entry $app $true)
$exists = Entry $root $true
switch ($op) {
    'list' {
        Header
        $more = 0
        if ($exists) {
            $rows = @(Get-ChildItem -LiteralPath $root -Directory -Force | Where-Object {
                $_.Name -cmatch '^verify-[0-9a-f]{32}$' -and (ValidRecord $_.FullName)
            } | Sort-Object CreationTimeUtc, Name -Descending | Select-Object -Skip $offset -First 101)
            if ($rows.Count -gt 100) { $more = 1 }
            $rows | Select-Object -First 100 | ForEach-Object { Row $_.FullName $_.Name }
        }
        [Console]::WriteLine("END`t$more")
    }
    'detail' {
        $dir = Join-Path $root $id
        if (!$exists -or !(ValidRecord $dir)) { throw 'Server task record was not found' }
        Header
        Row $dir $id
        $stream = [IO.File]::Open((Join-Path $dir 'output'), 'Open', 'Read', 'ReadWrite')
        try {
            $count = [int][Math]::Min(16384, $stream.Length)
            [void]$stream.Seek(-$count, 'End')
            $reader = [IO.BinaryReader]::new($stream)
            [Console]::WriteLine([Convert]::ToBase64String($reader.ReadBytes($count)))
        } finally { $stream.Dispose() }
    }
    'submit' {
        $dir = Join-Path $root $id
        $wanted = "$digest`n$source`n"
        if ($exists -and (Entry $dir $true)) {
            ConfirmExisting $dir $wanted
            return
        }
        $entry = Entry $source $false
        if (!$entry) { throw 'Select a regular file on the host' }
        [void][IO.Directory]::CreateDirectory($root)
        [void](Entry $app $true)
        [void](Entry $root $true)
        $name = JobName $id
        $arguments = JobArgs $dir $name
        if ($arguments.Length -gt 16384) { throw 'The scheduled task action is too long' }
        # Move a complete private staging directory into place without replacing an existing record.
        $stage = Join-Path $root ('.stage-' + [Guid]::NewGuid().ToString('N'))
        [void][IO.Directory]::CreateDirectory($stage)
        try {
            [IO.File]::WriteAllText((Join-Path $stage 'output'), '', $utf8)
            [IO.File]::WriteAllText((Join-Path $stage 'request'), $wanted, $utf8)
            [IO.File]::WriteAllText((Join-Path $stage 'jobargs'), $arguments, $utf8)
            try { [IO.Directory]::Move($stage, $dir) } catch [IO.IOException] {
                if (!(Entry $dir $true)) { throw }
                ConfirmExisting $dir $wanted
                return
            }
        } finally { if ([IO.Directory]::Exists($stage)) { [IO.Directory]::Delete($stage, $true) } }
        $account = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $principal = New-ScheduledTaskPrincipal -UserId $account -LogonType S4U -RunLevel Limited
        $action = New-ScheduledTaskAction -Execute (PowerShellExe) -Argument $arguments
        $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 24) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $name -TaskPath '\' -Action $action -Principal $principal -Settings $settings | Out-Null
        Start-ScheduledTask -TaskName $name -TaskPath '\'
        [Console]::WriteLine('OK')
    }
    'stop' {
        $dir = Join-Path $root $id
        if (!$exists -or !(ValidRecord $dir)) { throw 'Server task record was not found' }
        $final = Join-Path $dir 'result'
        if (!(Entry $final $false)) {
            if (!(OwnedJob $dir $id)) {
                if (!(Entry $final $false)) { throw 'The reviewed task is no longer available to stop' }
                [Console]::WriteLine('OK')
                return
            }
            Stop-ScheduledTask -TaskName (JobName $id) -TaskPath '\'
            $deadline = [Diagnostics.Stopwatch]::StartNew()
            do {
                $job = OwnedJob $dir $id
                if (!$job -or $job.State -notin @('Running', 'Queued')) { break }
                if ($deadline.Elapsed.TotalSeconds -ge 5) { throw 'The scheduler has not confirmed the stop yet' }
                Start-Sleep -Milliseconds 100
            } while ($true)
            if ($job) { Unregister-ScheduledTask -TaskName (JobName $id) -TaskPath '\' -Confirm:$false }
            $temp = Join-Path $dir ('.cancel-' + [Guid]::NewGuid().ToString('N'))
            [IO.File]::WriteAllText($temp, 'cancelled', $utf8)
            try { [IO.File]::Move($temp, $final) }
            catch [IO.IOException] { if (!(Entry $final $false)) { throw } }
            finally { [IO.File]::Delete($temp) }
        }
        [Console]::WriteLine('OK')
    }
    default { throw 'Invalid server task operation' }
}
