//! Sintec.PDF: scanning a page through Windows Image Acquisition (WIA), the scanner support built
//! into Windows: the system's own dialogs choose the scanner and show its settings (preview,
//! colour, resolution), and the page comes back as a PNG. Works with any scanner that has a WIA
//! driver, which is nearly all of them; no scanner software is needed.
//!
//! The script runs in PowerShell (WIA is a COM interface), handed over on standard input like the
//! printing and Office scripts (`pdfcraft_print::spool::run_powershell`).
//!
//! On Linux the page comes from SANE's `scanimage` instead (task 028); macOS has no scanning yet.

/// The WIA script: the scanner's dialog, the page saved as PNG to `$env:SINTEC_SCAN_FILE`. Prints
/// `OK` (scanned), `CANCEL` (the user closed the dialog), or `ERR <hresult> <message>` on stderr.
/// ASCII only: PowerShell reads standard input in the console code page.
pub const SCAN_SCRIPT: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
try {
  $dialog = New-Object -ComObject WIA.CommonDialog
} catch {
  [Console]::Error.WriteLine('NOWIA ' + $_.Exception.Message)
  exit 3
}
try {
  # Device type 1 = scanner; the scanner's own dialog; PNG.
  $image = $dialog.ShowAcquireImage(1, 0, 0, '{B96B3CAF-0728-11D3-9D7B-0000F81EF32E}', $false, $true, $false)
} catch {
  $code = '{0:X8}' -f $_.Exception.HResult
  [Console]::Error.WriteLine('ERR ' + $code + ' ' + $_.Exception.Message)
  exit 2
}
if ($null -eq $image) {
  'CANCEL'
  exit 0
}
$path = $env:SINTEC_SCAN_FILE
if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
$image.SaveFile($path)
'DPI ' + [int]$image.HorizontalResolution + ' ' + [int]$image.VerticalResolution
'OK'
"#;

/// The largest scanned page read back (an A4 colour page at 600 dpi as PNG stays well under it).
pub const MAX_SCAN_BYTES: u64 = 256 << 20;

/// Scan one page: `Ok(Some(png))`, `Ok(None)` when the user cancelled, or a readable reason.
/// Blocks until the scanner's dialog closes, so run it off the UI thread. `dir` holds the
/// temporary PNG.
#[cfg(windows)]
pub fn scan_page(dir: &std::path::Path) -> Result<Option<Vec<u8>>, ScanError> {
    let failed = |e: std::io::Error| ScanError::Failed(e.to_string());
    std::fs::create_dir_all(dir).map_err(failed)?;
    let file = dir.join("scan.png");
    let out = pdfcraft_print::spool::run_powershell(SCAN_SCRIPT, &[("SINTEC_SCAN_FILE", file.as_os_str())]).map_err(|_| ScanError::NoWia)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result = if stdout.lines().any(|l| l.trim() == "CANCEL") {
        Ok(None)
    } else if stdout.lines().any(|l| l.trim() == "OK") {
        match std::fs::metadata(&file) {
            Ok(m) if m.len() > MAX_SCAN_BYTES => Err(ScanError::TooLarge),
            Ok(_) => std::fs::read(&file).map(|png| Some(with_dpi(png, reported_dpi(&stdout)))).map_err(failed),
            Err(e) => Err(failed(e)),
        }
    } else {
        Err(explain(&String::from_utf8_lossy(&out.stderr)))
    };
    clean_up(dir, &file);
    result
}

/// Remove the temporary PNG and then its folder (`sintec-scan-<pid>`), which is left behind
/// otherwise, one per app run that scanned. A folder still holding anything else stays.
#[cfg(any(windows, test))]
fn clean_up(dir: &std::path::Path, file: &std::path::Path) {
    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_dir(dir);
}

/// Linux: SANE, the scanner support of Linux desktops, through its command-line front end
/// `scanimage` (the `sane-utils` package). It scans one page from the default scanner
/// (`$SANE_DEFAULT_DEVICE`, else the first one found) at [`SANE_DPI`] as PNG. There is no settings
/// dialog, so a scan never comes back cancelled. The page is read from `scanimage`'s output and
/// never written to disk, so `dir` is not used: on Linux the temporary folder would sit in the
/// shared `/tmp`, where another user could plant a link in it.
#[cfg(target_os = "linux")]
pub fn scan_page(_dir: &std::path::Path) -> Result<Option<Vec<u8>>, ScanError> {
    scan_with(std::ffi::OsStr::new("scanimage"))
}

/// Elsewhere (macOS) there is no scanner support yet.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn scan_page(_dir: &std::path::Path) -> Result<Option<Vec<u8>>, ScanError> {
    Err(ScanError::NotSupported)
}

/// The resolution SANE scans at, in dots per inch: a scanned page that reads well and OCRs well.
pub const SANE_DPI: u32 = 300;

/// The longest a SANE scan may take before it is given up (a scanner that stopped answering).
#[cfg(target_os = "linux")]
const SANE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// [`scan_page`] through the `scanimage` program `program` (tests run a stand-in).
#[cfg(target_os = "linux")]
fn scan_with(program: &std::ffi::OsStr) -> Result<Option<Vec<u8>>, ScanError> {
    // A backend that doesn't offer the resolution refuses the option: scan at its own then, and
    // the PNG's own resolution (scanimage records it) sizes the page.
    let (png, dpi) = match run_scanimage(program, Some(SANE_DPI)) {
        Err(ScanError::Failed(m)) if m.contains("--resolution") => (run_scanimage(program, None)?, None),
        other => (other?, Some((SANE_DPI, SANE_DPI))),
    };
    if png.is_empty() {
        return Err(ScanError::Failed(String::new()));
    }
    Ok(Some(with_dpi(png, dpi)))
}

/// Run `scanimage` once and return the PNG it writes to standard output; `dpi` asks for a
/// resolution. More than [`MAX_SCAN_BYTES`] is [`ScanError::TooLarge`].
#[cfg(target_os = "linux")]
fn run_scanimage(program: &std::ffi::OsStr, dpi: Option<u32>) -> Result<Vec<u8>, ScanError> {
    use std::io::Read as _;
    use std::process::{Command, Stdio};
    let failed = |e: std::io::Error| ScanError::Failed(e.to_string());
    let mut cmd = Command::new(program);
    cmd.arg("--format=png");
    if let Some(dpi) = dpi {
        cmd.arg("--resolution").arg(dpi.to_string());
    }
    // SANE's messages in English, so they can be read ([`explain_sane`]); shown translated.
    cmd.env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ScanError::NoSane),
        Err(e) => return Err(failed(e)),
    };
    // The page, read on its own thread up to one byte past the cap. Past it the pipe closes and
    // scanimage stops on the broken pipe instead of running into the timeout.
    let stdout = child.stdout.take().map(|out| {
        std::thread::spawn(move || {
            let mut png = Vec::new();
            let read = out.take(MAX_SCAN_BYTES.saturating_add(1)).read_to_end(&mut png);
            read.map(|_| png)
        })
    });
    // Read what it says on its own thread, so a chatty backend never fills the pipe and stalls.
    let stderr = child.stderr.take().map(|err| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = err.take(64 << 10).read_to_end(&mut text);
            String::from_utf8_lossy(&text).into_owned()
        })
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() > SANE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(ScanError::Failed("the scanner did not finish in time".into()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => break Err(failed(e)),
        }
    };
    let said = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
    let png = match stdout.map(|t| t.join()) {
        Some(Ok(Ok(png))) => png,
        Some(Ok(Err(e))) => return Err(failed(e)),
        // The reader thread panicked (it can't) or there was no pipe.
        _ => return Err(ScanError::Failed(String::new())),
    };
    if !u64::try_from(png.len()).is_ok_and(|n| n <= MAX_SCAN_BYTES) {
        return Err(ScanError::TooLarge);
    }
    match status? {
        s if s.success() => Ok(png),
        _ => Err(explain_sane(&said)),
    }
}

/// What `scanimage` reported on stderr (SANE's status messages in English), read as a
/// [`ScanError`].
#[cfg(any(target_os = "linux", test))]
pub fn explain_sane(stderr: &str) -> ScanError {
    let text = stderr.trim();
    let has = |s: &str| text.contains(s);
    if has("no SANE devices found") {
        ScanError::NoScanner
    } else if has("Device busy") {
        ScanError::Busy
    } else if has("Document feeder out of documents") {
        ScanError::FeederEmpty
    } else if has("Document feeder jammed") {
        ScanError::PaperJam
    } else if has("Scanner cover is open") {
        ScanError::CoverOpen
    } else if has("Error during device I/O") {
        ScanError::DeviceError
    } else {
        // The last line says what failed, without the program's name.
        let line = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
        ScanError::Failed(line.strip_prefix("scanimage:").unwrap_or(line).trim().chars().take(300).collect())
    }
}

/// The resolution the script reported (`DPI <h> <v>`).
pub fn reported_dpi(stdout: &str) -> Option<(u32, u32)> {
    let line = stdout.lines().map(str::trim).find_map(|l| l.strip_prefix("DPI "))?;
    let mut it = line.split_whitespace().map(|n| n.parse::<u32>().ok().filter(|&d| (10..=10_000).contains(&d)));
    let h = it.next().flatten()?;
    Some((h, it.next().flatten().unwrap_or(h)))
}

/// `png` with its resolution recorded (a `pHYs` chunk after the header) when it has none, so the
/// page comes out at the paper's size rather than one point per pixel.
pub fn with_dpi(png: Vec<u8>, dpi: Option<(u32, u32)>) -> Vec<u8> {
    const SIG: usize = 8;
    let Some((h, v)) = dpi else { return png };
    // Walk the chunks: length, type, data, CRC.
    let mut at = SIG;
    let mut after_header = None;
    // `head` is exactly 8 bytes, so indexing it cannot fail.
    while let Some(head) = png.get(at..at.saturating_add(8)) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = &head[4..8];
        if kind == b"pHYs" {
            return png;
        }
        let next = at.saturating_add(12).saturating_add(len);
        if kind == b"IHDR" {
            after_header = Some(next);
        }
        if kind == b"IDAT" || kind == b"IEND" || next > png.len() {
            break;
        }
        at = next;
    }
    let Some(pos) = after_header.filter(|&p| p <= png.len() && png.get(..SIG) == Some(b"\x89PNG\r\n\x1a\n".as_slice())) else { return png };
    // Pixels per metre, unit 1 (metre).
    let ppm = |d: u32| ((f64::from(d) / 0.0254).round() as u32).to_be_bytes();
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&ppm(h));
    chunk.extend_from_slice(&ppm(v));
    chunk.push(1);
    let crc = crc32(chunk.get(4..).unwrap_or_default());
    chunk.extend_from_slice(&crc.to_be_bytes());
    let mut out = png;
    out.splice(pos..pos, chunk);
    out
}

/// PNG's CRC-32 (ISO 3309).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Why a scan didn't work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    /// Neither Windows nor Linux: no scanner support.
    NotSupported,
    /// WIA itself is missing (a stripped-down Windows) or PowerShell can't run.
    NoWia,
    /// Linux: SANE's `scanimage` isn't installed.
    NoSane,
    NoScanner,
    Busy,
    Offline,
    FeederEmpty,
    PaperJam,
    CoverOpen,
    LampOff,
    /// The scanner reported a general error.
    DeviceError,
    /// The page is larger than [`MAX_SCAN_BYTES`].
    TooLarge,
    /// Anything else, with the system's own message (may be empty).
    Failed(String),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotSupported => "Scanning works on Windows, through its built-in scanner support",
            Self::NoWia => "Windows' scanner support (WIA) isn't available on this computer",
            Self::NoSane => "Scanning needs SANE's scanimage (the sane-utils package). Install it and try again",
            Self::NoScanner => "No scanner was found. Check that it is connected and switched on",
            Self::Busy => "The scanner is busy",
            Self::Offline => "The scanner is offline. Check that it is connected and switched on",
            Self::FeederEmpty => "The document feeder is empty",
            Self::PaperJam => "The scanner has a paper jam",
            Self::CoverOpen => "The scanner's cover is open",
            Self::LampOff => "The scanner's lamp is off",
            Self::DeviceError => "The scanner reported an error. Check it and try again",
            Self::TooLarge => "The scanned page is too large",
            Self::Failed(m) if m.is_empty() => "The scan failed",
            Self::Failed(m) => return write!(f, "The scan failed: {m}"),
        })
    }
}

/// What the script reported on stderr, read as a [`ScanError`] (WIA's error codes, else its text).
pub fn explain(stderr: &str) -> ScanError {
    let line = stderr.lines().map(str::trim).find(|l| l.starts_with("ERR ") || l.starts_with("NOWIA")).unwrap_or(stderr.trim());
    if line.starts_with("NOWIA") {
        return ScanError::NoWia;
    }
    let code = line.strip_prefix("ERR ").and_then(|rest| rest.split_whitespace().next()).unwrap_or("");
    match code {
        "80210015" => ScanError::NoScanner,
        "80210006" => ScanError::Busy,
        "80210005" => ScanError::Offline,
        "80210003" => ScanError::FeederEmpty,
        "80210002" => ScanError::PaperJam,
        "8021000C" => ScanError::CoverOpen,
        "80210016" => ScanError::LampOff,
        "80210001" | "8021000A" => ScanError::DeviceError,
        _ => ScanError::Failed(line.strip_prefix("ERR ").map_or(line, |rest| rest.split_once(' ').map_or("", |(_, m)| m)).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 008: the `sintec-scan-<pid>` folder went with the PNG, not just the PNG.
    #[test]
    fn a_scan_leaves_no_temporary_folder_behind() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("sintec-scan-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("scan.png");
        std::fs::write(&file, b"png").unwrap();
        clean_up(&dir, &file);
        // A folder holding something else is not the scan's to remove.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("other"), b"x").unwrap();
        clean_up(&dir, &file);
        assert!(dir.join("other").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_script_is_ascii_and_saves_png() {
        assert!(SCAN_SCRIPT.is_ascii(), "PowerShell reads standard input in the console code page");
        assert!(SCAN_SCRIPT.contains("SINTEC_SCAN_FILE") && SCAN_SCRIPT.contains("B96B3CAF-0728-11D3-9D7B-0000F81EF32E"));
    }

    #[test]
    fn a_scan_without_a_resolution_gets_the_scanners() {
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 2, 2);
            enc.set_color(png::ColorType::Grayscale);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 255, 255, 0]).unwrap();
        }
        assert_eq!(
            reported_dpi(
                "DPI 300 300
OK"
            ),
            Some((300, 300))
        );
        assert_eq!(reported_dpi("OK"), None);
        let fixed = with_dpi(png.clone(), Some((300, 300)));
        let reader = png::Decoder::new(std::io::Cursor::new(&fixed)).read_info().unwrap();
        let dims = reader.info().pixel_dims.unwrap();
        assert_eq!((dims.xppu, dims.unit), (11811, png::Unit::Meter), "300 dpi");
        // Already there: unchanged; garbage: unchanged.
        assert_eq!(with_dpi(fixed.clone(), Some((150, 150))), fixed);
        assert_eq!(with_dpi(b"not a png".to_vec(), Some((300, 300))), b"not a png");
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    /// Task 028: SANE's status messages (scanimage, `LC_ALL=C`) read as the same reasons as WIA's.
    #[test]
    fn sane_errors_read_plainly() {
        assert_eq!(explain_sane("scanimage: no SANE devices found\n"), ScanError::NoScanner);
        assert_eq!(explain_sane("scanimage: open of device genesys:libusb:001:004 failed: Device busy"), ScanError::Busy);
        assert_eq!(explain_sane("scanimage: sane_start: Document feeder out of documents"), ScanError::FeederEmpty);
        assert_eq!(explain_sane("scanimage: sane_read: Document feeder jammed"), ScanError::PaperJam);
        assert_eq!(explain_sane("scanimage: sane_start: Scanner cover is open"), ScanError::CoverOpen);
        assert_eq!(explain_sane("scanimage: sane_read: Error during device I/O"), ScanError::DeviceError);
        assert_eq!(
            explain_sane("noise\nscanimage: setting of option --resolution failed (Invalid argument)\n\n"),
            ScanError::Failed("setting of option --resolution failed (Invalid argument)".into())
        );
        assert_eq!(explain_sane("").to_string(), "The scan failed");
        assert_eq!(explain_sane(&"x".repeat(10_000)), ScanError::Failed("x".repeat(300)), "a runaway message is cut short");
        assert!(ScanError::NoSane.to_string().contains("sane-utils"));
    }

    /// Task 028: a stand-in `scanimage` (a shell script) scans, fails, or isn't installed.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scans_through_scanimage() {
        use std::os::unix::fs::PermissionsExt;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let base = std::env::temp_dir().join(format!("sintec-sane-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 2, 2);
            enc.set_color(png::ColorType::Grayscale);
            enc.write_header().unwrap().write_image_data(&[0, 255, 255, 0]).unwrap();
        }
        let page = base.join("page.png");
        std::fs::write(&page, &png).unwrap();
        let script = |name: &str, body: &str| {
            let path = base.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        // Scans: the page comes back at SANE_DPI, read from scanimage's output (no file in /tmp).
        let ok = script("ok", &format!("cat '{}'", page.display()));
        let scanned = scan_with(ok.as_os_str()).unwrap().unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(&scanned)).read_info().unwrap();
        assert_eq!(reader.info().pixel_dims.map(|d| d.xppu), Some(11811), "300 dpi");
        // A backend without 300 dpi: scanned again at its own resolution.
        let picky = script(
            "picky",
            &format!(
                "case \"$*\" in *--resolution*) echo 'scanimage: setting of option --resolution failed (Invalid argument)' >&2; exit 1;; esac; cat '{}'",
                page.display()
            ),
        );
        assert_eq!(scan_with(picky.as_os_str()).unwrap(), Some(png.clone()));
        // No scanner; nothing written; not installed.
        let none = script("none", "echo 'scanimage: no SANE devices found' >&2; exit 1");
        assert_eq!(scan_with(none.as_os_str()), Err(ScanError::NoScanner));
        let empty = script("empty", "exit 0");
        assert_eq!(scan_with(empty.as_os_str()), Err(ScanError::Failed(String::new())));
        assert_eq!(scan_with(base.join("no-such-scanimage").as_os_str()), Err(ScanError::NoSane));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn wia_errors_read_plainly() {
        assert_eq!(explain("ERR 80210015 No device"), ScanError::NoScanner);
        assert_eq!(explain("noise\r\nERR 80210003 Paper empty"), ScanError::FeederEmpty);
        assert_eq!(explain("NOWIA Retrieving the COM class factory failed"), ScanError::NoWia);
        assert_eq!(explain("ERR 80004005 Something odd").to_string(), "The scan failed: Something odd");
        assert_eq!(explain("").to_string(), "The scan failed");
    }
}
