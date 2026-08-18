# Menage des caches de compilation et des bacs a sable du banc.
#
# Usage : .\scripts\clean-cache.ps1 [-DryRun] [-Deps]
#
# CE QUI EST SUPPRIME PAR DEFAUT, et pourquoi c'est sans risque :
#
#   src-tauri/target/debug/incremental    (~27 Go mesures le 2026-08-10)
#   update-server/target/debug/incremental
#     Le cache de compilation incrementale de rustc. Il grossit a CHAQUE build
#     et n'est jamais purge par cargo. Le supprimer ne coute qu'une compilation
#     non incrementale des crates du projet - les dependances, elles, restent
#     dans `deps/` et ne sont PAS recompilees.
#
#   .e2e/run-*    les bacs a sable du banc graphique laisses par `npm run e2e -- --keep`
#     Chacun est un dossier portable jetable (config, bibliotheque, index). La
#     suite en cree un neuf a chaque execution ; ceux qui restent sont des restes.
#
# CE QUI N'EST PAS SUPPRIME sans -Deps :
#
#   target/debug/deps  (~21 Go) est le cache des DEPENDANCES compilees. Le vider
#     impose de recompiler webview2, rustls, argon2, zip, unrar... soit plusieurs
#     minutes. C'est un vrai cout, pas un detail : -Deps existe pour le cas ou le
#     disque manque vraiment, pas pour le menage courant.
#
# LA GARDE QUI COMPTE : le script REFUSE de tourner si un cargo/rustc est en
# cours. Supprimer `incremental/` sous les pieds d'une compilation en cours
# produit une erreur de build que personne ne rattache a un menage fait dans une
# autre fenetre - exactement le genre de faux probleme qui coute une soiree.

param(
    [switch]$DryRun,
    # Vide aussi target/debug/deps. Plusieurs minutes de recompilation ensuite.
    [switch]$Deps
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

function Get-SizeGo {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return 0 }
    $sum = (Get-ChildItem $Path -Recurse -File -ErrorAction SilentlyContinue |
        Measure-Object Length -Sum).Sum
    if ($null -eq $sum) { return 0 }
    return [math]::Round($sum / 1GB, 2)
}

# ── Garde : aucune compilation en cours ─────────────────────────────
$busy = Get-Process -Name 'cargo', 'rustc', 'tauri-driver', 'msedgedriver' -ErrorAction SilentlyContinue
if ($busy) {
    $names = ($busy | Select-Object -ExpandProperty Name -Unique) -join ', '
    Write-Error "Compilation ou banc en cours ($names). Le menage supprimerait des fichiers utilises. Reessaie apres."
    exit 1
}

# ── Cibles ──────────────────────────────────────────────────────────
$targets = @(
    (Join-Path $root 'src-tauri\target\debug\incremental'),
    (Join-Path $root 'update-server\target\debug\incremental')
)
if ($Deps) {
    $targets += (Join-Path $root 'src-tauri\target\debug\deps')
    $targets += (Join-Path $root 'update-server\target\debug\deps')
}
$targets += (Get-ChildItem (Join-Path $root '.e2e') -Directory -Filter 'run-*' -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty FullName)

$total = 0
foreach ($path in $targets) {
    if (-not (Test-Path $path)) { continue }
    $size = Get-SizeGo $path
    $total += $size
    $shown = $path.Replace("$root\", '')
    if ($DryRun) {
        Write-Host ("  a supprimer : {0,-45} {1,7:N2} Go" -f $shown, $size)
    }
    else {
        Remove-Item $path -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ("  supprime    : {0,-45} {1,7:N2} Go" -f $shown, $size)
    }
}

$verbe = if ($DryRun) { 'Liberables' } else { 'Liberes' }
Write-Host ("{0} : {1:N2} Go" -f $verbe, $total)
if (-not $Deps) {
    Write-Host "target/debug/deps est conserve (cache des dependances). -Deps pour le vider aussi."
}
