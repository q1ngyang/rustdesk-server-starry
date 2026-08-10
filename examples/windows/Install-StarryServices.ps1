#Requires -RunAsAdministrator
[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)]
    [string]$HbbsPath,

    [Parameter(Mandatory)]
    [string]$HbbrPath,

    [Parameter(Mandatory)]
    [string]$DataDirectory,

    [string]$HbbsServiceName = 'RustDeskStarryHBBS',
    [string]$HbbrServiceName = 'RustDeskOfficialHBBR'
)

$ErrorActionPreference = 'Stop'

$nssm = Get-Command 'nssm.exe' -ErrorAction Stop
$hbbs = (Resolve-Path -LiteralPath $HbbsPath).Path
$hbbr = (Resolve-Path -LiteralPath $HbbrPath).Path
$data = [System.IO.Path]::GetFullPath($DataDirectory)
$starryDirectory = Join-Path $data 'starry'
$config = Join-Path $starryDirectory 'config.yaml'

New-Item -ItemType Directory -Path $starryDirectory -Force | Out-Null
if (-not (Test-Path -LiteralPath $config)) {
    New-Item -ItemType File -Path $config | Out-Null
}

foreach ($service in @($HbbsServiceName, $HbbrServiceName)) {
    $existing = Get-Service -Name $service -ErrorAction SilentlyContinue
    if ($existing) {
        throw "Service '$service' already exists. Remove or rename it before continuing."
    }
}

if ($PSCmdlet.ShouldProcess($HbbsServiceName, 'Install NSSM service')) {
    & $nssm.Source install $HbbsServiceName $hbbs | Out-Null
    & $nssm.Source set $HbbsServiceName AppDirectory $data | Out-Null
    & $nssm.Source set $HbbsServiceName AppParameters "--starry-config=`"$config`"" | Out-Null
    & $nssm.Source set $HbbsServiceName AppExit Default Restart | Out-Null
    & $nssm.Source set $HbbsServiceName Start SERVICE_AUTO_START | Out-Null
}

if ($PSCmdlet.ShouldProcess($HbbrServiceName, 'Install NSSM service')) {
    & $nssm.Source install $HbbrServiceName $hbbr | Out-Null
    & $nssm.Source set $HbbrServiceName AppDirectory $data | Out-Null
    & $nssm.Source set $HbbrServiceName AppExit Default Restart | Out-Null
    & $nssm.Source set $HbbrServiceName Start SERVICE_AUTO_START | Out-Null
    & $nssm.Source set $HbbrServiceName DependOnService $HbbsServiceName | Out-Null
}

Write-Host "Created $HbbsServiceName and $HbbrServiceName."
Write-Host "Edit $config before enabling optional Starry features."
Write-Host "Review service accounts, directory ACLs, firewall rules, and logs before starting."
