#Requires -RunAsAdministrator
[CmdletBinding(SupportsShouldProcess)]
param(
    [string]$HbbsServiceName = 'RustDeskStarryHBBS',
    [string]$HbbrServiceName = 'RustDeskOfficialHBBR'
)

$ErrorActionPreference = 'Stop'
$nssm = Get-Command 'nssm.exe' -ErrorAction Stop

foreach ($service in @($HbbrServiceName, $HbbsServiceName)) {
    if (Get-Service -Name $service -ErrorAction SilentlyContinue) {
        if ($PSCmdlet.ShouldProcess($service, 'Stop and remove NSSM service')) {
            & $nssm.Source stop $service | Out-Null
            & $nssm.Source remove $service confirm | Out-Null
        }
    }
}

Write-Host 'Service definitions removed. Persistent data was not deleted.'
