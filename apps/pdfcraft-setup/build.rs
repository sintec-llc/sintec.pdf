//! Embeds the MSI that `setup.exe` installs, and (on Windows) the app icon and version info.
//!
//! `SINTEC_SETUP_MSI` names the built, signed MSI (packaging/windows/package.ps1 sets it). Without
//! it the payload is empty: workspace builds and tests still compile, and such a setup.exe says it
//! carries no installer.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SINTEC_SETUP_MSI");
    println!("cargo:rerun-if-env-changed=PDFCRAFT_REQUIRE_WINRES");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("payload.msi");
    match std::env::var_os("SINTEC_SETUP_MSI").filter(|v| !v.is_empty()) {
        Some(msi) => {
            println!("cargo:rerun-if-changed={}", std::path::Path::new(&msi).display());
            // The unpacked copy keeps the MSI's own name (Windows Installer shows it).
            let name = std::path::Path::new(&msi).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            println!("cargo:rustc-env=SINTEC_SETUP_MSI_NAME={name}");
            if let Err(e) = std::fs::copy(&msi, &out) {
                println!("cargo::error=SINTEC_SETUP_MSI {}: {e}", std::path::Path::new(&msi).display());
                return;
            }
        }
        None => {
            println!("cargo:rustc-env=SINTEC_SETUP_MSI_NAME=sintec-pdf-setup.msi");
            if let Err(e) = std::fs::write(&out, b"") {
                println!("cargo::error=writing the empty payload: {e}");
                return;
            }
        }
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/pdfcraft.ico")
        .set("ProductName", "Sintec.PDF")
        .set("FileDescription", "Sintec.PDF Setup")
        .set("CompanyName", "Sintec LLC")
        .set("ProductVersion", &version)
        .set("LegalCopyright", "Copyright (c) Sintec LLC. Based on PdfCraft, (c) the ArtCraft Team and the PdfCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "sintec-pdf-setup.exe")
        .set("InternalName", "sintec-pdf-setup");
    if let Err(e) = res.compile() {
        if std::env::var_os("PDFCRAFT_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources failed: {e}");
            return;
        }
        println!("cargo:warning=setup.exe built without icon/version resources: {e}");
    }
}
