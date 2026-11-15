# The stable release: `npm run build:app`. The same `tauri build` as before, with the builder's
# folders kept out of the executable (`release-paths.ps1`).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'release-paths.ps1')
Push-Location $root
Enable-PathRemap
try {
    & npx tauri build
    if ($LASTEXITCODE -ne 0) { throw "tauri build a echoue" }
    Assert-NoBuilderPath (Join-Path $root 'src-tauri\target\release\luavault.exe')
}
finally {
    Disable-PathRemap
    Pop-Location
}
