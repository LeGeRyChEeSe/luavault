# Keeps the builder's own folders out of a published executable.
#
# Rust writes the source path of every place that can panic into the binary, and the sources of
# the dependencies and of the standard library live under the builder's profile folder
# (`<profile>\.cargo\registry\...`, `<profile>\.rustup\toolchains\...`): a published executable
# carried the builder's Windows account name hundreds of times (found 2026-11-15). `trim-paths`
# is not stable in Cargo yet, so the prefix is remapped here, for release builds only.
#
# Dot-source it, call `Enable-PathRemap` before `tauri build` / `cargo build --release`, and
# `Assert-NoBuilderPath <exe>` on what was built: an executable that still names the profile
# folder is refused instead of being published. Nothing personal is written in the repository:
# the folders are read from this machine at build time.

# Cargo joins this variable with `.cargo/config.toml`'s own flags for the target (checked
# 2026-11-15): the static C runtime stays.
$script:RemapVar = 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS'

function Get-BuilderFolders {
    $profileDir = [Environment]::GetFolderPath('UserProfile').TrimEnd('\')
    $repo = (Split-Path $PSScriptRoot -Parent).TrimEnd('\')
    return @($profileDir, $repo)
}

function Enable-PathRemap {
    $profileDir, $repo = Get-BuilderFolders
    foreach ($dir in @($profileDir, $repo)) {
        # The variable is split on whitespace by Cargo: a folder with a space cannot be passed.
        if ($dir -match '\s') { throw "dossier avec un espace, chemin non retirable ainsi : $dir" }
    }
    $flags = "--remap-path-prefix=$profileDir=~ --remap-path-prefix=$repo=."
    $script:RemapBefore = [Environment]::GetEnvironmentVariable($script:RemapVar)
    Set-Item -Path "Env:$($script:RemapVar)" -Value $flags
}

function Disable-PathRemap {
    if ($null -eq $script:RemapBefore) {
        Remove-Item -Path "Env:$($script:RemapVar)" -ErrorAction SilentlyContinue
    } else {
        Set-Item -Path "Env:$($script:RemapVar)" -Value $script:RemapBefore
    }
}

function Assert-NoBuilderPath([string]$Exe) {
    if (-not (Test-Path $Exe)) { throw "executable introuvable : $Exe" }
    $profileDir = (Get-BuilderFolders)[0]
    $bytes = [IO.File]::ReadAllBytes($Exe)
    $ascii = [Text.Encoding]::ASCII.GetString($bytes)
    $wide = [Text.Encoding]::Unicode.GetString($bytes)
    foreach ($needle in @($profileDir, $profileDir.Replace('\', '/'))) {
        if ($ascii.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or
            $wide.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw "$Exe contient encore le chemin du dossier de profil ($needle) : ne pas publier"
        }
    }
    Write-Host "aucun chemin du profil dans $(Split-Path $Exe -Leaf)"
}
