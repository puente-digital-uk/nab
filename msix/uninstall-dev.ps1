# Remove the dev-signed Nab package and stop trusting its certificate (admin prompt once).
param([string]$Cert = "dist\Nab-dev.cer")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

Get-AppxPackage -Name "PuenteDigital.Nab*" | Remove-AppxPackage -ErrorAction SilentlyContinue
if (Test-Path $Cert) {
    $thumb = (New-Object Security.Cryptography.X509Certificates.X509Certificate2 (Resolve-Path $Cert)).Thumbprint
    if (Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq $thumb) {
        Write-Host "Removing the dev certificate from Trusted People (admin prompt)..."
        Start-Process powershell -Verb RunAs -Wait -ArgumentList "-NoProfile", "-Command", "Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq '$thumb' | Remove-Item"
    }
    Get-ChildItem Cert:\CurrentUser\My | Where-Object Thumbprint -eq $thumb | Remove-Item
}
Write-Host "Done: Nab (dev) uninstalled and its certificate is no longer trusted." -ForegroundColor Green
