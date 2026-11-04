//! Work that must outlive the app, then bring the app back.
//!
//! Two gestures close the window on purpose and want it reopened afterwards: the
//! silent update (the NSIS installer cannot replace a running exe) and the SteamTools
//! repair (the repair script closes Steam and kills the app with it). Both hand the
//! job to a small hidden PowerShell that waits for this process to be gone, does the
//! work, then starts the app again.
//!
//! **Nothing dynamic is ever spliced into the script text.** The scripts are
//! constants; paths and the pid travel as environment variables of the helper.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Where the helper leaves the outcome of the SteamTools repair for the next launch.
pub fn repair_result_path() -> PathBuf {
    crate::config::data_dir().join("steamtools_repair.json")
}

/// Wait for the app, run the installer silently, start the app again whatever
/// happened (an update that failed must not leave the user without the app).
const UPDATE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
Wait-Process -Id ([int]$env:LV_WAIT_PID) -Timeout 60 -ErrorAction SilentlyContinue
# From a notification there is no window to close: an instance left in the tray is
# stopped here, the app reopens at the end and starts its background service again.
if ($env:LV_STOP_OTHERS -eq '1') {
  $name = [System.IO.Path]::GetFileNameWithoutExtension($env:LV_EXE)
  Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne [int]$env:LV_WAIT_PID } | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 1
}
# The process is gone, but Windows (or an antivirus scan of the exiting process) can
# hold the exe a moment longer: an installer started inside that window skips it
# silently and still exits 0. Measured on the real chain, so wait until it opens.
for ($i = 0; $i -lt 60; $i++) {
  try {
    $f = [System.IO.File]::Open($env:LV_EXE, 'Open', 'ReadWrite', 'None'); $f.Close(); break
  } catch { Start-Sleep -Milliseconds 500 }
}
function Get-Sig { try { (Get-FileHash -LiteralPath $env:LV_EXE -Algorithm SHA256).Hash } catch { '' } }
$before = Get-Sig
$code = -1
for ($attempt = 0; $attempt -lt 3; $attempt++) {
  try {
    $p = Start-Process -FilePath $env:LV_INSTALLER -ArgumentList '/S' -Wait -PassThru
    $code = $p.ExitCode
  } catch { }
  # A silent installer that could not replace the exe still exits 0: judge by the exe.
  if ($code -ne 0 -or (Get-Sig) -ne $before) { break }
  Start-Sleep -Seconds 2
}
try {
  Add-Content -LiteralPath (Join-Path $env:LV_DATA_DIR 'update-helper.log') -Value ((Get-Date -Format s) + ' installer exit ' + $code)
} catch { }
# A background update (nobody asked, nobody is looking) must not pop a window open:
# the app comes back the way it was, hidden in the tray, or not at all.
switch ($env:LV_RELAUNCH) {
  'none' { }
  'background' { Start-Process -FilePath $env:LV_EXE -ArgumentList '--background' }
  default { Start-Process -FilePath $env:LV_EXE }
}
"#;

/// Wait for the app, run the repair script elevated and wait for it, write what
/// happened, start the app again. A refused UAC prompt is an outcome, not a crash.
const REPAIR_SCRIPT: &str = r#"
$ErrorActionPreference = 'Continue'
Wait-Process -Id ([int]$env:LV_WAIT_PID) -Timeout 30 -ErrorAction SilentlyContinue
$code = 1
$refused = $false
try {
  $null = Start-Process -FilePath 'powershell.exe' -ArgumentList $env:LV_ST_ARGS -Verb RunAs -Wait -PassThru -ErrorAction Stop
  # The script ends with a bare `exit` whether it worked or not: its exit code says
  # nothing. It prints "Process completed" only when every step went through.
  $done = $false
  try { $done = (Get-Content -LiteralPath $env:LV_ST_LOG -Raw -ErrorAction Stop) -match 'Process completed' } catch { }
  if ($done) { $code = 0 }
} catch { $refused = $true }
try {
  $json = '{"exit_code":' + $code + ',"refused":' + $refused.ToString().ToLower() + '}'
  [System.IO.File]::WriteAllText($env:LV_RESULT, $json, (New-Object System.Text.UTF8Encoding $false))
} catch { }
Start-Process -FilePath $env:LV_EXE
"#;

fn spawn_helper(script: &str, env: &[(&str, String)]) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

        let exe = std::env::current_exe().context("chemin de l'exécutable courant")?;
        let mut cmd = std::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", script])
            .env("LV_WAIT_PID", std::process::id().to_string())
            .env("LV_EXE", exe)
            .env("LV_DATA_DIR", crate::config::data_dir())
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.spawn().context("lancement de l'assistant de relance")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (script, env);
        Err(anyhow::anyhow!("Windows uniquement"))
    }
}

/// What the helper does once the installer is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relaunch {
    /// Open the app with its window (a person asked for the update).
    Window,
    /// Open it hidden, as the tray service (an update nobody was watching, on a
    /// machine where the service was running).
    Background,
    /// Leave it closed (an update nobody was watching, nothing was running).
    Nothing,
}

impl Relaunch {
    fn word(self) -> &'static str {
        match self {
            Relaunch::Window => "window",
            Relaunch::Background => "background",
            Relaunch::Nothing => "none",
        }
    }
}

/// Install `installer` silently once this process has exited, then bring the app
/// back as `relaunch` says. The caller exits right after this returns `Ok`.
///
/// `stop_others`: also stop every other running instance of the app first (the
/// headless paths, once they have checked that none of them has a window).
pub fn update_then_relaunch(installer: &Path, stop_others: bool, relaunch: Relaunch) -> Result<()> {
    spawn_helper(
        UPDATE_SCRIPT,
        &[
            ("LV_INSTALLER", installer.display().to_string()),
            ("LV_STOP_OTHERS", if stop_others { "1" } else { "0" }.to_string()),
            ("LV_RELAUNCH", relaunch.word().to_string()),
        ],
    )
}

/// The command line of the elevated PowerShell. It runs the repair script, fetched
/// the way it always was, with ONE change: each of its « Press any key » waits (a
/// `ReadKey`) becomes a six-second pause, so the window closes by itself and the app
/// can come back. Everything is transcribed to `log`, where the verdict is read.
/// Nothing from outside is spliced in but `log`, quoted for a PowerShell string.
pub fn repair_command_line(log: &Path) -> String {
    let log = log.display().to_string().replace('\'', "''");
    format!(
        "-NoProfile -ExecutionPolicy Bypass -Command \"Start-Transcript -Path '{log}' -Force | Out-Null; \
         $s = irm -useb cdn.openlua.cloud/fix-st.ps1; \
         $s = $s -replace '\\$Host\\.UI\\.RawUI\\.ReadKey\\([^)]*\\)','Start-Sleep -Seconds 6'; \
         iex $s\""
    )
}

/// Run the SteamTools repair script elevated once this process has exited, record
/// how it went, then reopen the app. The caller exits right after `Ok`.
pub fn repair_steamtools_then_relaunch() -> Result<()> {
    let _ = std::fs::remove_file(repair_result_path());
    let log = crate::config::data_dir().join("steamtools_repair.log");
    let _ = std::fs::remove_file(&log);
    spawn_helper(
        REPAIR_SCRIPT,
        &[
            ("LV_ST_ARGS", repair_command_line(&log)),
            ("LV_ST_LOG", log.display().to_string()),
            ("LV_RESULT", repair_result_path().display().to_string()),
        ],
    )
}

/// What the helper wrote about the repair.
#[derive(Debug, Clone, Deserialize)]
struct RawRepair {
    exit_code: i32,
    refused: bool,
}

/// The verdict shown at the next launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepairOutcome {
    pub ok: bool,
    /// The user declined the elevation prompt.
    pub refused: bool,
    pub exit_code: i32,
    /// SteamTools' DLLs are in place now (read from disk, not from the script).
    pub installed: bool,
}

/// Pure: the helper's JSON plus what the disk says now. Success needs all three:
/// the prompt accepted, the script ending cleanly, and the markers there. The exit
/// code alone is not trusted (an `iex`'d script can end 0 having done nothing), nor
/// are the markers alone (a repair of an already-installed tool would always pass).
pub fn judge_repair(json: &str, installed: bool) -> Option<RepairOutcome> {
    let raw: RawRepair = serde_json::from_str(json).ok()?;
    Some(RepairOutcome {
        ok: !raw.refused && raw.exit_code == 0 && installed,
        refused: raw.refused,
        exit_code: raw.exit_code,
        installed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_run_with_the_markers_in_place_is_a_success() {
        let o = judge_repair(r#"{"exit_code":0,"refused":false}"#, true).unwrap();
        assert!(o.ok && !o.refused);
    }

    #[test]
    fn a_refused_prompt_is_a_failure_even_if_the_markers_were_already_there() {
        let o = judge_repair(r#"{"exit_code":0,"refused":true}"#, true).unwrap();
        assert!(!o.ok && o.refused);
    }

    #[test]
    fn a_non_zero_exit_is_a_failure() {
        let o = judge_repair(r#"{"exit_code":1,"refused":false}"#, true).unwrap();
        assert!(!o.ok);
    }

    #[test]
    fn missing_markers_are_a_failure_even_with_exit_zero() {
        let o = judge_repair(r#"{"exit_code":0,"refused":false}"#, false).unwrap();
        assert!(!o.ok);
    }

    #[test]
    fn an_unreadable_file_says_nothing() {
        assert!(judge_repair("not json", true).is_none());
        assert!(judge_repair("", true).is_none());
    }

    /// The scripts are constants: whatever varies must come through the environment.
    #[test]
    fn scripts_read_every_variable_part_from_the_environment() {
        for s in [UPDATE_SCRIPT, REPAIR_SCRIPT] {
            assert!(s.contains("$env:LV_EXE") && s.contains("$env:LV_WAIT_PID"));
        }
        assert!(UPDATE_SCRIPT.contains("$env:LV_INSTALLER"));
        assert!(UPDATE_SCRIPT.contains("$env:LV_RELAUNCH"));
        // The three words the Rust side sends are the three the script switches on.
        for r in [Relaunch::Window, Relaunch::Background, Relaunch::Nothing] {
            let w = r.word();
            assert!(w == "window" || UPDATE_SCRIPT.contains(&format!("'{w}'")), "{w}");
        }
        assert!(REPAIR_SCRIPT.contains("$env:LV_ST_ARGS") && REPAIR_SCRIPT.contains("$env:LV_RESULT"));
        assert!(REPAIR_SCRIPT.contains("$env:LV_ST_LOG"));
    }

    /// The only edit made to the repair script is its keypress wait, and a log path
    /// with an apostrophe must not break out of its quotes.
    #[test]
    fn the_repair_command_replaces_the_key_wait_and_quotes_the_log() {
        let c = repair_command_line(Path::new("C:\\Users\\O'Brien\\x.log"));
        assert!(c.contains("cdn.openlua.cloud/fix-st.ps1"));
        assert!(c.contains("ReadKey") && c.contains("Start-Sleep"));
        assert!(c.contains("O''Brien") && !c.contains("O'Brien"), "{c}");
        assert!(c.contains("Start-Transcript"));
    }
}
