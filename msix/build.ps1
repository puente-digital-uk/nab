# Build Nab.msix from a release nab.exe. Run from the repo root, after .\build.ps1:
#
#   .\msix\build.ps1                       # for the Store (unsigned; Partner Center signs it)
#   .\msix\build.ps1 -DevSign              # self-signed, to install and test locally (see install-dev.ps1)
#
# Partner Center > Product identity gives the real values for -IdentityName, -Publisher and
# -PublisherDisplay. Without them the package has placeholder identity and can only be used for
# local testing.
param(
    [string]$IdentityName = "PuenteDigital.Nab",
    [string]$Publisher = "CN=Puente Digital (dev)",
    [string]$PublisherDisplay = "Puente Digital",
    [string]$Exe = "target\release\nab.exe",
    [string]$Out = "dist\Nab.msix",
    [switch]$DevSign
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not (Test-Path $Exe)) { throw "No $Exe - run .\build.ps1 first." }

# Version: Cargo.toml's x.y.z becomes x.y.z.0 (the Store requires the 4th part to be 0).
$cargo = Get-Content Cargo.toml -Raw
if ($cargo -notmatch '(?m)^version = "(\d+)\.(\d+)\.(\d+)"') { throw "Can't read the version from Cargo.toml" }
$version = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"

# Windows SDK tools (any installed SDK; the newest wins).
function Find-SdkTool([string]$name) {
    $hits = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\$name" -ErrorAction SilentlyContinue | Sort-Object FullName
    if (-not $hits) { throw "$name not found. Install the Windows 10/11 SDK (Visual Studio Installer > Individual components > Windows SDK)." }
    $hits[-1].FullName
}
$makeappx = Find-SdkTool "makeappx.exe"
$makepri = Find-SdkTool "makepri.exe"

# Stage the package contents.
$stage = Join-Path $PSScriptRoot ".pkg"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $stage "Assets") | Out-Null
Copy-Item $Exe (Join-Path $stage "nab.exe")
Copy-Item (Join-Path $PSScriptRoot "Assets\*.png") (Join-Path $stage "Assets")
$manifest = Get-Content (Join-Path $PSScriptRoot "AppxManifest.xml") -Raw
$manifest = $manifest.Replace("{{IDENTITY_NAME}}", $IdentityName).Replace("{{PUBLISHER}}", $Publisher).Replace("{{PUBLISHER_DISPLAY}}", $PublisherDisplay).Replace("{{VERSION}}", $version)
[IO.File]::WriteAllText((Join-Path $stage "AppxManifest.xml"), $manifest, (New-Object Text.UTF8Encoding $false))

# Resource index, so Assets\X.png resolves to X.scale-100.png / X.scale-200.png etc.
& $makepri createconfig /cf (Join-Path $stage "priconfig.xml") /dq en-GB /pv 10.0.0 /o | Out-Null
if ($LASTEXITCODE) { throw "makepri createconfig failed" }
& $makepri new /pr $stage /cf (Join-Path $stage "priconfig.xml") /of (Join-Path $stage "resources.pri") /mn (Join-Path $stage "AppxManifest.xml") /o | Out-Null
if ($LASTEXITCODE) { throw "makepri new failed" }
Remove-Item (Join-Path $stage "priconfig.xml")

# Pack.
$outDir = Split-Path $Out -Parent
if ($outDir -and -not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
if (Test-Path $Out) { Remove-Item $Out -Force }
& $makeappx pack /d $stage /p $Out /o | Out-Null
if ($LASTEXITCODE) { throw "makeappx failed" }

if ($DevSign) {
    # A throwaway certificate whose subject matches the manifest's Publisher (MSIX insists on that).
    $signtool = Find-SdkTool "signtool.exe"
    $cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq $Publisher -and $_.FriendlyName -eq "Nab dev signing" } | Select-Object -First 1
    if (-not $cert) {
        $cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher -FriendlyName "Nab dev signing" `
            -KeyUsage DigitalSignature -CertStoreLocation Cert:\CurrentUser\My `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    }
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $Out | Out-Null
    if ($LASTEXITCODE) { throw "signtool failed" }
    Export-Certificate -Cert $cert -FilePath (Join-Path $outDir "Nab-dev.cer") | Out-Null
    Write-Host "Signed with a dev certificate. To install it locally: .\msix\install-dev.ps1 (asks for admin once, to trust the certificate)."
}

$size = [math]::Round((Get-Item $Out).Length / 1KB)
Write-Host "OK  $Out  ($size KB)  version $version  identity $IdentityName  publisher $Publisher" -ForegroundColor Green
