# Build + test Nab on Windows. Usage:  .\build.ps1
# Needs Rust (https://rustup.rs) with the default MSVC toolchain.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

Write-Host "`n== Lint ==" -ForegroundColor Cyan
cargo clippy --locked --all-targets -- -D warnings
if ($LASTEXITCODE) { throw "clippy failed" }

Write-Host "`n== Tests (unit + real OCR, clipboard, registry) ==" -ForegroundColor Cyan
cargo test --locked -- --test-threads=1 --nocapture
if ($LASTEXITCODE) { throw "tests failed" }

Write-Host "`n== Release build ==" -ForegroundColor Cyan
cargo build --locked --release
if ($LASTEXITCODE) { throw "build failed" }

$exe = Get-Item target\release\nab.exe
Write-Host ("`nOK  {0}  ({1:N0} KB)" -f $exe.FullName, ($exe.Length / 1KB)) -ForegroundColor Green
Write-Host "Run it, press Alt+Space, drag over some text, then paste."
