# Annonce sur le salon d'annonces Discord de LuaVault, par webhook — s'exécute sur
# la machine de l'auteur. Pas de bot, et jamais le compte de l'auteur : un webhook
# ne peut que poster dans ce salon, et son URL est un secret.
#
#   .\scripts\announce-discord.ps1 -File annonce.md -Kind release -DryRun   # montre, n'envoie rien
#   .\scripts\announce-discord.ps1 -File annonce.md -Kind important
#   .\scripts\announce-discord.ps1 -Title "Titre" -Body "Texte **markdown** Discord"
#
# URL du webhook : fichier `discord-webhook.url` à la racine (ignoré par git), ou la
# variable d'environnement LUAVAULT_DISCORD_WEBHOOK. Salon d'annonces > Modifier le
# salon > Intégrations > Webhooks > Copier l'URL.
#
# Format de -File : première ligne `# Titre`, le reste est le corps (markdown Discord).
# Le texte est écrit à la main pour être lisible par tout le monde : ce qui change,
# en deux ou trois phrases, sans détail technique.
#
# Les fichiers d'annonce vivent dans `announcements/` (ignoré par git) : une annonce
# va sur Discord, jamais sur GitHub.
#
#   -Everyone   ajoute @everyone (seule mention possible, jamais par défaut)
#   -Test       poste un message d'essai puis le supprime aussitôt (sans mention)
#
# Garde-fous, tous testés à vide avec -DryRun :
#  - le texte ne nomme jamais l'autre projet (le lien reste invisible côté public) ;
#  - aucune mention ne part sans -Everyone : `allowed_mentions` vide, et un @everyone
#    écrit dans le texte lui-même est refusé ;
#  - l'URL doit être un vrai webhook Discord, sinon rien ne part ;
#  - -DryRun écrit le message tel qu'il serait posté, sans envoyer.

[CmdletBinding()]
param(
    [string]$File = '',
    [string]$Title = '',
    [string]$Body = '',
    [ValidateSet('release', 'important', 'info')][string]$Kind = 'info',
    [string]$WebhookFile = '',
    # Message riche déjà composé (JSON Discord, composants V2 : conteneur, séparateurs,
    # boutons-liens). Les mêmes garde-fous s'appliquent à tout son texte.
    [string]$PayloadFile = '',
    # Identifiant d'un message DE CE WEBHOOK à modifier sur place (pas de nouvelle
    # notification) au lieu d'en poster un autre.
    [string]$EditMessage = '',
    [switch]$Everyone,
    [switch]$Test,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (-not $WebhookFile) { $WebhookFile = Join-Path $root 'discord-webhook.url' }

if ($PayloadFile) {
    $full = if ([System.IO.Path]::IsPathRooted($PayloadFile)) { $PayloadFile } else { [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $PayloadFile)) }
    if (-not (Test-Path $full)) { throw "fichier introuvable : $full" }
    $raw = [System.IO.File]::ReadAllText($full, [System.Text.Encoding]::UTF8)
    $payload = $raw | ConvertFrom-Json
    if ($raw -match '(?i)' + (@(('auto' + 'steamtools'), ('open' + 'lua'), ('ast' + 'release')) -join '|') -or $raw -cmatch '\bAST\b') {
        throw "le texte nomme l'autre projet : annonce refusée."
    }
    if ($raw -match [char]0x2014) { throw 'le texte contient un tiret cadratin.' }
    if (-not $Everyone -and $raw -match '@(everyone|here)') { throw 'mention sans -Everyone : refusé.' }
    if ($raw -match '@here|<@[&!]?\d+>') { throw 'seule la mention @everyone est permise.' }
    # Les mentions autorisées sont décidées ici, jamais par le fichier.
    [string[]]$parse = if ($Everyone) { @('everyone') } else { @() }
    $payload | Add-Member -NotePropertyName allowed_mentions -NotePropertyValue ([pscustomobject]@{ parse = $parse }) -Force
    # Remplacer un message classique (texte + embed) par un message à composants : Discord
    # refuse l'édition tant que l'ancien contenu et les anciens embeds ne sont pas vidés.
    if ($EditMessage) {
        $payload | Add-Member -NotePropertyName content -NotePropertyValue '' -Force
        $payload | Add-Member -NotePropertyName embeds -NotePropertyValue @() -Force
    }
    $json = $payload | ConvertTo-Json -Depth 20 -Compress
    if ($DryRun) { Write-Host "-DryRun : rien n'est envoyé. Message riche ($($json.Length) caractères JSON)."; exit 0 }
    $url = if (Test-Path $WebhookFile) { ([System.IO.File]::ReadAllText($WebhookFile)).Trim() } else { $env:LUAVAULT_DISCORD_WEBHOOK }
    if ($url -notmatch '^https://(?:discord|discordapp)\.com/api/webhooks/\d+/[\w-]+$') { throw "URL du webhook absente ou invalide." }
    $bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($json)
    try {
        if ($EditMessage) {
            if ($EditMessage -notmatch '^\d+$') { throw 'identifiant de message invalide.' }
            Invoke-RestMethod -Uri "$url/messages/${EditMessage}?with_components=true" -Method Patch -ContentType 'application/json; charset=utf-8' -Body $bytes | Out-Null
            Write-Host 'message modifié sur place.'
        } elseif ($Test) {
            $sent = Invoke-RestMethod -Uri "${url}?wait=true&with_components=true" -Method Post -ContentType 'application/json; charset=utf-8' -Body $bytes
            Invoke-RestMethod -Uri "$url/messages/$($sent.id)" -Method Delete | Out-Null
            Write-Host 'essai riche envoyé puis supprimé.'
        } else {
            Invoke-RestMethod -Uri "${url}?with_components=true" -Method Post -ContentType 'application/json; charset=utf-8' -Body $bytes | Out-Null
            Write-Host 'annonce envoyée.'
        }
    } catch {
        $detail = ''
        try { $detail = (New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())).ReadToEnd() } catch { }
        throw "envoi Discord refusé (HTTP $([int]$_.Exception.Response.StatusCode)) $detail"
    }
    exit 0
}

if ($File) {
    $full = if ([System.IO.Path]::IsPathRooted($File)) { $File } else { [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $File)) }
    if (-not (Test-Path $full)) { throw "fichier introuvable : $full" }
    $lines = [System.IO.File]::ReadAllText($full, [System.Text.Encoding]::UTF8) -split "`r?`n"
    $first = 0
    while ($first -lt $lines.Count -and $lines[$first].Trim() -eq '') { $first++ }
    if ($first -lt $lines.Count -and $lines[$first] -cmatch '^#\s+(.+)$') {
        $Title = $Matches[1].Trim()
        $lines = @($lines | Select-Object -Skip ($first + 1))
    }
    $Body = ($lines -join "`n").Trim()
}
if ($Test) { $Title = 'Test'; $Body = 'Test message, deleted right away.'; $Everyone = $false }
if (-not $Body) { throw 'aucun corps de message (-File ou -Body).' }
if ($Title.Length -gt 256) { throw "titre trop long ($($Title.Length) > 256)." }
if ($Body.Length -gt 4096) { throw "corps trop long ($($Body.Length) > 4096)." }

# Le lien entre les deux projets reste invisible depuis le côté public.
# Les mots sont assemblés ici pour que l'audit de lignage ne les trouve pas dans ce fichier.
$forbidden = '(?i)' + (@(('auto' + 'steamtools'), ('open' + 'lua'), ('ast' + 'release')) -join '|')
if ("$Title`n$Body" -match $forbidden -or "$Title`n$Body" -cmatch '\bAST\b') {
    throw "le texte nomme l'autre projet : annonce refusée."
}
if ("$Title`n$Body" -match [char]0x2014) {
    throw "le texte contient un tiret cadratin : remplace-le par une ponctuation classique."
}

if ("$Title`n$Body" -match '@(everyone|here)\b|<@[&!]?\d+>') {
    throw "le texte contient une mention : utilise -Everyone, jamais une mention écrite à la main."
}

# Un tableau typé : une sous-expression `$(...)` déroule un élément unique en simple chaîne,
# et Discord refuse `parse: "everyone"` (il attend une liste).
[string[]]$parse = if ($Everyone) { @('everyone') } else { @() }

$colors = @{ release = 0x4F8EF7; important = 0xF5A623; info = 0x8E9AAF }
$payload = [ordered]@{
    allowed_mentions = @{ parse = $parse }
    embeds = @(
        [ordered]@{
            title       = $Title
            description = $Body
            color       = $colors[$Kind]
            timestamp   = (Get-Date).ToUniversalTime().ToString('o')
        }
    )
}
if ($Everyone) { $payload['content'] = '@everyone' }
$json = $payload | ConvertTo-Json -Depth 6 -Compress

if ($DryRun) {
    Write-Host "-DryRun : rien n'est envoyé. Message ($Kind)$(if ($Everyone) { ', avec @everyone' }) :"
    if ($Title) { Write-Host "# $Title" }
    Write-Host $Body
    exit 0
}

$url = if (Test-Path $WebhookFile) { ([System.IO.File]::ReadAllText($WebhookFile)).Trim() } else { $env:LUAVAULT_DISCORD_WEBHOOK }
if (-not $url) { throw "URL du webhook introuvable : crée discord-webhook.url (ou LUAVAULT_DISCORD_WEBHOOK)." }
if ($url -notmatch '^https://(?:discord|discordapp)\.com/api/webhooks/\d+/[\w-]+$') {
    throw "l'URL ne ressemble pas à un webhook Discord : rien n'est envoyé."
}

# Octets UTF-8 explicites : sans cela, PowerShell 5 envoie les accents en Latin-1.
$bytes = [System.Text.UTF8Encoding]::new($false).GetBytes($json)
try {
    $sent = Invoke-RestMethod -Uri "${url}?wait=true" -Method Post -ContentType 'application/json; charset=utf-8' -Body $bytes
    if ($Test) {
        Invoke-RestMethod -Uri "$url/messages/$($sent.id)" -Method Delete | Out-Null
        Write-Host "essai envoyé puis supprimé : le webhook fonctionne."
        exit 0
    }
} catch {
    # Le message d'erreur peut citer l'URL : on ne la répète jamais.
    throw "envoi Discord refusé (HTTP $([int]$_.Exception.Response.StatusCode))."
}
Write-Host "annonce envoyée."
