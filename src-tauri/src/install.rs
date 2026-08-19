use anyhow::{Context, Result};
use std::path::PathBuf;

pub const STEAM_INSTALLER_URL: &str = "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe";

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
    let bytes = http
        .get(STEAM_INSTALLER_URL)
        .send()
        .await
        .context("téléchargement de SteamSetup.exe")?
        .error_for_status()
        .context("SteamSetup.exe: réponse invalide")?
        .bytes()
        .await
        .context("lecture de SteamSetup.exe")?;
    let dir = std::env::temp_dir().join("LuaVault");
    tokio::fs::create_dir_all(&dir).await.context("création du dossier temp")?;
    let out = dir.join("SteamSetup.exe");
    tokio::fs::write(&out, &bytes)
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
