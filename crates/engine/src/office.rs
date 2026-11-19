//! Office documents to PDF through the installed office suite: Microsoft Office (Word, Excel,
//! PowerPoint through their automation interface, invisibly) or, without it, LibreOffice.
//! Sintec.PDF doesn't read Office formats itself; the suite that wrote them converts them with
//! their formatting, tables and images intact. Where Microsoft Office can't (or isn't there),
//! `pdfcraft_create::office_to_pdf` runs LibreOffice, which is also what upstream's File ▸ Open uses.
//!
//! One PowerShell run converts a whole batch, starting each Office application once. Files open
//! read-only and aren't added to the recent-files list; a password-protected file fails (it is
//! skipped) instead of waiting for a password nobody will type.

use std::path::{Path, PathBuf};

/// Which application converts a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    Excel,
    PowerPoint,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Word => "word",
            Kind::Excel => "excel",
            Kind::PowerPoint => "powerpoint",
        }
    }
}

/// Word-processing types.
pub const WORD_EXTS: &[&str] = &["doc", "docx", "docm", "dot", "dotx", "rtf", "odt"];
/// Spreadsheet types.
pub const EXCEL_EXTS: &[&str] = &["xls", "xlsx", "xlsm", "xlsb", "ods"];
/// Presentation types.
pub const POWERPOINT_EXTS: &[&str] = &["ppt", "pptx", "pptm", "pps", "ppsx", "odp"];

/// The application for a file extension (lowercase), if it is an Office document.
pub fn kind_for(ext: &str) -> Option<Kind> {
    if WORD_EXTS.contains(&ext) {
        Some(Kind::Word)
    } else if EXCEL_EXTS.contains(&ext) {
        Some(Kind::Excel)
    } else if POWERPOINT_EXTS.contains(&ext) {
        Some(Kind::PowerPoint)
    } else {
        None
    }
}

/// Every Office extension.
pub fn exts() -> impl Iterator<Item = &'static str> {
    WORD_EXTS.iter().chain(EXCEL_EXTS).chain(POWERPOINT_EXTS).copied()
}

/// The job file: `kind<TAB>input<TAB>output` per line (Windows paths hold neither tabs nor line
/// breaks).
pub fn job_lines(items: &[(Kind, PathBuf, PathBuf)]) -> String {
    items.iter().map(|(k, i, o)| format!("{}\t{}\t{}\n", k.as_str(), office_path(i), office_path(o))).collect()
}

/// A path as Office wants it: absolute, with backslashes (Word can't open `C:/a/b.docx` or a
/// path mixing separators, though Excel can).
pub fn office_path(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.display().to_string();
    if cfg!(windows) { s.replace('/', "\\") } else { s }
}

/// Converts every line of the job file in `$env:SINTEC_OFFICE_JOB`, one application instance per
/// kind, and prints `OK<TAB>input` or `ERR<TAB>input<TAB>message` per file (UTF-8).
pub const OFFICE_SCRIPT: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
$apps = @{}
function Get-App($kind) {
  if (-not $apps.ContainsKey($kind)) {
    switch ($kind) {
      'word' { $a = New-Object -ComObject Word.Application; $a.Visible = $false; $a.DisplayAlerts = 0 }
      'excel' { $a = New-Object -ComObject Excel.Application; $a.Visible = $false; $a.DisplayAlerts = $false; $a.AskToUpdateLinks = $false }
      'powerpoint' { $a = New-Object -ComObject PowerPoint.Application }
    }
    $apps[$kind] = $a
  }
  return $apps[$kind]
}
# A password nobody set: a protected file fails instead of asking for its password.
$nopw = 'sintec-no-password'
foreach ($line in (Get-Content -LiteralPath $env:SINTEC_OFFICE_JOB -Encoding UTF8)) {
  if (-not $line) { continue }
  $kind, $in, $out = $line -split "`t", 3
  try {
    $app = Get-App $kind
    switch ($kind) {
      'word' {
        # FileName, ConfirmConversions, ReadOnly, AddToRecentFiles, PasswordDocument
        $doc = $app.Documents.Open($in, $false, $true, $false, $nopw)
        try { $doc.ExportAsFixedFormat($out, 17) } finally { $doc.Close(0) }
      }
      'excel' {
        # Filename, UpdateLinks, ReadOnly, Format, Password
        $wb = $app.Workbooks.Open($in, 0, $true, 5, $nopw)
        try { $wb.ExportAsFixedFormat(0, $out) } finally { $wb.Close($false) }
      }
      'powerpoint' {
        # FileName (with an unused password, so a protected file fails), ReadOnly, Untitled, WithWindow
        $p = $app.Presentations.Open($in + '::' + $nopw + '::', -1, 0, 0)
        try { $p.SaveAs($out, 32) } finally { $p.Close() }
      }
    }
    Write-Output ("OK`t" + $in)
  } catch {
    Write-Output ("ERR`t" + $in + "`t" + ($_.Exception.Message -replace '\s+', ' '))
  }
}
foreach ($a in $apps.Values) { try { $a.Quit() } catch {} }"#;

/// The results of a batch, by input path: the PDF bytes or why it failed.
pub type Results = Vec<(PathBuf, Result<Vec<u8>, String>)>;

/// Convert `inputs` to PDF with the installed office suite. Every input gets a result. On
/// Windows Microsoft Office goes first; whatever it can't do (or on other systems) LibreOffice
/// converts ([`pdfcraft_create::office_to_pdf`]).
pub fn convert(inputs: &[(Kind, PathBuf)]) -> Results {
    if inputs.is_empty() {
        return Vec::new();
    }
    #[cfg(windows)]
    let first: Results = {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("sintec-pdf-office-{}-{stamp}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return inputs.iter().map(|(_, p)| (p.clone(), Err(e.to_string()))).collect();
        }
        let items: Vec<(Kind, PathBuf, PathBuf)> =
            inputs.iter().enumerate().map(|(i, (k, p))| (*k, p.clone(), dir.join(format!("doc-{i:05}.pdf")))).collect();
        let results = run_office(&items, &dir).unwrap_or_else(|e| inputs.iter().map(|(_, p)| (p.clone(), Err(e.clone()))).collect());
        let _ = std::fs::remove_dir_all(&dir);
        results
    };
    #[cfg(not(windows))]
    let first: Results = inputs.iter().map(|(_, p)| (p.clone(), Err("Microsoft Office is available only on Windows".to_string()))).collect();
    first
        .into_iter()
        .map(|(p, r)| match r {
            // A password-protected file stays that way in LibreOffice too.
            Err(e) if !e.contains("password") => match libreoffice(&p) {
                Ok(b) => (p, Ok(b)),
                // Off Windows the only converter is LibreOffice: say what it said.
                Err(l) if cfg!(not(windows)) => (p, Err(l)),
                Err(_) => (p, Err(e)),
            },
            other => (p, other),
        })
        .collect()
}

/// `input` converted by LibreOffice.
fn libreoffice(input: &Path) -> Result<Vec<u8>, String> {
    let name = input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
    pdfcraft_create::office_to_pdf(&name, &bytes).map_err(|e| e.to_string())
}

/// Runs a PowerShell script from standard input (not `-EncodedCommand`, which antivirus products
/// read as malware, and not a `.ps1` file, which the execution policy can block). The script must
/// be ASCII (PowerShell reads standard input in the console code page); data goes in `env`.
#[cfg(windows)]
pub(crate) fn run_powershell(script: &str, env: &[(&str, &std::ffi::OsStr)]) -> std::io::Result<std::process::Output> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !script.is_ascii() {
        return Err(std::io::Error::other("PowerShell scripts must be ASCII"));
    }
    // System32's PowerShell, not whatever a writable folder earlier on PATH holds.
    let exe = std::env::var_os("SystemRoot").map_or_else(
        || PathBuf::from("powershell.exe"),
        |root| PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
    );
    let mut c = std::process::Command::new(exe);
    c.args(["-NoProfile", "-NonInteractive", "-Command", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn()?;
    let stdin = child.stdin.take();
    // Written on its own thread so a script that prints a lot before reading all of its input
    // can't deadlock against a full output pipe.
    let text = format!("& {{\r\n{script}\r\n}}\r\n\r\n");
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(text.as_bytes());
        }
    });
    let out = child.wait_with_output();
    let _ = writer.join();
    out
}

#[cfg(windows)]
fn run_office(items: &[(Kind, PathBuf, PathBuf)], dir: &Path) -> Result<Results, String> {
    let job = dir.join("job.tsv");
    std::fs::write(&job, job_lines(items)).map_err(|e| e.to_string())?;
    let out = run_powershell(OFFICE_SCRIPT, &[("SINTEC_OFFICE_JOB", job.as_os_str())]).map_err(|e| format!("PowerShell is not available: {e}"))?;
    let report = String::from_utf8_lossy(&out.stdout);
    Ok(items
        .iter()
        .map(|(kind, input, output)| {
            let key = office_path(input);
            let line = report.lines().find(|l| l.split('\t').nth(1) == Some(key.as_str()));
            let r = match line.map(|l| l.splitn(3, '\t').collect::<Vec<_>>()) {
                Some(parts) if parts.first() == Some(&"OK") => std::fs::read(output).map_err(|e| e.to_string()),
                Some(parts) if parts.first() == Some(&"ERR") => Err(explain(*kind, parts.get(2).copied().unwrap_or("conversion failed"))),
                _ => Err(explain(*kind, String::from_utf8_lossy(&out.stderr).trim())),
            };
            (input.clone(), r)
        })
        .collect())
}

/// A readable reason for an Office failure.
pub fn explain(kind: Kind, raw: &str) -> String {
    let app = match kind {
        Kind::Word => "Microsoft Word",
        Kind::Excel => "Microsoft Excel",
        Kind::PowerPoint => "Microsoft PowerPoint",
    };
    let lower = raw.to_lowercase();
    if lower.contains("80040154") || lower.contains("class not registered") || lower.contains("не зарегистрирован") {
        format!("{app} isn't installed")
    } else if lower.contains("password") || lower.contains("парол") {
        "the document is password-protected".to_string()
    } else if raw.is_empty() {
        format!("{app} couldn't convert it")
    } else {
        format!("{app}: {raw}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_error_for_that_file_not_a_crash() {
        let missing = PathBuf::from("no-such-dir-047/missing.docx");
        let out = convert(&[(Kind::Word, missing.clone())]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, missing);
        assert!(out[0].1.is_err());
        assert!(convert(&[]).is_empty());
    }

    #[test]
    fn office_types_and_job_file() {
        // Handed to PowerShell on standard input, read in the console code page.
        assert!(OFFICE_SCRIPT.is_ascii());
        assert_eq!(kind_for("docx"), Some(Kind::Word));
        assert_eq!(kind_for("rtf"), Some(Kind::Word));
        assert_eq!(kind_for("xlsx"), Some(Kind::Excel));
        assert_eq!(kind_for("pptx"), Some(Kind::PowerPoint));
        assert_eq!(kind_for("pdf"), None);
        // Windows paths: Office conversion runs only on Windows (elsewhere `C:\…` isn't absolute).
        if cfg!(windows) {
            let lines = job_lines(&[(Kind::Word, PathBuf::from("C:\\Документы\\отчёт.docx"), PathBuf::from("C:\\tmp\\doc-00000.pdf"))]);
            assert_eq!(lines, "word\tC:\\Документы\\отчёт.docx\tC:\\tmp\\doc-00000.pdf\n");
        }
        assert_eq!(explain(Kind::Excel, "Retrieving the COM class factory failed: 80040154 Class not registered"), "Microsoft Excel isn't installed");
        assert_eq!(explain(Kind::Word, "Неверный пароль"), "the document is password-protected");
    }
}
