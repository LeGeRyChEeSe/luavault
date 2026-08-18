# Mutation campaign for the graphical suite.
#
#   npm run e2e:mutate
#
# For each mutation: break one behaviour the suite claims to protect, rebuild,
# run only the suite that should notice, and record whether it turned red.
# A mutation that SURVIVES is a guard that guards nothing.
#
# The tree is restored with `git checkout --`, never by copying a saved file:
# `copy /Y` preserves the source's LastWriteTime, cargo then skips the rebuild
# and the *mutated* binary gets re-run against restored source (pitfall 37).
#
# Written in plain ASCII with no nested quotes inside string sub-expressions:
# npm invokes Windows PowerShell 5.1, which parses neither.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

# Each mutation names the exact behaviour it removes, and the case that must die.
$mutations = @(
    @{
        Name  = "M1 le garde 'champ de saisie' des raccourcis"
        File  = "src/lib/keyboard-shortcuts.ts"
        Old   = "  if (editable) return null;"
        New   = "  // MUTATION"
        Suite = "shortcuts"
        Kills = "un raccourci tape dans un champ de saisie est ignore"
    },
    @{
        Name  = "M2 le focus place dans le champ apres Ctrl+F"
        File  = "src/App.svelte"
        Old   = 'document.querySelector<HTMLInputElement>("[data-shortcut-search]")?.focus();'
        New   = "/* MUTATION */"
        Suite = "shortcuts"
        Kills = "Ctrl+F place le focus dans son champ"
    },
    @{
        Name  = "M3 le garde 'une modale est ouverte'"
        File  = "src/App.svelte"
        Old   = "    if (document.querySelector('[aria-modal=""true""]') !== null) return;"
        New   = "    // MUTATION"
        Suite = "shortcuts"
        Kills = "un raccourci est ignore tant qu'une modale est ouverte"
    },
    @{
        # Mutation POSITIONNELLE, et non une suppression. `{#if false && ...}`
        # retirait la banniere du rendu : c'est la forme de garde creuse n.3 de
        # ETAT.md (prouver une presence quand c'est la position qui porte
        # l'exigence). Ici la banniere reste dans le source et reste atteignable
        # en theorie, mais quand libraryError est vrai `visible` est vide, donc
        # la branche suivante gagne et la banniere n'apparait jamais. C'est
        # exactement le defaut que le LOT-21 avait livre.
        Name  = "M4 la banniere d'integrite est presente mais inatteignable"
        File  = "src/views/LibraryView.svelte"
        Old   = "  {#if appState.libraryError}"
        New   = "  {#if visible.length > 0 && appState.libraryError}"
        Suite = "integrity"
        Kills = "la banniere gagne sur la branche bibliotheque vide"
    },
    @{
        # Mutation RUST : sans elle la campagne ne couvre que le frontend et ne
        # protege pas readopt_index, dont depend le dernier cas d'integrite.
        Name  = "M6 readopt_index valide le JSON mais ne re-signe pas"
        File  = "src-tauri/src/commands.rs"
        Old   = "    hmac::sign_index(&idx, &key).map_err(|e| format!(""signature de l'index : {e}""))?;"
        New   = "    let _ = (&idx, &key); // MUTATION"
        Suite = "integrity"
        Kills = "la re-adoption confirmee rend la bibliotheque"
    },
    @{
        # Garde de l'isolation : la relecture adverse a fait survivre exactement
        # cette mutation, faute d'un cas qui observe ou l'application ecrit.
        Name  = "M7 steam_dir pointe hors du bac a sable"
        File  = "e2e/harness.ts"
        Old   = "      steam_dir: steamDir,"
        New   = "      steam_dir: join(ROOT, '.e2e', 'faux-steam-hors-bac'),"
        Suite = "isolation"
        Kills = "le dossier Steam vu par le backend est le faux"
    },
    @{
        Name  = "M5 le piege a focus du dialogue d'aide"
        File  = "src/App.svelte"
        Old   = "      use:focusTrap"
        New   = "      "
        Suite = "shell"
        Kills = "le dialogue d'aide piege le focus"
    }
)

function Write-Utf8NoBom($path, $text) {
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
}

function Rebuild {
    cmd /c "npm run build > nul 2>&1"
    if ($LASTEXITCODE -ne 0) { throw "vite build a echoue" }
    cmd /c "npm run tauri -- build --debug --no-bundle > nul 2>&1"
    if ($LASTEXITCODE -ne 0) { throw "tauri build a echoue" }
}

# A dirty tree would make `git checkout --` restore the wrong thing — it would
# roll the file back to HEAD and silently destroy uncommitted work. `e2e` is in
# the list because the campaign now mutates harness.ts too.
$dirty = git status --porcelain -- src src-tauri/src e2e
if ($dirty) {
    Write-Host "Arbre modifie sous src/ - commite ou remise d'abord :" -ForegroundColor Red
    Write-Host $dirty
    exit 2
}

Write-Host "Reference : la suite complete doit etre verte avant de muter." -ForegroundColor Cyan
Rebuild
cmd /c "npm run e2e > nul 2>&1"
if ($LASTEXITCODE -ne 0) {
    Write-Host "La suite est deja rouge - campagne annulee." -ForegroundColor Red
    exit 2
}
Write-Host "Reference verte." -ForegroundColor Green
Write-Host ""

$killed = 0
$survivors = @()

foreach ($m in $mutations) {
    Write-Host ("--- " + $m.Name) -ForegroundColor Cyan
    $path = Join-Path $root $m.File
    $text = [System.IO.File]::ReadAllText($path)
    if (-not $text.Contains($m.Old)) {
        Write-Host "    ANCRE INTROUVABLE - mutation non appliquee." -ForegroundColor Yellow
        $survivors += ($m.Name + " (ancre introuvable)")
        continue
    }
    Write-Utf8NoBom $path ($text.Replace($m.Old, $m.New))

    try {
        Rebuild
        cmd /c ("npm run e2e -- " + $m.Suite + " > nul 2>&1")
        $code = $LASTEXITCODE
        # run.ts sort en 1 pour un cas rouge et en 2 pour un prerequis manquant.
        # Les confondre creditait une mutation d'un echec d'infrastructure - et
        # avec une suite qui rougissait seule 25 % du temps, par pur hasard.
        if ($code -eq 1) {
            Write-Host ("    TUEE - la suite " + $m.Suite + " est passee au rouge.") -ForegroundColor Green
            $killed++
        } elseif ($code -eq 0) {
            Write-Host ("    SURVIVANTE - la suite reste verte sans : " + $m.Kills) -ForegroundColor Red
            $survivors += $m.Name
        } else {
            Write-Host ("    INDETERMINEE - echec d'infrastructure (code " + $code + "), mutation non comptee.") -ForegroundColor Yellow
            $survivors += ($m.Name + " (indeterminee, code " + $code + ")")
        }
    } finally {
        git checkout -- $m.File
    }
}

Write-Host ""
Write-Host ("Score de mutation : " + $killed + "/" + $mutations.Count) -ForegroundColor Yellow
if ($survivors.Count) {
    Write-Host "Survivantes :" -ForegroundColor Red
    $survivors | ForEach-Object { Write-Host ("  - " + $_) -ForegroundColor Red }
}

Write-Host ""
Write-Host "Reconstruction depuis les sources restaurees..." -ForegroundColor Cyan
Rebuild
if ($survivors.Count) { exit 1 }
