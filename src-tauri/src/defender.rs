//! Windows Defender exclusions, managed transparently and with consent.
//!
//! Online-fix patches are modified game binaries: Defender flags them the moment
//! they are written to disk. The fix archive is extracted *inside* the game's
//! own folder, so a single exclusion over the Steam games folders
//! (`steamapps\common`, across every library) covers every current and future
//! install — no per-game rule and no `%TEMP%` rule. The app offers to add it
//! once, right after the licence, behind a UAC prompt.
//!
//! Reading the exclusion list back needs admin (`Get-MpPreference`.ExclusionPath
//! is restricted), so it can't be verified without elevation. We therefore trust
//! the recorded choice: once the user accepted, installs proceed on that basis.
//!
//! This is not evasion: an exclusion added here is a normal, visible Windows
//! Defender setting the user approves through a UAC prompt, and it only covers
//! Windows Defender. A third-party antivirus is out of reach and will still
//! react.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// What the UI needs to decide whether exclusions are even possible/useful.
#[derive(Debug, Clone, Serialize, Default)]
pub struct DefenderStatus {
    /// False when Defender cannot be queried.
    pub available: bool,
    /// False when Defender is present but a third-party antivirus owns real-time
    /// protection. An exclusion added to Defender would then protect nothing.
    pub active: bool,
}

/// Detect Defender without elevation. `Get-MpComputerStatus` answers for a
/// normal user; the exclusion *list* does not (it needs admin). Its running
/// mode still tells us whether a Defender exclusion can be useful.
pub fn status() -> DefenderStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-MpComputerStatus -ErrorAction Stop | Select-Object AMRunningMode | ConvertTo-Json -Compress",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        match output {
            Ok(out) if out.status.success() => DefenderStatus {
                available: true,
                active: active_from_status_json(&String::from_utf8_lossy(&out.stdout)),
            },
            _ => DefenderStatus::default(),
        }
    }
    #[cfg(not(windows))]
    {
        DefenderStatus::default()
    }
}

/// Treat the missing field and malformed legacy output as active. Older
/// Defender versions did not expose `AMRunningMode`; they must keep the same
/// availability behaviour as before this distinction was introduced.
fn active_from_status_json(raw: &str) -> bool {
    #[derive(Deserialize)]
    struct MpComputerStatus {
        #[serde(rename = "AMRunningMode")]
        am_running_mode: Option<String>,
    }

    serde_json::from_str::<MpComputerStatus>(raw)
        .ok()
        .and_then(|status| status.am_running_mode)
        .map(|mode| mode == "Normal")
        .unwrap_or(true)
}

#[cfg(test)]
fn status_uses_active_parser(source: &str) -> bool {
    let Some(function_start) = source.find("pub fn status() -> DefenderStatus") else {
        return false;
    };
    let Some(block_start) = source[function_start..].find('{') else {
        return false;
    };
    let block_start = function_start + block_start;
    let mut depth = 0;
    for (offset, character) in source[block_start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let body = &source[block_start + 1..block_start + offset];
                    return body.contains("active: active_from_status_json(");
                }
            }
            _ => {}
        }
    }
    false
}

/// Add path exclusions through an elevated PowerShell, waiting for it to finish
/// so the caller can write files immediately afterwards. Each path is quoted so
/// spaces are safe; embedded single quotes are doubled per PowerShell rules.
pub fn add_exclusions(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let list = paths
        .iter()
        .map(|p| format!("'{}'", p.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!("Add-MpPreference -ExclusionPath {list} -ErrorAction Stop");
    let args = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \"{script}\""
    );

    let code = crate::install::run_elevated_wait("powershell.exe", &args)
        .context("ajout des exclusions Defender")?;
    if code != 0 {
        bail!("Windows Defender a refusé l'ajout des exclusions (code {code}).");
    }
    Ok(())
}

/// What an elevated verify-and-repair found and did.
#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyReport {
    /// Required folders already excluded in the correct form.
    pub already_present: Vec<String>,
    /// Required folders that were missing and got added.
    pub added: Vec<String>,
    /// Malformed/duplicate exclusions that pointed at a required folder and got
    /// removed (e.g. a lowercase forward-slash variant of the same path).
    pub removed: Vec<String>,
}

/// Elevated verify-and-repair. Reading Defender's exclusion list requires admin,
/// so this runs a small elevated PowerShell that reads the list, works out which
/// of `required` are missing (case- and separator-insensitively), adds them, and
/// writes a JSON result back to a temp file we then read.
pub fn verify_and_fix(required: &[String]) -> Result<VerifyReport> {
    if required.is_empty() {
        return Ok(VerifyReport::default());
    }

    let dir = std::env::temp_dir().join("LuaVault");
    std::fs::create_dir_all(&dir).context("création du dossier temp")?;
    let pid = std::process::id();
    let script_path = dir.join(format!("defender_verify_{pid}.ps1"));
    let result_path = dir.join(format!("defender_result_{pid}.json"));
    let _ = std::fs::remove_file(&result_path);

    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let array = required.iter().map(|p| quote(p)).collect::<Vec<_>>().join(",");
    let result_literal = quote(&result_path.display().to_string());

    // `ConvertTo-Json` collapses single-element arrays to scalars, so the reader
    // below accepts either shape. The result is written with .NET's WriteAllText
    // (UTF-8, *no* BOM) — `Set-Content -Encoding UTF8` would prepend a BOM that
    // breaks serde_json.
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$required = @({array})
$resultFile = {result_literal}
try {{
  $existing = @()
  $ep = (Get-MpPreference).ExclusionPath
  if ($ep) {{ $existing = @($ep) }}
  function Norm([string]$p) {{ return ($p -replace '/','\').TrimEnd('\').ToLower() }}

  # Map each required folder (normalized) to its canonical spelling.
  $requiredNorm = @{{}}
  foreach ($r in $required) {{ $requiredNorm[(Norm $r)] = $r }}

  $present  = New-Object System.Collections.Generic.List[string]
  $toAdd    = New-Object System.Collections.Generic.List[string]
  $toRemove = New-Object System.Collections.Generic.List[string]
  $coveredClean = @{{}}

  foreach ($e in $existing) {{
    $ne = Norm $e
    if ($requiredNorm.ContainsKey($ne)) {{
      $canonical = $requiredNorm[$ne]
      if ($e -ceq $canonical) {{
        # Same folder, same spelling: a clean rule we keep.
        if (-not $coveredClean.ContainsKey($ne)) {{ $present.Add($canonical) }}
        $coveredClean[$ne] = $true
      }} else {{
        # Same folder, different spelling (lowercase / forward slashes): a
        # malformed duplicate — drop it in favour of the canonical one.
        $toRemove.Add($e)
      }}
    }}
  }}
  foreach ($r in $required) {{
    if (-not $coveredClean.ContainsKey((Norm $r))) {{ $toAdd.Add($r) }}
  }}

  if ($toRemove.Count -gt 0) {{ Remove-MpPreference -ExclusionPath $toRemove.ToArray() -ErrorAction Stop }}
  if ($toAdd.Count -gt 0)    {{ Add-MpPreference    -ExclusionPath $toAdd.ToArray()    -ErrorAction Stop }}

  $out = @{{ already_present = $present.ToArray(); added = $toAdd.ToArray(); removed = $toRemove.ToArray() }} | ConvertTo-Json -Compress
  [System.IO.File]::WriteAllText($resultFile, $out)
  exit 0
}} catch {{
  [System.IO.File]::WriteAllText($resultFile, (@{{ error = $_.Exception.Message }} | ConvertTo-Json -Compress))
  exit 1
}}
"#
    );

    std::fs::write(&script_path, script).context("écriture du script de vérification")?;
    let args = format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\"",
        script_path.display()
    );
    let code = crate::install::run_elevated_wait("powershell.exe", &args)
        .context("vérification des exclusions Defender");

    let raw = std::fs::read_to_string(&result_path).ok();
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&result_path);
    code?;
    let raw = raw.context("résultat de vérification introuvable")?;
    // Defensive: drop a UTF-8 BOM if one slipped in anyway.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);

    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        already_present: Option<serde_json::Value>,
        #[serde(default)]
        added: Option<serde_json::Value>,
        #[serde(default)]
        removed: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    }
    fn to_vec(value: Option<serde_json::Value>) -> Vec<String> {
        match value {
            Some(serde_json::Value::Array(items)) => items
                .into_iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect(),
            Some(serde_json::Value::String(single)) => vec![single],
            _ => Vec::new(),
        }
    }

    let parsed: Raw = serde_json::from_str(raw).context("résultat de vérification illisible")?;
    if let Some(message) = parsed.error {
        bail!("Windows Defender a signalé : {message}");
    }
    Ok(VerifyReport {
        already_present: to_vec(parsed.already_present),
        added: to_vec(parsed.added),
        removed: to_vec(parsed.removed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_never_panics_and_reports_availability() {
        // Off-Windows (CI) this is simply "not available"; on Windows it queries
        // the real Defender. Either way it must not panic.
        let _ = status();
    }

    #[test]
    fn non_normal_running_mode_means_defender_is_passive() {
        assert!(active_from_status_json(r#"{"AMRunningMode":"Normal"}"#));
        assert!(!active_from_status_json(r#"{"AMRunningMode":"Passive Mode"}"#));
        assert!(!active_from_status_json(r#"{"AMRunningMode":"SxS Passive Mode"}"#));
        assert!(active_from_status_json("{}"));
    }

    #[test]
    fn status_wires_active_to_the_status_parser() {
        assert!(status_uses_active_parser(include_str!("defender.rs")));
    }

    #[test]
    fn add_exclusions_with_nothing_to_do_is_a_noop() {
        assert!(add_exclusions(&[]).is_ok());
    }
}
