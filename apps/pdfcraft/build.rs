//! Windows only: embed the app icon and version info (VERSIONINFO) into the app exe (shipped as
//! `sintec-pdf.exe`, see packaging/windows/package.ps1), so it shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `PDFCRAFT_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/pdfcraft.ico");
    println!("cargo:rerun-if-env-changed=PDFCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/pdfcraft.ico")
        .set("ProductName", "Sintec.PDF")
        .set("FileDescription", "Sintec.PDF")
        .set("CompanyName", "Sintec LLC")
        .set("LegalCopyright", "Copyright (c) Sintec LLC. Based on PdfCraft, (c) the ArtCraft Team and the PdfCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "sintec-pdf.exe")
        .set("InternalName", "sintec-pdf");
    if let Err(e) = res.compile() {
        if std::env::var_os("PDFCRAFT_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources failed: {e}");
            return;
        }
        println!("cargo:warning=pdfcraft.exe built without icon/version resources: {e}");
    }
}
