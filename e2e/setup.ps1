# One-time setup for the graphical end-to-end suite.
#
#   npm run e2e:setup
#
# Installs tauri-driver (the WebDriver shim that launches a Tauri app) and
# downloads the msedgedriver whose version matches the *installed* WebView2
# runtime. That match is not a detail: msedgedriver refuses to drive a runtime
# of a different build, and the failure it reports names neither version.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$binDir = Join-Path $root ".e2e\bin"

Write-Host "1/2  tauri-driver" -ForegroundColor Cyan
if (Get-Command tauri-driver -ErrorAction SilentlyContinue) {
    Write-Host "     deja installe" -ForegroundColor Green
} else {
    cargo install tauri-driver --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo install tauri-driver a echoue" }
}

Write-Host "2/2  msedgedriver" -ForegroundColor Cyan

# The WebView2 runtime registers its version under EdgeUpdate. Both hives are
# checked: the per-machine install writes to WOW6432Node, a per-user one does not.
$clientId = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$version = $null
foreach ($hive in @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$clientId",
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$clientId",
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$clientId"
)) {
    try {
        $pv = (Get-ItemProperty $hive -ErrorAction Stop).pv
        if ($pv) { $version = $pv; break }
    } catch { }
}
if (-not $version) { throw "Runtime WebView2 introuvable dans le registre - est-il installe ?" }
Write-Host "     runtime WebView2 : $version"

$existing = Join-Path $binDir "msedgedriver.exe"
if (Test-Path $existing) {
    $have = (& $existing --version) -replace '^\D+', '' -replace ' .*$', ''
    if ($have -eq $version) {
        Write-Host "     msedgedriver $have deja present" -ForegroundColor Green
        exit 0
    }
    Write-Host "     msedgedriver $have ne correspond pas, remplacement"
}

New-Item -ItemType Directory -Force $binDir | Out-Null
$zip = Join-Path $env:TEMP "edgedriver_$version.zip"
$url = "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip"
Write-Host "     telechargement $url"
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
Expand-Archive $zip -DestinationPath $binDir -Force
Remove-Item $zip -Force

$got = & (Join-Path $binDir "msedgedriver.exe") --version
Write-Host "     $got" -ForegroundColor Green
Write-Host ""
Write-Host "Pret. Construisez le binaire de test puis lancez la suite :" -ForegroundColor Yellow
Write-Host "  npm run e2e:build"
Write-Host "  npm run e2e"
