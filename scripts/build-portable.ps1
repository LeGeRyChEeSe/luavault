# Packages the portable edition: release exe + portable marker + README, zipped.
# Run 'npm run tauri build' first so the release exe exists.
$ErrorActionPreference = 'Stop'

$root = Split-Path $PSScriptRoot -Parent
# Cargo names the binary after the package (lowercase); ship it with a pretty name.
$exe = Join-Path $root 'src-tauri\target\release\LuaVault.exe'
if (-not (Test-Path $exe)) {
    throw "Release exe not found at $exe — run 'npm run tauri build' first."
}

$version = (Get-Content (Join-Path $root 'package.json') -Raw | ConvertFrom-Json).version

$outDir = Join-Path $root 'dist-portable\LuaVault-portable'
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir -Force | Out-Null

Copy-Item $exe (Join-Path $outDir 'LuaVault.exe')
# The marker switches the app to portable mode (data stored next to the exe).
New-Item -ItemType File -Path (Join-Path $outDir 'LuaVault.portable') -Force | Out-Null
Copy-Item (Join-Path $root 'README.md') (Join-Path $outDir 'README.md') -ErrorAction SilentlyContinue

# Versioned like the NSIS installer (LuaVault_<version>_x64-setup.exe) so two
# releases never collide by filename in a downloads folder.
$zip = Join-Path $root "dist-portable\LuaVault-${version}-portable-win64.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $outDir '*') -DestinationPath $zip

Write-Host "Portable build ready:"
Write-Host "  folder: $outDir"
Write-Host "  zip:    $zip"

# ── Collect both artifacts into releases/<version>/ ──
$relDir = Join-Path $root "releases\$version"
New-Item -ItemType Directory -Path $relDir -Force | Out-Null

$nsis = Join-Path $root "src-tauri\target\release\bundle\nsis\LuaVault_${version}_x64-setup.exe"
if (Test-Path $nsis) {
    Copy-Item $nsis (Join-Path $relDir (Split-Path $nsis -Leaf)) -Force
} else {
    Write-Host "WARN: NSIS installer not found at $nsis" -ForegroundColor Yellow
}
Copy-Item $zip (Join-Path $relDir (Split-Path $zip -Leaf)) -Force

Write-Host ""
Write-Host "Release $version collected:"
Get-ChildItem $relDir | ForEach-Object { Write-Host "  releases/$version/$($_.Name)" }
