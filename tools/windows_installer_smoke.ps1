param(
    [Parameter(Mandatory)] [string] $Tag,
    [string] $Artifacts = 'dist'
)

$ErrorActionPreference = 'Stop'
$stem = "arcnix-$Tag-windows-amd64"
$setup = Join-Path $Artifacts "$stem-setup.exe"
$updater = Join-Path $Artifacts "$stem.exe"
$installDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Programs\Arcnix'
$app = Join-Path $installDir 'arcnix.exe'
$uninstaller = Join-Path $installDir 'unins000.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Arcnix\Arcnix.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{CB89D5EC-A886-4760-8578-99CCCE459EA1}_is1'

foreach ($path in @($installDir, $shortcut, $uninstallKey)) {
    if (Test-Path -LiteralPath $path) {
        throw "Test requires a Windows user without an Arcnix installation: $path"
    }
}

$expectedHash = (Get-FileHash -LiteralPath $updater -Algorithm SHA256).Hash
$silent = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
$version = $Tag.Substring(1)
try {
    $process = Start-Process -FilePath (Resolve-Path -LiteralPath $setup).Path `
        -ArgumentList ($silent + '/SP-') -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Arcnix Setup failed ($($process.ExitCode))."
    }
    if ((Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash -ne $expectedHash) {
        throw 'The installed executable differs from the updater asset.'
    }
    if (-not (Test-Path -LiteralPath $shortcut)) {
        throw 'Setup did not create the Start menu shortcut.'
    }
    $registration = Get-ItemProperty -LiteralPath $uninstallKey
    if ($registration.DisplayName -ne 'Arcnix' -or $registration.DisplayVersion -ne $version) {
        throw 'Setup did not register the expected version in Windows installed apps.'
    }
    if ((& $app --version | Out-String).Trim() -ne "arcnix $version" -or $LASTEXITCODE -ne 0) {
        throw 'The installed application did not report the expected version.'
    }
} finally {
    if (Test-Path -LiteralPath $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -ArgumentList $silent -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) {
            throw "Arcnix uninstall failed ($($process.ExitCode))."
        }
    }
}

foreach ($path in @($installDir, $shortcut, $uninstallKey)) {
    if (Test-Path -LiteralPath $path) {
        throw "Uninstall left behind $path"
    }
}
Write-Output "Verified Arcnix $version installation, version, shortcut and uninstall."
