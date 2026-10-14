# Install a dev-signed Nab.msix (from .\msix\build.ps1 -DevSign) on this PC for testing.
# Trusting the dev certificate needs admin once; the install itself doesn't.
param(
    [string]$Package = "dist\Nab.msix",
    [string]$Cert = "dist\Nab-dev.cer"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$thumb = (New-Object Security.Cryptography.X509Certificates.X509Certificate2 (Resolve-Path $Cert)).Thumbprint
if (-not (Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq $thumb)) {
    Write-Host "Trusting the dev certificate (admin prompt)..."
    Start-Process powershell -Verb RunAs -Wait -ArgumentList "-NoProfile", "-Command", "Import-Certificate -FilePath '$(Resolve-Path $Cert)' -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null"
}

# Replace any previous dev install, then add the new one.
Get-AppxPackage -Name "PuenteDigital.Nab*" | Remove-AppxPackage -ErrorAction SilentlyContinue
Add-AppxPackage -Path (Resolve-Path $Package)
Write-Host "Installed. Find Nab in Start. Check: Settings > Apps > Startup lists Nab once you tick 'Start with Windows'." -ForegroundColor Green
Write-Host "Remove it again with:  Get-AppxPackage PuenteDigital.Nab* | Remove-AppxPackage"
