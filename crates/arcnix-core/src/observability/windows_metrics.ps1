$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$sample = @{
    uptime_seconds = $null
    memory = $null
    cpu = $null
    networks = @()
    disks = @()
    warnings = @()
}
try {
    $os = Get-CimInstance -ClassName Win32_OperatingSystem
    $sample.uptime_seconds = ([DateTime]::UtcNow - $os.LastBootUpTime.ToUniversalTime()).TotalSeconds
    $sample.memory = @{
        total_bytes = [uint64]$os.TotalVisibleMemorySize * 1024
        available_bytes = [uint64]$os.FreePhysicalMemory * 1024
    }
} catch {
    $sample.warnings += 'Windows uptime and memory are unavailable'
}
try {
    $cpu = Get-CimInstance -ClassName Win32_PerfRawData_PerfOS_Processor -Filter "Name='_Total'"
    if ($null -ne $cpu) {
        # PERF_100NSEC_TIMER_INV uses the change in PercentProcessorTime as idle time.
        $sample.cpu = @{
            total_ticks = $cpu.Timestamp_Sys100NS
            idle_ticks = $cpu.PercentProcessorTime
            logical_cpus = [Environment]::ProcessorCount
        }
    }
} catch {
    $sample.warnings += 'Windows CPU performance counters could not be read'
}
try {
    $sample.networks = @(Get-NetAdapterStatistics | ForEach-Object {
        if ($_.Name -and $null -ne $_.ReceivedBytes -and $null -ne $_.SentBytes) {
            @{ name = $_.Name; received_bytes = $_.ReceivedBytes; transmitted_bytes = $_.SentBytes }
        }
    })
} catch {
    $sample.warnings += 'Windows network counters are unavailable'
}
try {
    $sample.disks = @(Get-CimInstance -ClassName Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object {
        if ($null -ne $_.Size -and $null -ne $_.FreeSpace) {
            @{ drive = $_.DeviceID; total_bytes = $_.Size; available_bytes = $_.FreeSpace }
        }
    })
} catch {
    $sample.warnings += 'Windows disk usage is unavailable'
}
[Console]::WriteLine('ARCNIX_WINDOWS_METRICS_V1')
[Console]::WriteLine(($sample | ConvertTo-Json -Compress -Depth 4))
