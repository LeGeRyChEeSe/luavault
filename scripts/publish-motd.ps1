# Publication du message du jour — s'exécute sur la machine de l'auteur, avec la
# clé privée de publication et `gh` authentifié.
#
#   .\scripts\publish-motd.ps1 -Fr "Le serveur est **en panne**." -En "The server is **down**." -Hours 12
#   .\scripts\publish-motd.ps1 -File motd.md -Days 3 -Severity warning
#   .\scripts\publish-motd.ps1 -Fr "…" -En "…"          # sans durée : affiché jusqu'à -Clear
#   .\scripts\publish-motd.ps1 -Clear
#   .\scripts\publish-motd.ps1                # sans argument : mode interactif (questions)
#   ... -DryRun          écrit et signe releases\motd\motd.json sans rien publier
#
# Le message est affiché par l'application au lancement, jusqu'à `expires_at`
# (calculé ici : maintenant + durée) — ou, sans -Hours ni -Days, jusqu'à ce que
# tu le retires avec -Clear : le cas de la panne dont on ignore la durée.
# Il est SIGNÉ avec la même clé que le manifeste : un hébergeur compromis ne
# peut pas afficher un texte que l'auteur n'a pas signé. Le client vérifie la signature sur les octets bruts, d'où
# l'écriture sans BOM.
#
# Hébergement : les deux fichiers sont les assets d'une PRÉ-RELEASE GitHub
# taguée `motd`, créée une fois et réécrite à chaque publication (`--clobber`).
# Une pré-release ne devient jamais « latest » : publier un message ne déplace
# pas le pointeur de mise à jour, et publier une version n'efface pas le message.
# Mesuré le 2026-09-18 : après un remplacement, le CDN de GitHub a servi l'ancien
# fichier pendant ~2 minutes avant le nouveau. Un client qui tombe dans cette
# fenêtre lit l'ancien document avec la nouvelle signature, le rejette, et
# n'affiche rien — jamais un message périmé tenu pour valide.
#
# Format de -File : une section par langue, la première ligne `# Titre` donne le
# titre, le reste est le corps (markdown restreint, voir
# src/lib/motd-markdown.ts pour le dialecte et les couleurs) :
#
#   ## fr
#   # Panne en cours
#   Le serveur est {red}indisponible{/} jusqu'à 18 h.
#
#   ## en
#   # Ongoing outage
#   The server is {red}unavailable{/} until 6 pm.
#
# Chaque publication porte un identifiant neuf (-Id, sinon horodatage) : c'est
# lui que mémorise « Ne plus afficher ». Republier = réafficher chez tout le monde.

[CmdletBinding()]
param(
    [string]$Fr = '',
    [string]$En = '',
    [string]$TitleFr = '',
    [string]$TitleEn = '',
    # Un fichier markdown à sections `## <langue>` ; remplace -Fr/-En/-Title*.
    [string]$File = '',
    [int]$Hours = 0,
    [int]$Days = 0,
    [ValidateSet('info', 'warning', 'critical')][string]$Severity = 'info',
    [string]$Id = '',
    # Retire le message : publie `message: null`, signé. Les assets restent en
    # place pour que le client lise un document valide plutôt qu'un 404.
    [switch]$Clear,
    [string]$SigningKey,
    [string]$Tag = 'motd',
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$dir = Join-Path $root 'releases\motd'
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

if ([string]::IsNullOrWhiteSpace($SigningKey)) {
    $SigningKey = Join-Path $root 'release-primary.key'
}
if (-not (Test-Path -LiteralPath $SigningKey -PathType Leaf)) {
    throw "Clé de signature introuvable : $SigningKey."
}

# --------------------------------------------------------------- lecture de -File
#
# Chemin absolu passé à System.IO : le répertoire courant du processus .NET n'est
# pas celui de PowerShell.
function Read-MotdFile([string]$Path) {
    $full = if ([System.IO.Path]::IsPathRooted($Path)) { $Path } else { [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Path)) }
    if (-not (Test-Path -LiteralPath $full)) { throw "fichier introuvable : $full" }
    $text = [System.IO.File]::ReadAllText($full, [System.Text.Encoding]::UTF8)

    $titles = [ordered]@{}
    $bodies = [ordered]@{}
    $lang = $null
    $buffer = @()
    $flush = {
        if ($null -eq $lang) { return }
        $lines = @($buffer)
        $firstIndex = -1
        for ($i = 0; $i -lt $lines.Count; $i++) { if ($lines[$i].Trim() -ne '') { $firstIndex = $i; break } }
        if ($firstIndex -ge 0 -and $lines[$firstIndex] -cmatch '^#\s+(.+)$') {
            $titles[$lang] = $Matches[1].Trim()
            $lines = @($lines | Select-Object -Skip ($firstIndex + 1))
        }
        $body = (($lines -join "`n").Trim())
        if ($body -ne '') { $bodies[$lang] = $body }
    }
    foreach ($line in ($text -split "`r?`n")) {
        if ($line -cmatch '^##\s+([a-z]{2,3})\s*$') {
            & $flush
            $lang = $Matches[1]
            $buffer = @()
        }
        elseif ($null -ne $lang) {
            $buffer += $line
        }
    }
    & $flush
    return @{ titles = $titles; bodies = $bodies }
}

# --------------------------------------------------------------- mode interactif
#
# Lancé sans argument, le script pose les questions une à une. Un message
# vide à une étape optionnelle passe a la suivante. Rien n'est signé ni publié
# avant l'aperçu et la confirmation finale.
function Read-MultiLine([string]$Prompt) {
    Write-Host $Prompt -ForegroundColor Cyan
    Write-Host "  (ligne vide pour terminer, laisser vide pour ignorer)" -ForegroundColor DarkGray
    $lines = @()
    while ($true) {
        $line = Read-Host
        if ($null -eq $line -or $line -eq '') { break }
        $lines += $line
    }
    return ($lines -join "`n")
}

function Read-Duration {
    while ($true) {
        $raw = (Read-Host "Durée d'affichage (ex. 12h, 3d, 2d12h ; vide = jusqu'au retrait)").Trim().ToLower()
        if ($raw -eq '') { return @{ Days = 0; Hours = 0 } }
        if ($raw -cmatch '^(?:(\d+)d)?(?:(\d+)h)?$' -and $raw -ne '') {
            $d = if ($Matches[1]) { [int]$Matches[1] } else { 0 }
            $h = if ($Matches[2]) { [int]$Matches[2] } else { 0 }
            if ($d -gt 0 -or $h -gt 0) { return @{ Days = $d; Hours = $h } }
        }
        Write-Host "  format attendu : <jours>d, <heures>h ou les deux (ex. 1d6h)." -ForegroundColor Yellow
    }
}

$interactive = (-not $Clear -and -not $File -and -not $Fr -and -not $En)
if ($interactive) {
    Write-Host ""
    Write-Host "=== Message du jour — LuaVault ===" -ForegroundColor Green
    $choice = (Read-Host "1) publier un nouveau message   2) retirer le message actuel   [1]").Trim()
    if ($choice -eq '2') {
        $Clear = $true
    }
    else {
        $TitleFr = (Read-Host "Titre (fr, optionnel)").Trim()
        $Fr = Read-MultiLine "Corps (fr) — markdown restreint : **gras**, {red}…{/}, - puces, [lien](https://…)"
        $TitleEn = (Read-Host "Titre (en, optionnel)").Trim()
        $En = Read-MultiLine "Corps (en)"
        if (-not $Fr -and -not $En) { throw "aucun corps de message saisi." }
        $sev = (Read-Host "Sévérité : info / warning / critical  [info]").Trim().ToLower()
        if ($sev -eq '') { $sev = 'info' }
        if ($sev -notin @('info', 'warning', 'critical')) { throw "sévérité inconnue : $sev" }
        $Severity = $sev
        $duration = Read-Duration
        $Days = $duration.Days
        $Hours = $duration.Hours
    }
}

# --------------------------------------------------------------- composition

if ($Clear) {
    $document = [ordered]@{ schema = 1; message = $null }
}
else {
    if ($Hours -lt 0 -or $Days -lt 0) { throw "une durée négative n'a pas de sens." }
    $openEnded = ($Hours -eq 0 -and $Days -eq 0)

    $titles = [ordered]@{}
    $bodies = [ordered]@{}
    if ($File) {
        $parsed = Read-MotdFile $File
        $titles = $parsed.titles
        $bodies = $parsed.bodies
    }
    else {
        if ($Fr) { $bodies['fr'] = $Fr }
        if ($En) { $bodies['en'] = $En }
        if ($TitleFr) { $titles['fr'] = $TitleFr }
        if ($TitleEn) { $titles['en'] = $TitleEn }
    }
    if ($bodies.Count -eq 0) {
        throw "aucun corps de message : -Fr / -En, ou -File avec une section '## fr'."
    }
    if (-not $bodies.Contains('fr') -or -not $bodies.Contains('en')) {
        Write-Warning "message publié sans 'fr' ET 'en' — les autres locales retomberont sur ce qui existe."
    }

    $now = (Get-Date).ToUniversalTime()
    if (-not $Id) { $Id = $now.ToString('yyyyMMdd-HHmmss') }

    $message = [ordered]@{
        id           = $Id
        published_at = $now.ToString('yyyy-MM-ddTHH:mm:ssZ')
    }
    # Sans durée, pas de champ du tout : le client l'entend comme « jusqu'au
    # retrait ». Un champ présent mais illisible serait ignoré, jamais éternel.
    if (-not $openEnded) {
        $expires = $now.AddDays($Days).AddHours($Hours)
        $message['expires_at'] = $expires.ToString('yyyy-MM-ddTHH:mm:ssZ')
    }
    $message['severity'] = $Severity
    $message['title'] = $titles
    $message['body'] = $bodies
    $document = [ordered]@{ schema = 1; message = $message }
}

New-Item -ItemType Directory -Force $dir | Out-Null
$docPath = Join-Path $dir 'motd.json'
$json = $document | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText($docPath, $json, $utf8NoBom)

Write-Host "message écrit : $docPath"
Write-Host $json

if ($interactive -and -not $DryRun) {
    $go = (Read-Host "Signer et publier ce document ? (o/N)").Trim().ToLower()
    if ($go -notin @('o', 'oui', 'y', 'yes')) {
        Write-Host "abandon : rien n'est signé ni publié."
        exit 0
    }
}

# --------------------------------------------------------------- signature

$signature = & cargo run --quiet --manifest-path (Join-Path $root 'src-tauri\Cargo.toml') `
    --bin lvrelease -- sign $SigningKey $docPath
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($signature)) {
    throw 'La signature Ed25519 du message a échoué.'
}
$sigPath = "$docPath.sig"
[System.IO.File]::WriteAllText($sigPath, $signature.Trim(), $utf8NoBom)
Write-Host "signature : $($signature.Trim())"

if ($DryRun) {
    Write-Host "-DryRun : rien n'est publié."
    exit 0
}

# --------------------------------------------------------------- publication

# La pré-release `motd` est créée une fois ; ensuite, seuls ses assets bougent.
& gh release view $Tag *> $null
if ($LASTEXITCODE -ne 0) {
    & gh release create $Tag --prerelease --title 'Message of the day' `
        --notes 'Signed notice shown by the application at launch. Assets are rewritten on every publication; this pre-release never becomes the latest release.'
    if ($LASTEXITCODE -ne 0) { throw "la pré-release $Tag n'a pas pu être créée." }
}

# La signature part en premier : entre les deux envois, un client lit l'ancien
# document avec la nouvelle signature et le rejette — un instant sans message,
# jamais un message dont la signature passe à tort.
foreach ($asset in @($sigPath, $docPath)) {
    & gh release upload $Tag $asset --clobber
    if ($LASTEXITCODE -ne 0) { throw "échec de l'envoi de $(Split-Path $asset -Leaf) sur la pré-release $Tag." }
}

if ($Clear) { Write-Host "message retiré." }
elseif ($openEnded) { Write-Host "publié : message $Id, sans expiration — retire-le avec -Clear." }
else { Write-Host "publié : message $Id, jusqu'au $($expires.ToString('yyyy-MM-dd HH:mm')) UTC" }
Write-Host "vérifie depuis l'extérieur :"
Write-Host "  curl -sL https://github.com/LeGeRyChEeSe/luavault/releases/download/$Tag/motd.json"
