$ErrorActionPreference='Stop'
$docker=Get-Command docker.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if ($docker) { $exe=$docker.Path }
else {
    $exe=Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Docker\Docker\resources\bin\docker.exe'
    if (![IO.File]::Exists($exe)) { throw 'Docker is no longer installed' }
}
foreach ($name in 'DOCKER_CONTEXT','DOCKER_HOST','DOCKER_TLS_VERIFY','DOCKER_CERT_PATH') {
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
function Run-Docker([string]$arguments, [bool]$merge) {
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$exe
    $start.Arguments='--host npipe:////./pipe/docker_engine ' + $arguments
    $start.UseShellExecute=$false
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    $p=[Diagnostics.Process]::Start($start)
    try {
        $timer=[Diagnostics.Stopwatch]::StartNew()
        $out=[Console]::OpenStandardOutput()
        $err=[Console]::OpenStandardError()
        if ($merge) { $out=[IO.Stream]::Synchronized($out); $err=$out }
        $copy=@($p.StandardOutput.BaseStream.CopyToAsync($out), $p.StandardError.BaseStream.CopyToAsync($err))
        if (!$p.WaitForExit(25000)) { $p.Kill(); exit 124 }
        if (![Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$copy, [Math]::Max(0, 25000 - [int]$timer.ElapsedMilliseconds))) { exit 124 }
        $out.Flush(); $err.Flush()
        if ($p.ExitCode -ne 0) { exit $p.ExitCode }
    } finally { $p.Dispose() }
}
