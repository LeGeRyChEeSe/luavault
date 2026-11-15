# Builds an experimental edition of LuaVault from the same commit as the stable.
#
#   .\scripts\build-exp.ps1            # the edition that REPLACES the stable (npm run build:app:exp)
#   .\scripts\build-exp.ps1 -Beside    # the edition installed BESIDE the stable
#
# Three things make the edition, and they all start here:
#   1. the Cargo feature `experimental` (config::EXPERIMENTAL = true), or `beside`, which implies it
#      and gives the edition its own data folder and registrations (config::BESIDE);
#   2. for the edition beside only, the override `src-tauri/tauri.exp.conf.json` (productName,
#      identifier, window title) passed with `--config`, which Tauri merges with tauri.conf.json:
#      NSIS installs it in its own folder under its own uninstall entry;
#   3. VITE_LV_EDITION=experimental for the frontend (vite.config.js freezes it), run by the
#      beforeBuildCommand of `tauri build`, which inherits this environment.
#
# The edition that replaces the stable keeps the stable's name and identifier: a stable that
# switches to the experimental channel installs it over itself (same folder, same data), and it goes
# back to the stable by itself when the experimental channel is withdrawn or a newer stable comes out
# (update::stable_takes_over).
#
# Cargo has one target/: build the experimental editions first, each followed by its
# `build-portable.ps1 -Edition`, and the stable last.
param(
    [switch]$Beside
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$expConf = Join-Path $root 'src-tauri\tauri.exp.conf.json'
if (-not (Test-Path $expConf)) { throw "surcharge introuvable : $expConf" }

$env:VITE_LV_EDITION = 'experimental'
# The builder's folders stay out of the executable (`release-paths.ps1`).
. (Join-Path $PSScriptRoot 'release-paths.ps1')
$exe = Join-Path $root 'src-tauri\target\release\luavault.exe'
Push-Location $root
try {
    if ($Beside) {
        Enable-PathRemap
        & npx tauri build --config $expConf --features beside
        if ($LASTEXITCODE -ne 0) { throw "tauri build (experimental, a cote) a echoue" }
        Assert-NoBuilderPath $exe
        Write-Host "release experimentale (a cote) construite : lance maintenant 'build-portable.ps1 -Edition beside'."
    }
    else {
        Enable-PathRemap
        & npx tauri build --features experimental
        if ($LASTEXITCODE -ne 0) { throw "tauri build (experimental) a echoue" }
        Assert-NoBuilderPath $exe
        Write-Host "release experimentale construite : lance maintenant 'build-portable.ps1 -Edition experimental'."
    }
}
finally {
    Disable-PathRemap
    Pop-Location
    Remove-Item Env:\VITE_LV_EDITION -ErrorAction SilentlyContinue
}
