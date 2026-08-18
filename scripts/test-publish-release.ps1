# Publication-script regression test. It invokes the production script against a
# disposable fixture: no release directory, private key, lvrelease binary or GitHub
# credentials from the working tree are required.

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$publishScript = Join-Path $root 'scripts\publish-release.ps1'
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("luavault-publish-test-" + [guid]::NewGuid().ToString('N'))

function Assert-That {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw "Publication test failure: $Message" }
}

function Invoke-PublishFixtureCapture {
    param([Parameter(Mandatory)][string]$Version)

    $escapedPublishScript = $publishScript.Replace("'", "''")
    $escapedFixtureRoot = $fixtureRoot.Replace("'", "''")
    $captureCommand = @"
`$Host.UI.RawUI.BufferSize = [System.Management.Automation.Host.Size]::new(4096, `$Host.UI.RawUI.BufferSize.Height)
& '$escapedPublishScript' -Version '$Version' -ProjectRoot '$escapedFixtureRoot' -DryRun -SkipSigning
"@
    & powershell -NoProfile -ExecutionPolicy Bypass -Command $captureCommand
}

try {
    $releaseDir = Join-Path $fixtureRoot 'releases\9.9.9'
    New-Item -ItemType Directory -Force $releaseDir | Out-Null
    [System.IO.File]::WriteAllText(
        (Join-Path $fixtureRoot 'CHANGELOG.md'),
        @'
## 9.9.9 - 2026-08-12

### fr
- French isolated note.

### en
- Separate English note.

## 9.9.8 - 2026-08-11

- Legacy note.
'@,
        [System.Text.UTF8Encoding]::new($false)
    )
    [System.IO.File]::WriteAllText((Join-Path $releaseDir 'LuaVault-portable.zip'), 'fixture', [System.Text.UTF8Encoding]::new($false))

    & powershell -NoProfile -ExecutionPolicy Bypass -File $publishScript -Version '9.9.9' -ProjectRoot $fixtureRoot -DryRun -SkipSigning
    Assert-That ($LASTEXITCODE -eq 0) 'le dry-run de fixture doit réussir.'

    $manifest = Get-Content -Raw (Join-Path $releaseDir 'manifest.json') | ConvertFrom-Json
    Assert-That ($manifest.notes -ceq 'French isolated note.') 'notes fallback must contain only French.'
    Assert-That ($manifest.notes_i18n.fr -ceq 'French isolated note.') 'notes_i18n.fr must preserve French notes.'
    Assert-That ($manifest.notes_i18n.en -ceq 'Separate English note.') 'notes_i18n.en must preserve English notes.'
    Assert-That ($manifest.history.Count -eq 2) 'history must keep every fixture version.'
    Assert-That ($manifest.history[0].notes_i18n.en -ceq 'Separate English note.') 'history must keep localized notes.'
    Assert-That ($manifest.history[1].notes -ceq 'Legacy note.') 'a legacy history entry must remain readable.'
    Assert-That ($null -eq $manifest.history[1].notes_i18n) 'a legacy history entry must not invent a locale.'

    $previousErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $missingVersionOutput = Invoke-PublishFixtureCapture -Version '9.9.7' 2>&1
        $missingVersionExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    Assert-That ($missingVersionExitCode -ne 0) 'a missing changelog version must be rejected.'
    Assert-That (($missingVersionOutput | Out-String -Width 4096) -match 'aucune entr') 'the rejection must cite the changelog entry.'

    [System.IO.File]::WriteAllText(
        (Join-Path $fixtureRoot 'CHANGELOG.md'),
        @'
## 9.9.9 - 2026-08-12

### fr
- First note.

### fr
- Second note.
'@,
        [System.Text.UTF8Encoding]::new($false)
    )
    try {
        $ErrorActionPreference = 'Continue'
        $duplicateLocaleOutput = Invoke-PublishFixtureCapture -Version '9.9.9' 2>&1
        $duplicateLocaleExitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    Assert-That ($duplicateLocaleExitCode -ne 0) 'duplicate locale subsections must be rejected.'
    Assert-That (($duplicateLocaleOutput | Out-String -Width 4096) -match 'deux sous-sections ### fr') 'rejection must mention duplicate subsection.'

    # Exercise the real publication branch without contacting GitHub or generating a
    # real signature. The production script still builds the exact gh argument list.
    [System.IO.File]::WriteAllText(
        (Join-Path $fixtureRoot 'CHANGELOG.md'),
        @'
## 9.9.9 - 2026-08-12

- Draft-capable release.
'@,
        [System.Text.UTF8Encoding]::new($false)
    )
    $fixtureSigningKey = Join-Path $fixtureRoot 'fixture-signing.key'
    [System.IO.File]::WriteAllText($fixtureSigningKey, 'not-a-real-key', [System.Text.UTF8Encoding]::new($false))
    # `gh` is called once for `release create` and then once per asset for
    # `release upload` (each asset uploaded separately so a failure names the
    # exact file — see publish-release.ps1; it does NOT control the order
    # shown on the release page, GitHub sorts that alphabetically). Every
    # call is captured, not just the last one, so the create call's flags
    # can still be told apart from the upload calls that follow it.
    $global:ghCalls = @()
    function global:cargo {
        $global:LASTEXITCODE = 0
        'fixture-signature'
    }
    function global:gh {
        $global:ghCalls += , @($args)
        $global:LASTEXITCODE = 0
    }

    & $publishScript -Version '9.9.9' -ProjectRoot $fixtureRoot -SigningKey $fixtureSigningKey -Draft
    Assert-That ($LASTEXITCODE -eq 0) 'the draft publication fixture must succeed.'
    Assert-That ($global:ghCalls[0] -contains '--draft') 'a -Draft publication must pass --draft to the release create call.'
    Assert-That ($global:ghCalls.Count -gt 1) 'assets must be uploaded via separate gh calls after the release is created.'
    for ($i = 1; $i -lt $global:ghCalls.Count; $i++) {
        Assert-That ($global:ghCalls[$i][0] -ceq 'release' -and $global:ghCalls[$i][1] -ceq 'upload') "upload call $i must be a single 'gh release upload', not a bundled create."
    }

    $global:ghCalls = @()
    & $publishScript -Version '9.9.9' -ProjectRoot $fixtureRoot -SigningKey $fixtureSigningKey
    Assert-That ($LASTEXITCODE -eq 0) 'the default publication fixture must succeed.'
    Assert-That ($global:ghCalls[0] -notcontains '--draft') 'the default publication must remain non-draft.'

    Write-Host 'OK: simulated publication - bilingual notes, history, changelog guard and draft flag'
} finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
