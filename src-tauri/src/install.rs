use anyhow::{Context, Result};
use std::path::PathBuf;

pub const STEAM_INSTALLER_URL: &str = "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe";
/// SteamSetup.exe measured 2,380,800 bytes on 2026-09-06; 16 MiB leaves room for
/// normal installer growth while retaining a meaningful cap for remote content.
pub const MAX_STEAM_INSTALLER_BYTES: u64 = 16 * 1024 * 1024;

fn steam_installer_temp_dir() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticket = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("LuaVault_SteamSetup_{}_{ticket}", std::process::id()))
}

#[cfg(test)]
fn function_body<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
    let function_start = source.find(signature)?;
    let block_start = function_start + source[function_start..].find('{')?;
    let mut depth = 0;
    for (offset, character) in source[block_start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[block_start + 1..block_start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
fn download_uses_capped_read(source: &str) -> bool {
    let Some(body) = function_body(
        source,
        concat!(
            "pub async fn ",
            "download_steam_installer(http: &reqwest::Client) -> Result<PathBuf>"
        ),
    ) else {
        return false;
    };
    body.contains("crate::update::read_capped")
        && body.contains("MAX_STEAM_INSTALLER_BYTES")
        && !body.contains(".bytes()")
}

/// Launch a program with the "runas" verb so Windows prompts for elevation (UAC).
pub fn run_elevated(program: &str, args: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let verb = w!("runas");
        let prog = HSTRING::from(program);
        let params = HSTRING::from(args);
        unsafe {
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(prog.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info)
                .context("élévation refusée ou échec du lancement")?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (program, args);
        Err(anyhow::anyhow!("Windows uniquement"))
    }
}

/// Like [`run_elevated`], but waits for the elevated process to exit and returns
/// its exit code. Needed when the caller must know the result (e.g. Defender
/// exclusions have to be in place *before* the files are written).
pub fn run_elevated_wait(program: &str, args: &str) -> Result<i32> {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let verb = w!("runas");
        let prog = HSTRING::from(program);
        let params = HSTRING::from(args);
        unsafe {
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(prog.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info)
                .context("élévation refusée ou échec du lancement")?;

            let handle = HANDLE(info.hProcess.0);
            if handle.0.is_null() {
                // Launched, but no handle to wait on — assume success.
                return Ok(0);
            }
            if WaitForSingleObject(handle, u32::MAX) == WAIT_OBJECT_0 {
                let mut code = 0u32;
                GetExitCodeProcess(handle, &mut code).ok();
                let _ = CloseHandle(handle);
                Ok(code as i32)
            } else {
                let _ = CloseHandle(handle);
                Ok(0)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (program, args);
        Err(anyhow::anyhow!("Windows uniquement"))
    }
}

/// Hand a `steam://` URI to the Steam client (install, run, validate…).
pub fn open_steam_uri(uri: &str) -> Result<()> {
    if !uri.starts_with("steam://") {
        return Err(anyhow::anyhow!("URI Steam invalide"));
    }
    #[cfg(windows)]
    {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let target = HSTRING::from(uri);
        unsafe {
            let result = ShellExecuteW(
                None,
                PCWSTR::null(),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            // ShellExecuteW returns a value <= 32 on failure.
            if result.0 as usize <= 32 {
                return Err(anyhow::anyhow!(
                    "Steam n'a pas pu être contacté — est-il installé et lancé ?"
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(anyhow::anyhow!("Windows uniquement"))
    }
}

/// Download the official Steam installer to a temp folder; returns its path.
pub async fn download_steam_installer(http: &reqwest::Client) -> Result<PathBuf> {
    let response = http
        .get(STEAM_INSTALLER_URL)
        .send()
        .await
        .context("téléchargement de SteamSetup.exe")?
        .error_for_status()
        .context("SteamSetup.exe: réponse invalide")?;
    let bytes = crate::update::read_capped(response, MAX_STEAM_INSTALLER_BYTES)
        .await
        .context("lecture de SteamSetup.exe")?;
    let dir = steam_installer_temp_dir();
    tokio::fs::create_dir(&dir)
        .await
        .context("création du dossier temp unique")?;
    let out = dir.join("SteamSetup.exe");
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .await
        .context("création de SteamSetup.exe")?;
    use tokio::io::AsyncWriteExt;
    file.write_all(&bytes)
        .await
        .context("écriture de SteamSetup.exe")?;
    Ok(out)
}

/// Close Steam and start it again. Not needed for `.lua` files (SteamTools reads
/// `config\lua` live) — this is a troubleshooting aid, e.g. after repairing SteamTools.
pub fn restart_steam(steam_exe: &std::path::Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        // `steam.exe -shutdown` is the graceful path; ignore failure if not running.
        let _ = std::process::Command::new(steam_exe)
            .arg("-shutdown")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|mut child| child.wait());

        std::thread::sleep(std::time::Duration::from_secs(4));
        std::process::Command::new(steam_exe)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("relancement de Steam")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = steam_exe;
        Err(anyhow::anyhow!("Windows uniquement"))
    }
}

/// The elevated command used to install/repair SteamTools (LuaVault fix script).
pub fn steamtools_command() -> (&'static str, String) {
    (
        "powershell.exe",
        "-NoProfile -ExecutionPolicy Bypass -Command \"irm -useb cdn.openlua.cloud/fix-st.ps1 | iex\"".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_installer_size_limit_is_useful() {
        const MEASURED_STEAM_INSTALLER_BYTES: u64 = 2_380_800;
        const ABSURD_STEAM_INSTALLER_BYTES: u64 = 300 * 1024 * 1024;
        let limit = std::hint::black_box(MAX_STEAM_INSTALLER_BYTES);

        assert!(
            limit >= MEASURED_STEAM_INSTALLER_BYTES * 4,
            "le plafond doit rester largement au-dessus de SteamSetup.exe mesuré"
        );
        assert!(
            limit < ABSURD_STEAM_INSTALLER_BYTES,
            "le plafond doit rester sous une taille absurde"
        );
    }

    #[test]
    fn steam_download_uses_capped_read_in_its_own_body() {
        assert!(download_uses_capped_read(include_str!("install.rs")));
    }

    #[test]
    fn steam_download_guard_rejects_a_missing_function_anchor() {
        assert!(!download_uses_capped_read("pub fn restart_steam() {}"));
    }

    #[test]
    fn steam_installer_destination_is_unique_per_invocation() {
        let first = steam_installer_temp_dir();
        let second = steam_installer_temp_dir();

        assert_ne!(first, second);
    }
}
