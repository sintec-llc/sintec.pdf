//! `sintec-pdf-<version>-windows-<arch>-setup.exe`: the signed Sintec.PDF MSI, wrapped in an .exe
//! for people and tools that expect one. It writes the MSI to `%LOCALAPPDATA%\Sintec.PDF\setup`,
//! runs Windows Installer on it (the MSI's own dialogs, elevation and upgrade logic, exactly as
//! double-clicking the MSI) and passes msiexec's exit code on. The MSI stays there (older versions'
//! are removed): a later repair may need the package it was installed from.
//!
//! Arguments go to msiexec after the package, so `setup.exe /qn` installs silently and
//! `setup.exe INSTALLDESKTOPSHORTCUT=0` skips the desktop shortcut. The silent switches deployment
//! tools use for .exe installers (`/S`, `/silent`, `/quiet`, `/verysilent`) mean `/qn /norestart`.
//! A silent install still needs administrator rights: deploy it as SYSTEM (computer configuration).

#![cfg_attr(not(test), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

/// The MSI, embedded by build.rs (empty in builds without `SINTEC_SETUP_MSI`).
static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.msi"));

/// Windows Installer's "this installation package could not be opened" (ERROR_INSTALL_PACKAGE_OPEN_FAILED).
const PACKAGE_OPEN_FAILED: i32 = 1619;

fn main() {
    if PAYLOAD.is_empty() {
        eprintln!("this setup.exe was built without an installer inside");
        std::process::exit(PACKAGE_OPEN_FAILED);
    }
    let dir = setup_dir();
    let name = MSI_NAME;
    let msi = dir.join(name);
    let code = match write_payload(&dir, &msi) {
        Ok(()) => run_msiexec(&msi, msiexec_args(std::env::args_os().skip(1).collect())),
        Err(e) => {
            eprintln!("couldn't unpack the installer to {}: {e}", dir.display());
            PACKAGE_OPEN_FAILED
        }
    };
    remove_older(&dir, name);
    // msiexec's own code, whole: deployment tools read 0, 1602 (cancelled), 3010 (restart needed)…
    std::process::exit(code);
}

/// The unpacked MSI's name, the packaged MSI's own (Windows Installer shows it in its dialogs).
const MSI_NAME: &str = env!("SINTEC_SETUP_MSI_NAME");

/// Where the MSI is kept: per user (no administrator rights needed to write it), next to the app's
/// settings.
fn setup_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from).join("Sintec.PDF").join("setup")
}

/// Removes the MSIs of other versions from `dir`, keeping `keep`.
fn remove_older(dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("sintec-pdf-") && name.ends_with(".msi") && name != keep {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn write_payload(dir: &Path, msi: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(msi, PAYLOAD)
}

/// The arguments for msiexec: the usual .exe silent switches become `/qn /norestart` (once); the
/// rest pass through unchanged (msiexec switches, `PROPERTY=value`).
fn msiexec_args(args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    const SILENT: [&str; 4] = ["s", "silent", "quiet", "verysilent"];
    let is_silent = |a: &std::ffi::OsString| {
        let a = a.to_string_lossy().to_ascii_lowercase();
        let bare = a.trim_start_matches(['/', '-']);
        a.len() > bare.len() && SILENT.contains(&bare)
    };
    if !args.iter().any(is_silent) {
        return args;
    }
    let mut out: Vec<std::ffi::OsString> = vec!["/qn".into()];
    let restart_given = args.iter().any(|a| {
        let a = a.to_string_lossy().to_ascii_lowercase();
        a == "/norestart" || a == "/forcerestart" || a == "/promptrestart"
    });
    if !restart_given {
        out.push("/norestart".into());
    }
    out.extend(args.into_iter().filter(|a| !is_silent(a)));
    out
}

/// `msiexec /i <msi> <args…>`, waiting for it; its exit code (1 when it couldn't run).
fn run_msiexec(msi: &Path, args: Vec<std::ffi::OsString>) -> i32 {
    let mut cmd = std::process::Command::new(msiexec());
    cmd.arg("/i").arg(msi).args(args);
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("couldn't start Windows Installer: {e}");
            1
        }
    }
}

/// System32's msiexec.exe, not whatever a writable folder earlier on PATH holds.
fn msiexec() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("msiexec.exe"), |root| PathBuf::from(root).join("System32").join("msiexec.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert!(MSI_NAME.starts_with("sintec-pdf-") && MSI_NAME.ends_with(".msi"), "{MSI_NAME}");
        let m = msiexec();
        assert!(m.ends_with("msiexec.exe"), "{}", m.display());
        if std::env::var_os("SystemRoot").is_some() {
            assert!(m.to_string_lossy().to_ascii_lowercase().contains("system32"), "{}", m.display());
        }
    }

    #[test]
    fn silent_switches_become_qn_and_the_rest_pass_through() {
        let v = |xs: &[&str]| xs.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert_eq!(msiexec_args(v(&[])), v(&[]));
        assert_eq!(msiexec_args(v(&["/qn", "INSTALLDESKTOPSHORTCUT=0"])), v(&["/qn", "INSTALLDESKTOPSHORTCUT=0"]));
        for s in ["/S", "/silent", "-quiet", "/VERYSILENT", "--silent"] {
            assert_eq!(msiexec_args(v(&[s, "INSTALLDESKTOPSHORTCUT=0"])), v(&["/qn", "/norestart", "INSTALLDESKTOPSHORTCUT=0"]), "{s}");
        }
        assert_eq!(msiexec_args(v(&["/S", "/forcerestart"])), v(&["/qn", "/forcerestart"]));
        // Not switches: a property value or a path that happens to say "silent".
        assert_eq!(msiexec_args(v(&["silent", "MODE=quiet"])), v(&["silent", "MODE=quiet"]));
    }

    #[test]
    fn the_payload_is_written_and_older_versions_removed() {
        let dir = std::env::temp_dir().join(format!("sintec-pdf-setup-test-{}", std::process::id()));
        let msi = dir.join("sintec-pdf-0.2.3-windows-x64.msi");
        write_payload(&dir, &msi).unwrap();
        assert_eq!(std::fs::read(&msi).unwrap(), PAYLOAD);
        std::fs::write(dir.join("sintec-pdf-0.2.2-windows-x64.msi"), b"old").unwrap();
        std::fs::write(dir.join("notes.txt"), b"other").unwrap();
        remove_older(&dir, "sintec-pdf-0.2.3-windows-x64.msi");
        assert!(msi.exists() && dir.join("notes.txt").exists() && !dir.join("sintec-pdf-0.2.2-windows-x64.msi").exists());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(setup_dir().ends_with(std::path::Path::new("Sintec.PDF").join("setup")));
    }
}
