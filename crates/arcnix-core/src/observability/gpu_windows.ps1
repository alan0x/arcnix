[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$ErrorActionPreference = 'SilentlyContinue'
$registry = @{}
foreach ($a in Get-ChildItem 'HKLM:\SOFTWARE\Microsoft\DirectX' | Get-ItemProperty) {
    if ($null -ne $a.AdapterLuid) { $registry[[string]$a.AdapterLuid] = $a }
}
$controllers = @(Get-CimInstance Win32_VideoController)
$cards = @{}
function Card($id) {
    if (!$cards.ContainsKey($id)) {
        $null = $id -match 'luid_0x([0-9a-f]+)_0x([0-9a-f]+)'
        $a = $registry[[string][Convert]::ToUInt64(($Matches[1]+$Matches[2]),16)]
        $c = @($controllers | Where-Object Name -eq $a.Description)
        $g = @{id=$id;name=$a.Description; dedicated_total=$a.DedicatedVideoMemory;shared_total=$a.SharedSystemMemory}
        if ($c.Count -eq 1) { $g.driver=$c[0].DriverVersion }
        $cards[$id] = $g
    }
    $cards[$id]
}
foreach ($m in Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUAdapterMemory) {
    $g = Card $m.Name
    $g.dedicated = $m.DedicatedUsage
    $g.shared = $m.SharedUsage
}
$engines = @{}
foreach ($e in Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine) {
    if ($e.Name -match '(luid_0x[0-9a-f]+_0x[0-9a-f]+_phys_\d+)_eng_(\d+)_engtype_(.+)$') {
        $id,$n,$type = $Matches[1],$Matches[2],$Matches[3]
        $key = "$id/$n/$type"
        $engines[$key] += [double]$e.UtilizationPercentage
    }
}
# Windows reports the busiest physical engine as adapter utilization.
foreach ($key in $engines.Keys) {
    $id,$n,$type = $key.Split('/')
    $g = Card $id
    $v = [Math]::Min(100.0,$engines[$key])
    $g.utilization = [Math]::Max([double]$g.utilization,$v)
    $field = @{ '3D'='graphics'; Copy='copy'; VideoEncode='encode'; VideoDecode='decode' }[$type]
    if ($field) { $g[$field] = [Math]::Max([double]$g[$field],$v) }
}
foreach ($c in $controllers) {
    if ($cards.Values.name -notcontains $c.Name) {
        $cards[$c.PNPDeviceID] = @{id=$c.PNPDeviceID;name=$c.Name;driver=$c.DriverVersion}
    }
}
foreach ($g in $cards.Values) {
    if ($g.name -or $g.utilization -or $g.dedicated -or $g.shared) {
        "GPU" + [char]9 + ($g | ConvertTo-Json -Compress)
    }
}
if (Get-Command nvidia-smi -CommandType Application) {
    nvidia-smi --query-gpu=pci.bus_id,name,driver_version,utilization.gpu,utilization.encoder,utilization.decoder,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits 2>$null | ForEach-Object { "NV" + [char]9 + $_ }
}

exit 0
