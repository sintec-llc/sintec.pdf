//! Asks GitHub for the latest Sintec.PDF release (Help ▸ Check for updates, issue #28) and, when
//! the user chooses Update, downloads its installer, checks it against the release's SHA-256
//! list and starts it.

use std::time::Duration;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release};

const LATEST: &str = "https://api.github.com/repos/sintec-llc/sintec.pdf/releases/latest";
/// Every release download lives under this prefix; nothing else is ever downloaded.
const DOWNLOADS: &str = "https://github.com/sintec-llc/sintec.pdf/releases/download/";
/// The largest installer accepted.
const MAX_INSTALLER: u64 = 512 << 20;

fn agent(timeout: Duration) -> Result<ureq::Agent, String> {
    Ok(ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent())
}

/// The latest release. The answer is untrusted: its size is capped, only a page under
/// [`RELEASES_PAGE`] is ever offered for download (anything else falls back to that list), and
/// only assets under [`DOWNLOADS`] are kept.
pub fn latest_release() -> Result<Release, String> {
    let mut response = match agent(Duration::from_secs(10))?
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("Sintec.PDF/", env!("CARGO_PKG_VERSION")))
        .call()
    {
        Ok(r) => r,
        // GitHub answers 404 while no release is published (drafts don't count): nothing newer
        // than this build, so it is up to date rather than a connection problem.
        Err(ureq::Error::StatusCode(404)) => return Ok(no_release_yet()),
        Err(e) => return Err(format!("couldn't reach GitHub ({e})")),
    };
    let body = response.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| format!("unreadable answer ({e})"))?;
    parse(&body)
}

/// The certificate authorities the operating system trusts.
fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted certificates found on this system".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// What "no release published yet" means to the update check: this very version.
fn no_release_yet() -> Release {
    Release { version: concat!("v", env!("CARGO_PKG_VERSION")).to_string(), url: RELEASES_PAGE.to_string(), ..Default::default() }
}

/// The installer name suffix for this machine (`-windows-x64.msi`).
fn installer_suffix() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x64",
    };
    format!("-windows-{arch}.msi")
}

fn parse(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("unreadable answer ({e})"))?;
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).ok_or("no release found")?.to_string();
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.strip_prefix(RELEASES_PAGE).is_some_and(|rest| rest.starts_with('/') && !rest.contains(['?', '#', '\\'])))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    let asset = |pred: &dyn Fn(&str) -> bool| -> Option<String> {
        v["assets"].as_array()?.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            let link = a["browser_download_url"].as_str()?;
            (pred(name) && link.strip_prefix(DOWNLOADS).is_some_and(|rest| !rest.contains(['?', '#', '\\', ' ']) && rest.ends_with(name)))
                .then(|| link.to_string())
        })
    };
    let suffix = installer_suffix();
    let installer = asset(&|n: &str| n.starts_with("sintec-pdf-") && n.ends_with(suffix.as_str()));
    let checksums = asset(&|n: &str| n == "SHA256SUMS.txt");
    Ok(Release { version, url, installer, checksums })
}

/// The checksum `SHA256SUMS.txt` lists for `name` (`<hex>  <name>` per line).
fn listed_sha256(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = agent(Duration::from_secs(600))?
        .get(url)
        .header("User-Agent", concat!("Sintec.PDF/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("download failed ({e})"))?;
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("download failed ({e})"))
}

/// Update: download the release's installer, check it against the release's SHA-256 list, and
/// start it so that it installs once this app has closed and then restarts it.
pub fn install_update(release: &Release) -> Result<(), String> {
    launch_installer(&fetch_installer(release)?)
}

/// Download the release's installer and check it against the release's SHA-256 list; returns
/// where the checked installer was saved.
pub fn fetch_installer(release: &Release) -> Result<std::path::PathBuf, String> {
    let installer = release.installer.as_deref().ok_or("this release has no installer for this computer")?;
    let sums_url = release.checksums.as_deref().ok_or("this release has no checksum list, so its installer can't be checked")?;
    let name = installer.rsplit('/').next().filter(|n| !n.is_empty()).ok_or("bad installer link")?;
    let sums = String::from_utf8_lossy(&download(sums_url, 64 << 10)?).into_owned();
    let expected = listed_sha256(&sums, name).ok_or("the installer isn't in the release's checksum list")?;
    let bytes = download(installer, MAX_INSTALLER)?;
    if sha256_hex(&bytes) != expected {
        return Err("the downloaded installer doesn't match the release's checksum; nothing was installed".into());
    }
    let dir = std::env::temp_dir().join("sintec-pdf-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let msi = dir.join(name);
    std::fs::write(&msi, &bytes).map_err(|e| e.to_string())?;
    Ok(msi)
}

/// The script that waits for this process to exit, installs the MSI (one administrator prompt)
/// and starts the app again, whether or not the installation went ahead.
pub fn installer_script(pid: u32, msi: &str, exe: &str) -> String {
    let q = |s: &str| s.replace('\'', "''");
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'\n\
         Wait-Process -Id {pid} -Timeout 60\n\
         Start-Process msiexec.exe -Verb RunAs -Wait -ArgumentList '/i \"{msi}\" /passive /norestart'\n\
         Start-Process '{exe}'\n",
        msi = q(msi),
        exe = q(exe)
    )
}

#[cfg(windows)]
fn launch_installer(msi: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let script = installer_script(std::process::id(), &msi.to_string_lossy(), &exe.to_string_lossy());
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &pdfcraft_print::spool::powershell_encoded(&script)])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start the installer: {e}"))
}

#[cfg(not(windows))]
fn launch_installer(_msi: &std::path::Path) -> Result<(), String> {
    Err("automatic updates install the Windows installer; download the new version instead".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_published_release_reads_as_up_to_date() {
        let r = no_release_yet();
        assert!(!pdfcraft_ui_egui::updates::is_newer(&r.version, env!("CARGO_PKG_VERSION")), "{r:?}");
        assert_eq!(r.url, RELEASES_PAGE);
    }

    #[test]
    fn answers_are_read_and_only_our_release_pages_are_offered() {
        let r = parse(r#"{"tag_name":"v0.2.0","html_url":"https://github.com/sintec-llc/sintec.pdf/releases/tag/v0.2.0"}"#).unwrap();
        assert_eq!(
            r,
            Release { version: "v0.2.0".into(), url: "https://github.com/sintec-llc/sintec.pdf/releases/tag/v0.2.0".into(), ..Default::default() }
        );
        for elsewhere in ["https://example.com/pdfcraft.exe", "https://github.com/sintec-llc/sintec.pdf/releases.evil/x", "javascript:alert(1)"] {
            let r = parse(&format!(r#"{{"tag_name":"v9.9.9","html_url":"{elsewhere}"}}"#)).unwrap();
            assert_eq!(r.url, RELEASES_PAGE, "{elsewhere}");
        }
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn the_installer_and_checksums_come_only_from_our_downloads() {
        let msi = format!("sintec-pdf-0.1.3{}", installer_suffix());
        let body = format!(
            r#"{{"tag_name":"v0.1.3","html_url":"https://github.com/sintec-llc/sintec.pdf/releases/tag/v0.1.3","assets":[
            {{"name":"sintec-pdf-0.1.3-windows-x64-portable.zip","browser_download_url":"https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/sintec-pdf-0.1.3-windows-x64-portable.zip"}},
            {{"name":"{msi}","browser_download_url":"https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/{msi}"}},
            {{"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/SHA256SUMS.txt"}}]}}"#
        );
        let r = parse(&body).unwrap();
        assert_eq!(r.installer.as_deref(), Some(format!("https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/{msi}").as_str()));
        assert_eq!(r.checksums.as_deref(), Some("https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/SHA256SUMS.txt"));
        let evil = body.replace("https://github.com/sintec-llc/sintec.pdf/releases/download/v0.1.3/sintec-pdf", "https://evil.example/sintec-pdf");
        assert_eq!(parse(&evil).unwrap().installer, None, "an installer elsewhere is never offered");
    }

    #[test]
    fn checksums_are_read_from_the_release_list() {
        let sums = "0b1e…  junk\nabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  sintec-pdf-0.1.3-windows-x64.msi\n";
        assert_eq!(
            listed_sha256(sums, "sintec-pdf-0.1.3-windows-x64.msi").as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert_eq!(listed_sha256(sums, "other.msi"), None);
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn the_installer_script_waits_installs_and_restarts() {
        let s = installer_script(
            4242,
            r"C:\Users\O'Neil\AppData\Local\Temp\sintec-pdf-update\sintec-pdf-0.1.3-windows-x64.msi",
            r"C:\Program Files\Sintec.PDF\sintec-pdf.exe",
        );
        assert!(s.contains("Wait-Process -Id 4242"), "{s}");
        assert!(
            s.contains(r#"'/i "C:\Users\O''Neil\AppData\Local\Temp\sintec-pdf-update\sintec-pdf-0.1.3-windows-x64.msi" /passive /norestart'"#),
            "{s}"
        );
        assert!(s.contains("-Verb RunAs"), "{s}");
        assert!(s.trim_end().ends_with(r"Start-Process 'C:\Program Files\Sintec.PDF\sintec-pdf.exe'"), "{s}");
    }

    /// Live: asks GitHub over TLS with the OS's roots (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn github_answers_with_the_latest_release() {
        let r = latest_release().unwrap();
        assert!(pdfcraft_ui_egui::updates::is_newer(&r.version, "0.0.0"), "{r:?}");
        assert!(r.url.starts_with(RELEASES_PAGE), "{r:?}");
        assert!(r.installer.is_some() && r.checksums.is_some(), "{r:?}");
    }

    /// Live: downloads the latest release's installer and checks it (nothing is installed).
    #[test]
    #[ignore = "needs network access; downloads the installer"]
    fn the_latest_installer_downloads_and_matches_its_checksum() {
        let r = latest_release().unwrap();
        let msi = fetch_installer(&r).unwrap();
        assert!(std::fs::metadata(&msi).unwrap().len() > 1 << 20, "{}", msi.display());
        let _ = std::fs::remove_file(msi);
    }
}
