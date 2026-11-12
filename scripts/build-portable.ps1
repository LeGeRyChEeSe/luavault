# Packages the portable edition: release exe + portable marker + README, zipped.
# Run 'npm run tauri build' first so the release exe exists.
#
# -Edition experimental / beside: run right after `build-exp.ps1` (resp. `build-exp.ps1 -Beside`).
# The files carry the edition in their name (`LuaVault-Experimental…`, `LuaVault-Experimental-Beside…`):
# the client refuses an artefact named for the other channel, and `publish-release.ps1` reads the
# kind (`nsis-beside`, `portable-beside`) from the name.
param(
    [ValidateSet('stable', 'experimental', 'beside')]
    [string]$Edition = 'stable'
)
$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
# Tauri names the binary after the productName: the edition beside has its own.
$product = if ($Edition -eq 'beside') { 'LuaVault Experimental' } else { 'LuaVault' }
$prefix = switch ($Edition) {
    'experimental' { 'LuaVault-Experimental' }
    'beside' { 'LuaVault-Experimental-Beside' }
    default { 'LuaVault' }
}
$exe = Join-Path $root "src-tauri\target\release\$product.exe"
if (-not (Test-Path $exe)) {
    throw "Release exe not found at $exe — run 'npm run tauri build' first."
}

$version = (Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json).version

$outDir = Join-Path $root "dist-portable\$prefix-portable"
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

Copy-Item $exe (Join-Path $outDir "$product.exe")
# The marker switches the app to portable mode (data stored next to the exe).
New-Item -ItemType File -Path (Join-Path $outDir 'LuaVault.portable') -Force | Out-Null
Copy-Item (Join-Path $root 'README.md') (Join-Path $outDir 'README.md') -ErrorAction SilentlyContinue

# Versioned like the NSIS installer (LuaVault_<version>_x64-setup.exe) so two
# releases never collide by filename in a downloads folder.
$zip = Join-Path $root "dist-portable\$prefix-${version}-portable-win64.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $outDir '*') -DestinationPath $zip

Write-Host "Portable build ready:"
Write-Host "  folder: $outDir"
Write-Host "  zip:    $zip"

# ── Collect both artifacts into releases/<version>/ ──
$relDir = Join-Path $root "releases\$version"
New-Item -ItemType Directory -Path $relDir -Force | Out-Null

$nsis = Join-Path $root "src-tauri\target\release\bundle\nsis\${product}_${version}_x64-setup.exe"
if (Test-Path $nsis) {
    Copy-Item $nsis (Join-Path $relDir "${prefix}_${version}_x64-setup.exe") -Force
} else {
    Write-Host "WARN: NSIS installer not found at $nsis" -ForegroundColor Yellow
}
Copy-Item $zip (Join-Path $relDir (Split-Path $zip -Leaf)) -Force

Write-Host ""
Write-Host "Release $version collected:"
Get-ChildItem $relDir | ForEach-Object { Write-Host "  releases/$version/$($_.Name)" }
