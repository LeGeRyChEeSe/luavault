# Pre-flight validation script for LuaVault.
# Run after every code change BEFORE asking the user to test manually.
# Exit code 0 = all green, non-zero = something failed.
#
# -Quiet : print only a compact summary and, for a failing step, the lines that
#          actually matter. The full output of every step goes to
#          .orchestration/logs/validate-<step>.log instead of the console.
#          This mode exists because an agent on a small context window would
#          otherwise burn tens of thousands of tokens on cargo/vite noise —
#          which is exactly how LOT-02 hit its context limit mid-run.

param(
    [switch]$Quiet
)

$root = Split-Path $PSScriptRoot -Parent
$failed = @()
$logDir = Join-Path $root ".orchestration\logs"
if ($Quiet -and -not (Test-Path $logDir)) {
    New-Item -ItemType Directory -Force $logDir | Out-Null
}

# Lines worth surfacing in quiet mode: compiler errors, panics, failing tests.
$signal = 'error(\[E\d+\])?:|^error|panicked at|test result: FAILED|^\s+--> |assertion .*failed|^failures:|^warning: '

function Run-Step($label, $cmd, $dir, $slug) {
    if (-not $Quiet) { Write-Host "`n=== $label ===" -ForegroundColor Cyan }
    Push-Location $dir
    try {
        # Use cmd /c with 2>&1 inside the string so cmd handles the redirect
        $output = cmd /c "$cmd 2>&1"
        $stepFailed = $LASTEXITCODE -ne 0

        if ($Quiet) {
            $logFile = Join-Path $logDir "validate-$slug.log"
            $output | Set-Content -Path $logFile -Encoding utf8
            if ($stepFailed) {
                Write-Host "FAILED: $label" -ForegroundColor Red
                Write-Host "  journal complet : .orchestration/logs/validate-$slug.log"
                $hits = $output | Select-String -Pattern $signal | Select-Object -First 40
                if ($hits) {
                    Write-Host "  --- lignes significatives ---"
                    $hits | ForEach-Object { Write-Host "  $_" }
                } else {
                    Write-Host "  --- 30 dernieres lignes ---"
                    $output | Select-Object -Last 30 | ForEach-Object { Write-Host "  $_" }
                }
            } else {
                Write-Host "OK: $label" -ForegroundColor Green
            }
        } else {
            $output | ForEach-Object { Write-Host $_ }
            if ($stepFailed) {
                Write-Host "FAILED: $label (exit $LASTEXITCODE)" -ForegroundColor Red
            } else {
                Write-Host "OK: $label" -ForegroundColor Green
            }
        }

        if ($stepFailed) { $script:failed += $label }
    } finally {
        Pop-Location
    }
}

if (-not $Quiet) {
    Write-Host "LuaVault validation suite" -ForegroundColor Yellow
    Write-Host "==============================" -ForegroundColor Yellow
}

Run-Step "Rust: cargo check" "cargo check" "$root\src-tauri" "cargo-check"
Run-Step "Rust: cargo test" "cargo test" "$root\src-tauri" "cargo-test"
# Bloquant, et pas seulement informatif : dix-neuf avertissements s'etaient
# accumules sur src-tauri sans que rien ne les arrete, parce que seul le serveur
# de mise a jour passait par clippy. Un avertissement toujours tolere est un
# avertissement que plus personne ne lit.
Run-Step "Rust: cargo clippy" "cargo clippy --all-targets -- -D warnings" "$root\src-tauri" "cargo-clippy"
Run-Step "Frontend: npm run build" "npm run build" "$root" "vite-build"
Run-Step "Frontend: svelte-check" "npm run check" "$root" "svelte-check"
# Les tests unitaires frontend (virtual-scroll + tripwires structurels de
# LibraryView) — sans cette etape, rien ne rejoue scripts/test-virtual-scroll.ts
# et une fenetre virtuelle cassee passe la validation sans un signal.
Run-Step "Frontend: tests unitaires" "npx --yes tsx@4.19.2 scripts/test-virtual-scroll.ts" "$root" "ts-tests"
# La charte d'apparence, rendue executable. Elle existe pour le pipeline autonome :
# les regles qui font que cette application ressemble a elle-meme ont tenu 26 lots
# parce qu'un humain relisait chaque diff. Retirer ce relecteur du chemin critique
# demande de transformer l'interdiction en prose en procedure executable.
# Etape distincte de la precedente a dessein : son echec doit nommer la charte,
# pas se noyer dans les tests unitaires.
Run-Step "Frontend: charte d'apparence" "npx --yes tsx@4.19.2 scripts/test-charte.ts" "$root" "charte"
# Le script de publication est exécuté contre une fixture temporaire : cette étape
# couvre son parsing bilingue et sa garde changelog sans artefact ni clé de release.
Run-Step "Publication: manifeste simulé" "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-publish-release.ps1" "$root" "publish-release"
Run-Step "Frontend: audit de lignage" "npx --yes tsx@4.19.2 scripts/test-lineage.ts" "$root" "lineage"

if (-not $Quiet) { Write-Host "`n==============================" -ForegroundColor Yellow }
if ($failed.Count -gt 0) {
    Write-Host "FAILED steps:" -ForegroundColor Red
    $failed | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
} else {
    Write-Host "ALL CHECKS PASSED" -ForegroundColor Green
    exit 0
}
