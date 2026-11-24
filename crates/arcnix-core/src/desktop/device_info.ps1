[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$processors = @(Get-CimInstance Win32_Processor -OperationTimeoutSec 5 -ErrorAction SilentlyContinue)
$system = Get-CimInstance Win32_ComputerSystem -OperationTimeoutSec 5 -ErrorAction SilentlyContinue
$os = Get-CimInstance Win32_OperatingSystem -OperationTimeoutSec 5 -ErrorAction SilentlyContinue
$graphics = Get-CimInstance Win32_VideoController -OperationTimeoutSec 5 -ErrorAction SilentlyContinue
function Field($key, $value) {
    if ($null -ne $value) { [Console]::WriteLine("$key`t$value") }
}
if ($os.Caption) { Field os $os.Caption } else { Field os 'Windows' }
Field version $os.Version
Field arch $(if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE })
foreach ($name in ($processors.Name | Select-Object -Unique)) { Field cpu $name }
Field cores ($processors | Measure-Object NumberOfCores -Sum).Sum
Field logical $system.NumberOfLogicalProcessors
Field memory $system.TotalPhysicalMemory
foreach ($adapter in $graphics) { Field gpu $adapter.Name }
