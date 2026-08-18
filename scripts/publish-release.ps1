[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$')]
    [string]$Version,
    [string]$SigningKey,
    [switch]$DryRun,
    # A draft keeps the GitHub release private until it is explicitly published.
    [switch]$Draft,
    # Test hook: points the script at a disposable project fixture. Normal
    # publications always use the repository that contains this script.
    [string]$ProjectRoot,
    # A dry-run fixture has no release key or lvrelease binary. Never allow
    # this escape hatch for a real publication.
    [switch]$SkipSigning
)

$ErrorActionPreference = 'Stop'
$root = if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    Split-Path $PSScriptRoot -Parent
} else {
    [System.IO.Path]::GetFullPath($ProjectRoot)
}
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)

if ($SkipSigning -and -not $DryRun) {
    throw '-SkipSigning est réservé aux simulations -DryRun.'
}

function Get-SectionNotes {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][int]$Start,
        [Parameter(Mandatory)][int]$End
    )

    $section = $Text.Substring($Start, $End - $Start)
    # A bullet is authored across several physical lines for readability in
    # CHANGELOG.md; only the line starting with `- `/`* ` used to be kept,
    # silently dropping every wrapped continuation. A non-empty line that is
    # neither a new bullet nor a heading extends the bullet being built.
    $notes = [System.Collections.Generic.List[string]]::new()
    foreach ($line in ($section -split "`r?`n")) {
        if ($line -match '^(?:-|\*) (?<note>\S.*)$') {
            $notes.Add($Matches['note'].TrimEnd())
        } elseif ($notes.Count -gt 0 -and $line.Trim().Length -gt 0 -and $line -notmatch '^#') {
            $notes[$notes.Count - 1] = ($notes[$notes.Count - 1] + ' ' + $line.Trim()).TrimEnd()
        }
    }
    @($notes)
}

function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)

    $stream = [System.IO.File]::OpenRead($Path)
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        ([System.BitConverter]::ToString($hasher.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
        $hasher.Dispose()
        $stream.Dispose()
    }
}

function Get-SectionNoteFields {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][int]$Start,
        [Parameter(Mandatory)][int]$End
    )

    $section = $Text.Substring($Start, $End - $Start)
    $subsections = [regex]::Matches($section, '(?m)^### (?<title>.+)\s*$')
    $localized = [ordered]@{}

    for ($i = 0; $i -lt $subsections.Count; $i++) {
        $locale = $subsections[$i].Groups['title'].Value.Trim()
        if ($locale -cnotin @('fr', 'en')) {
            continue
        }
        if ($localized.Contains($locale)) {
            throw "CHANGELOG.md contient deux sous-sections ### $locale dans une même version."
        }

        $noteStart = $subsections[$i].Index + $subsections[$i].Length
        $noteEnd = if ($i + 1 -lt $subsections.Count) {
            $subsections[$i + 1].Index
        } else {
            $section.Length
        }
        $notes = Get-SectionNotes -Text $section -Start $noteStart -End $noteEnd
        if ($notes.Count -gt 0) {
            $localized[$locale] = $notes -join "`n"
        }
    }

    if ($localized.Count -gt 0) {
        # `notes` remains a backward-compatible plain-text fallback. The
        # frontend selects `notes_i18n` when it is present.
        $fallback = if ($localized.Contains('fr')) {
            $localized['fr']
        } else {
            @($localized.Values)[0]
        }
        return [pscustomobject][ordered]@{
            notes = $fallback
            notes_i18n = [pscustomobject]$localized
        }
    }

    $notes = Get-SectionNotes -Text $Text -Start $Start -End $End
    [pscustomobject][ordered]@{
        notes = ($notes -join "`n")
        notes_i18n = $null
    }
}

$changelogPath = Join-Path $root 'CHANGELOG.md'
$changelog = [System.IO.File]::ReadAllText($changelogPath)
$headings = [regex]::Matches(
    $changelog,
    '(?m)^## (?<version>\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?) - (?<date>\d{4}-\d{2}-\d{2})\s*$'
)
$targetIndex = -1
for ($i = 0; $i -lt $headings.Count; $i++) {
    if ($headings[$i].Groups['version'].Value -ceq $Version) {
        $targetIndex = $i
        break
    }
}

# This check is intentionally before release files, keys, signing and `gh`: a
# missing CHANGELOG entry must reject a publication at its earliest frontier.
if ($targetIndex -lt 0) {
    throw "Publication refusée : CHANGELOG.md ne contient aucune entrée pour la version $Version."
}

$targetHeading = $headings[$targetIndex]
$targetStart = $targetHeading.Index + $targetHeading.Length
$targetEnd = if ($targetIndex + 1 -lt $headings.Count) {
    $headings[$targetIndex + 1].Index
} else {
    $changelog.Length
}
$targetNoteFields = Get-SectionNoteFields -Text $changelog -Start $targetStart -End $targetEnd
if ([string]::IsNullOrWhiteSpace($targetNoteFields.notes)) {
    throw "Publication refusée : l'entrée CHANGELOG.md de $Version ne contient aucune puce de description (- ou *)."
}

$history = foreach ($i in 0..($headings.Count - 1)) {
    $heading = $headings[$i]
    $start = $heading.Index + $heading.Length
    $end = if ($i + 1 -lt $headings.Count) { $headings[$i + 1].Index } else { $changelog.Length }
    $noteFields = Get-SectionNoteFields -Text $changelog -Start $start -End $end
    [pscustomobject][ordered]@{
        version = $heading.Groups['version'].Value
        published_at = $heading.Groups['date'].Value + 'T00:00:00Z'
        notes = $noteFields.notes
        notes_i18n = $noteFields.notes_i18n
    }
}

$releaseDir = Join-Path $root "releases\$Version"
if (-not (Test-Path -LiteralPath $releaseDir -PathType Container)) {
    throw "Dossier des artefacts introuvable : $releaseDir. Construisez d'abord la release."
}

$artifacts = @(
    Get-ChildItem -LiteralPath $releaseDir -File |
        ForEach-Object {
            $kind = switch -Regex ($_.Name) {
                '\.exe$' { 'nsis'; break }
                '\.zip$' { 'portable'; break }
                default { $null }
            }
            if ($null -ne $kind) {
                [pscustomobject][ordered]@{
                    kind = $kind
                    file = $_.Name
                    size = [uint64]$_.Length
                    sha256 = Get-Sha256 -Path $_.FullName
                }
            }
        }
)
if ($artifacts.Count -eq 0) {
    throw "Aucun artefact .exe ou .zip à publier dans $releaseDir."
}

$manifest = [pscustomobject][ordered]@{
    schema = 1
    version = $Version
    published_at = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    notes = $targetNoteFields.notes
    notes_i18n = $targetNoteFields.notes_i18n
    history = @($history)
    artifacts = $artifacts
}
$manifestPath = Join-Path $releaseDir 'manifest.json'
[System.IO.File]::WriteAllText(
    $manifestPath,
    ($manifest | ConvertTo-Json -Depth 6 -Compress),
    $utf8NoBom
)

if (-not $SkipSigning) {
    if ([string]::IsNullOrWhiteSpace($SigningKey)) {
        $SigningKey = Join-Path $root 'release-primary.key'
    }
    if (-not (Test-Path -LiteralPath $SigningKey -PathType Leaf)) {
        throw "Clé de signature introuvable : $SigningKey."
    }

    $signature = & cargo run --quiet --manifest-path (Join-Path $root 'src-tauri\Cargo.toml') `
        --bin lvrelease -- sign $SigningKey $manifestPath
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($signature)) {
        throw 'La signature Ed25519 du manifeste a échoué.'
    }
    $signaturePath = Join-Path $releaseDir 'manifest.json.sig'
    [System.IO.File]::WriteAllText($signaturePath, $signature.Trim(), $utf8NoBom)
}

if ($DryRun) {
    $result = if ($SkipSigning) { 'manifeste créé (signature simulée)' } else { 'manifeste et signature créés' }
    Write-Host "Dry-run réussi : $result dans $releaseDir"
    exit 0
}

$gh = Get-Command gh -ErrorAction SilentlyContinue
if ($null -eq $gh) {
    throw 'GitHub CLI (gh) est requis pour publier la release.'
}

$tag = "v$Version"
# Installer, then portable, then the manifest pair below them — a fixed,
# readable order on the release page rather than filesystem enumeration order.
$assetOrder = @('nsis', 'portable')
$orderedArtifacts = foreach ($kind in $assetOrder) { $artifacts | Where-Object { $_.kind -eq $kind } }
$assets = @(
    $orderedArtifacts | ForEach-Object { Join-Path $releaseDir $_.file }
) + @($manifestPath, $signaturePath)

# The GitHub release page is public-facing and English by convention (see
# CLAUDE.md — commit messages, README, screenshots). This is independent of
# `manifest.json`'s `notes`/`notes_i18n`, which keep French as the in-app
# fallback for every existing client — only what `gh release create` shows
# changes here.
$githubNotesSource = if ($targetNoteFields.notes_i18n -and $targetNoteFields.notes_i18n.en) {
    $targetNoteFields.notes_i18n.en
} else {
    $targetNoteFields.notes
}
$releaseNotes = ($githubNotesSource -split "`n" | ForEach-Object { "- $_" }) -join "`n"

# Uploaded one asset at a time, after the release exists, so a failed upload
# names the exact file that didn't make it rather than a bundled `gh release
# create` error. This does NOT control the order shown on the release page:
# GitHub always lists assets alphabetically by filename regardless of upload
# order — measured directly against the API after a run, so don't reintroduce
# a comment claiming otherwise.
$releaseArguments = @('release', 'create', $tag, '--title', "LuaVault $Version", '--notes', $releaseNotes)
if ($Draft) {
    $releaseArguments += '--draft'
}
& gh @releaseArguments
if ($LASTEXITCODE -ne 0) {
    throw "GitHub Release $tag n'a pas pu être créée."
}

foreach ($asset in $assets) {
    & gh release upload $tag $asset
    if ($LASTEXITCODE -ne 0) {
        throw "Échec de l'envoi de l'artefact $(Split-Path $asset -Leaf) sur la release $tag."
    }
}

Write-Host "Release GitHub $tag publiée avec $($artifacts.Count) artefact(s), manifest.json et manifest.json.sig."
