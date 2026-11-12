use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const STEAM_INSTALLER_URL: &str = "https://cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe";
/// SteamSetup.exe measured 2,380,800 bytes on 2026-09-06; 16 MiB leaves room for
/// normal installer growth while retaining a meaningful cap for remote content.
pub const MAX_STEAM_INSTALLER_BYTES: u64 = 16 * 1024 * 1024;

fn steam_installer_temp_dir() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticket = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("LuaVault_SteamSetup_{}_{ticket}", std::process::id()))
}

/// Fetch the installer body through `update::read_capped`. Never `resp.bytes()`
/// here: that materializes the whole body before any limit can run (piège n°29),
/// and this response comes from a remote host we do not authenticate.
async fn fetch_installer_bytes(http: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>> {
    let response = http
        .get(url)
        .send()
        .await
        .context("téléchargement de SteamSetup.exe")?
        .error_for_status()
        .context("SteamSetup.exe: réponse invalide")?;
    let bytes = crate::update::read_capped(response, limit)
        .await
        .context("lecture de SteamSetup.exe")?;
    Ok(bytes)
}

/// Reserve a staging folder that no earlier invocation used. `create_dir`, not
/// `create_dir_all`, is what carries the guarantee: it fails when the path is
/// already there, so a second invocation cannot land in the first one's folder.
async fn claim_installer_dir(dir: &Path) -> Result<()> {
    tokio::fs::create_dir(dir)
        .await
        .context("création du dossier temp unique")
}

/// Write the installer, refusing to touch a file that is already there.
/// `create_new`, not `create`, is what carries the guarantee: a file planted
/// between the reservation and the write makes this an error instead of an
/// overwrite of something we did not download.
async fn write_installer(dir: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let out = dir.join("SteamSetup.exe");
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .await
        .context("création de SteamSetup.exe")?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes)
        .await
        .context("écriture de SteamSetup.exe")?;
    // A tokio file hands the bytes to a background task: `write_all` can return before they
    // reach the file, and the installer would then be launched empty or cut short.
    file.flush()
        .await
        .context("écriture de SteamSetup.exe")?;
    Ok(out)
}

async fn download_installer_from(
    http: &reqwest::Client,
    url: &str,
    limit: u64,
) -> Result<PathBuf> {
    let bytes = fetch_installer_bytes(http, url, limit).await?;
    let dir = steam_installer_temp_dir();
    claim_installer_dir(&dir).await?;
    write_installer(&dir, &bytes).await
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
                    "Steam n'a pas pu être contacté : est-il installé et lancé ?"
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
    download_installer_from(http, STEAM_INSTALLER_URL, MAX_STEAM_INSTALLER_BYTES).await
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
    use crate::update::tests::{http_ok, spawn_raw_server};
    use std::collections::HashMap;

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
    fn steam_installer_destination_is_unique_per_invocation() {
        let first = steam_installer_temp_dir();
        let second = steam_installer_temp_dir();

        assert_ne!(first, second);
        // The folder must not be a shared, predictable one: a fixed name under
        // %TEMP% is where an earlier attacker could have left their own exe.
        assert_ne!(first, std::env::temp_dir().join("LuaVault"));
        assert_ne!(first, std::env::temp_dir().join("LuaVault").join("SteamSetup.exe"));
    }

    // ------------------------------------------------------------- behavior tests
    // These call the production helpers against a local server and a real temp
    // folder. A text guard would not notice `create_dir` becoming
    // `create_dir_all`, nor `create_new` becoming `create` ; these two do.

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lv_install_test_{}_{}_{}",
            tag,
            std::process::id(),
            steam_installer_temp_dir().file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn raw_response(headers: &str, body: &[u8]) -> Vec<u8> {
        format!("HTTP/1.1 200 OK\r\n{headers}\r\n\r\n")
            .into_bytes()
            .into_iter()
            .chain(body.iter().copied())
            .collect()
    }

    fn chunked_response(chunks: &[&[u8]]) -> Vec<u8> {
        let mut out = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for chunk in chunks {
            out.extend(format!("{:x}\r\n", chunk.len()).into_bytes());
            out.extend_from_slice(chunk);
            out.extend(b"\r\n");
        }
        out.extend(b"0\r\n\r\n");
        out
    }

    #[tokio::test]
    async fn installer_download_refuses_a_response_announced_over_the_cap() {
        // Announces 20 MiB but sends almost nothing: the cap must reject it on the
        // announced length, before a single body byte is materialized. A swapped-in
        // `resp.bytes()` would accept this response and wait for the body.
        let mut routes = HashMap::new();
        routes.insert(
            "/SteamSetup.exe".to_string(),
            raw_response(
                &format!(
                    "Content-Length: {}\r\nConnection: close",
                    MAX_STEAM_INSTALLER_BYTES + 1
                ),
                b"MZ",
            ),
        );
        let base = spawn_raw_server(routes).await;
        let http = reqwest::Client::new();
        let error = download_installer_from(&http, &format!("{base}/SteamSetup.exe"), MAX_STEAM_INSTALLER_BYTES)
            .await
            .expect_err("une reponse annoncee au-dessus du plafond doit etre refusee");
        let cause = error.root_cause().to_string();
        assert!(
            cause.contains("plafond"),
            "refus attendu par taille annoncee, obtenu : {cause}"
        );
    }

    #[tokio::test]
    async fn installer_download_cuts_a_body_that_lies_about_its_length() {
        // Chunked, no Content-Length, 12 KiB actually streamed against a 4 KiB cap:
        // the refusal must come from the stream, not from the announcement.
        let big = vec![0x90u8; 4096];
        let mut routes = HashMap::new();
        routes.insert(
            "/SteamSetup.exe".to_string(),
            chunked_response(&[&big, &big, &big]),
        );
        let base = spawn_raw_server(routes).await;
        let http = reqwest::Client::new();
        let error = download_installer_from(&http, &format!("{base}/SteamSetup.exe"), 4096)
            .await
            .expect_err("12 KiB streames sous un plafond de 4 KiB doivent etre refusees");
        let cause = error.root_cause().to_string();
        assert!(
            cause.contains("plafond"),
            "refus attendu en cours de flux, obtenu : {cause}"
        );
    }

    #[tokio::test]
    async fn installer_download_writes_the_body_it_received_and_reports_its_path() {
        let body = b"MZ\x90\x00steam setup stub";
        let mut routes = HashMap::new();
        routes.insert("/SteamSetup.exe".to_string(), http_ok(body));
        let base = spawn_raw_server(routes).await;
        let http = reqwest::Client::new();
        let out = download_installer_from(&http, &format!("{base}/SteamSetup.exe"), MAX_STEAM_INSTALLER_BYTES)
            .await
            .expect("telechargement local valide");
        assert_eq!(std::fs::read(&out).unwrap(), body);
        assert_eq!(out.file_name().unwrap(), "SteamSetup.exe");
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
    }

    #[tokio::test]
    async fn installer_download_refuses_a_non_200_and_says_which_fetch_failed() {
        let base = spawn_raw_server(HashMap::new()).await; // every route answers 404
        let http = reqwest::Client::new();
        let error = download_installer_from(&http, &format!("{base}/SteamSetup.exe"), MAX_STEAM_INSTALLER_BYTES)
            .await
            .expect_err("un 404 doit etre une erreur, pas un fichier vide");
        let message = error.to_string();
        assert!(message.contains("SteamSetup.exe"), "{message}");
        assert!(!message.contains("ecriture"), "{message}");
    }

    #[tokio::test]
    async fn claim_refuses_a_folder_that_already_exists() {
        // Mutation guard for `create_dir` -> `create_dir_all` : the recursive
        // variant would silently accept an already-present folder, and two
        // invocations would then share one staging directory.
        let dir = scratch_dir("claim");
        claim_installer_dir(&dir).await.expect_err("le dossier existe deja");
        std::fs::remove_dir_all(&dir).unwrap();
        claim_installer_dir(&dir).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_refuses_to_overwrite_a_file_it_did_not_download() {
        // Mutation guard for `create_new(true)` -> `create(true)` : the plain
        // variant opens an existing file and truncates it, so a planted
        // SteamSetup.exe would be handed to `run_elevated` as if it were ours.
        let dir = scratch_dir("planted");
        let target = dir.join("SteamSetup.exe");
        std::fs::write(&target, b"not our installer").unwrap();
        let error = write_installer(&dir, b"MZ our installer")
            .await
            .expect_err("un fichier deja present ne doit pas ecrase");
        assert!(error.to_string().contains("SteamSetup.exe"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"not our installer");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_creates_the_file_when_the_folder_is_empty() {
        let dir = scratch_dir("clean");
        let out = write_installer(&dir, b"MZ payload").await.unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"MZ payload");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
