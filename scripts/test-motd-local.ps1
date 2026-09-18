# Essai du message du jour en conditions de production, sans rien publier.
#
#   npm run build:app                                   # une fois : le binaire RELEASE
#   .\scripts\test-motd-local.ps1 -Fr "…" -En "…" -Hours 2 [-Severity warning] [-File motd.md]
#   .\scripts\test-motd-local.ps1 -Fr "…" -En "…"       # sans duree : jusqu'au -Clear
#   .\scripts\test-motd-local.ps1 -Clear                # le cas « rien à dire »
#
# Ce que ce script rend fidele a la production, et pourquoi :
#   - le binaire est celui de `npm run build:app` (profil release, front embarque),
#     pas le binaire de banc ni `tauri dev` ;
#   - le message est signe par `publish-motd.ps1 -DryRun` avec la VRAIE cle primaire,
#     donc verifie par les VRAIES cles compilees dans le binaire — un octet change et
#     rien ne s'affiche, exactement comme en prod ;
#   - la seule difference est l'adresse : `LV_MOTD_BASE` (et `LV_UPDATE_BASE`) pointent
#     sur un serveur statique local qui sert releases\motd\ (et repond 404 au
#     manifeste, donc pas de fenetre de mise a jour). Ces variables sont honorees en
#     release parce que la signature, pas l'URL, est le controle.
#
# L'application tourne en mode PORTABLE dans un dossier jetable (.e2e\motd-manual\) :
# ta vraie installation, sa bibliotheque et son config.json ne sont jamais touches.
#
# -Keep garde le dossier (donc le config.json et son motd_dismissed_id) et le serveur
# en vie apres la fermeture de l'application. Relancer ensuite LA MEME commande verifie
# « Ne plus afficher » d'un lancement a l'autre : l'identifiant est derive du texte, le
# message est donc le meme (voir -Id). Sans -Keep, le dossier est efface et le choix
# avec lui.

param(
    [string]$Fr = '',
    [string]$En = '',
    [string]$TitleFr = '',
    [string]$TitleEn = '',
    [string]$File = '',
    [int]$Hours = 0,
    [int]$Days = 0,
    [ValidateSet('info', 'warning', 'critical')][string]$Severity = 'info',
    [switch]$Clear,
    # Identifiant du message. Sans lui, il est DERIVE DU TEXTE : relancer la meme
    # commande republie le meme message (donc « Ne plus afficher » tient d'un
    # lancement a l'autre), changer le texte en publie un nouveau. En production,
    # publish-motd.ps1 horodate : chaque publication est un message neuf, c'est voulu.
    [string]$Id = '',
    [int]$Port = 8765,
    [string]$Exe = '',
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

if (-not $Exe) { $Exe = Join-Path $root 'src-tauri\target\release\luavault.exe' }
if (-not (Test-Path $Exe)) {
    throw "binaire release introuvable : $Exe — lance d'abord 'npm run build:app'."
}

# 1. Le document signe, sans televersement.
$publish = @{ DryRun = $true; Severity = $Severity }
if ($Clear) { $publish.Clear = $true }
else {
    if ($File) { $publish.File = $File } else { $publish.Fr = $Fr; $publish.En = $En; $publish.TitleFr = $TitleFr; $publish.TitleEn = $TitleEn }
    $publish.Hours = $Hours; $publish.Days = $Days
    if (-not $Id) {
        $material = if ($File) { [System.IO.File]::ReadAllText((Resolve-Path $File)) } else { "$TitleFr|$TitleEn|$Fr|$En" }
        $sha = [System.Security.Cryptography.SHA256]::Create()
        $digest = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes("$Severity|$material"))
        $Id = 'local-' + (($digest[0..5] | ForEach-Object { $_.ToString('x2') }) -join '')
    }
    $publish.Id = $Id
}
& (Join-Path $PSScriptRoot 'publish-motd.ps1') @publish
if ($LASTEXITCODE -ne 0) { throw "publish-motd.ps1 -DryRun a echoue" }
$motdDir = Join-Path $root 'releases\motd'

# 2. Le bac a sable portable.
$sandbox = Join-Path $root '.e2e\motd-manual'
New-Item -ItemType Directory -Force $sandbox | Out-Null
Copy-Item $Exe (Join-Path $sandbox 'luavault.exe') -Force
New-Item -ItemType File -Force (Join-Path $sandbox 'LuaVault.portable') | Out-Null
# Locale francaise par defaut, comme le banc ; change `locale` pour voir l'anglais.
$configPath = Join-Path $sandbox 'config.json'
if (-not (Test-Path $configPath)) {
    [System.IO.File]::WriteAllText($configPath, '{ "first_run_done": true, "locale": "fr" }', (New-Object System.Text.UTF8Encoding($false)))
}

# 3. Le serveur statique local : sert motd.json + motd.json.sig, 404 pour le reste.
# Un serveur laisse par un precedent -Keep tient encore le port : on le remplace,
# sinon le nouveau meurt en silence et le PID note ci-dessous ne designe plus rien.
Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
    ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
$server = Start-Process -FilePath 'python' -ArgumentList @('-m', 'http.server', "$Port", '--bind', '127.0.0.1') `
    -WorkingDirectory $motdDir -PassThru -WindowStyle Hidden
Start-Sleep -Milliseconds 800
try {
    $probe = Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$Port/motd.json" -TimeoutSec 5
    Write-Host "serveur local : http://127.0.0.1:$Port/motd.json ($($probe.Content.Length) octets)"
}
catch {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    throw "le serveur local ne repond pas sur le port $Port : $($_.Exception.Message)"
}

# 4. L'application, pointee sur le serveur local. Tout le reste est la production.
$env:LV_MOTD_BASE = "http://127.0.0.1:$Port"
# Le manifeste aussi, pour que l'essai ne depende pas de GitHub (404 = pas de mise a jour).
$env:LV_UPDATE_BASE = "http://127.0.0.1:$Port"
Write-Host "lancement de $sandbox\luavault.exe (LV_MOTD_BASE=$env:LV_MOTD_BASE)"
$app = Start-Process -FilePath (Join-Path $sandbox 'luavault.exe') -WorkingDirectory $sandbox -PassThru
$app.WaitForExit()
Remove-Item Env:LV_MOTD_BASE
Remove-Item Env:LV_UPDATE_BASE

if ($Keep) {
    Write-Host "-Keep : serveur (PID $($server.Id)) et bac a sable conserves : $sandbox"
    Write-Host "  relance : `$env:LV_MOTD_BASE='http://127.0.0.1:$Port'; & '$sandbox\luavault.exe'"
}
else {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    Write-Host "config.json du bac a sable (motd_dismissed_id) :"
    Get-Content $configPath | Select-String 'motd_dismissed_id'
    Remove-Item $sandbox -Recurse -Force
}
