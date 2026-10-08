//! Office documents to PDF through the installed office suite: Microsoft Office (Word, Excel,
//! PowerPoint through their automation interface, invisibly) or, without it, LibreOffice.
//! Sintec.PDF doesn't read Office formats itself; the suite that wrote them converts them with
//! their formatting, tables and images intact.
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

/// Convert `inputs` to PDF with the installed office suite. Every input gets a result.
pub fn convert(inputs: &[(Kind, PathBuf)]) -> Results {
    if inputs.is_empty() {
        return Vec::new();
    }
    #[cfg(windows)]
    {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("sintec-pdf-office-{}-{stamp}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return inputs.iter().map(|(_, p)| (p.clone(), Err(e.to_string()))).collect();
        }
        let items: Vec<(Kind, PathBuf, PathBuf)> =
            inputs.iter().enumerate().map(|(i, (k, p))| (*k, p.clone(), dir.join(format!("doc-{i:05}.pdf")))).collect();
        let results = run_office(&items, &dir).unwrap_or_else(|e| inputs.iter().map(|(_, p)| (p.clone(), Err(e.clone()))).collect());
        // Whatever Microsoft Office couldn't do, LibreOffice may.
        let results = results
            .into_iter()
            .map(|(p, r)| match r {
                Err(e) if !e.contains("password") => match libreoffice(&p, &dir) {
                    Some(Ok(b)) => (p, Ok(b)),
                    _ => (p, Err(e)),
                },
                other => (p, other),
            })
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        results
    }
    #[cfg(not(windows))]
    {
        inputs.iter().map(|(_, p)| (p.clone(), Err("Office documents are converted with Microsoft Office on Windows".to_string()))).collect()
    }
}

#[cfg(windows)]
fn run_office(items: &[(Kind, PathBuf, PathBuf)], dir: &Path) -> Result<Results, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let job = dir.join("job.tsv");
    std::fs::write(&job, job_lines(items)).map_err(|e| e.to_string())?;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &pdfcraft_print::spool::powershell_encoded(OFFICE_SCRIPT)])
        .env("SINTEC_OFFICE_JOB", &job)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("PowerShell is not available: {e}"))?;
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

/// LibreOffice's soffice, if installed in the usual place.
#[cfg(windows)]
fn libreoffice(input: &Path, dir: &Path) -> Option<Result<Vec<u8>, String>> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let soffice = [std::env::var_os("ProgramFiles"), std::env::var_os("ProgramFiles(x86)")]
        .into_iter()
        .flatten()
        .map(|p| PathBuf::from(p).join("LibreOffice").join("program").join("soffice.exe"))
        .find(|p| p.is_file())?;
    let out_dir = dir.join("libreoffice");
    let _ = std::fs::create_dir_all(&out_dir);
    let status = std::process::Command::new(soffice)
        .args(["--headless", "--norestore", "--convert-to", "pdf", "--outdir"])
        .arg(&out_dir)
        .arg(input)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    let stem = input.file_stem()?.to_string_lossy().into_owned();
    Some(match status {
        Ok(s) if s.success() => std::fs::read(out_dir.join(format!("{stem}.pdf"))).map_err(|e| e.to_string()),
        Ok(_) => Err("LibreOffice couldn't convert it".to_string()),
        Err(e) => Err(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_types_and_job_file() {
        assert_eq!(kind_for("docx"), Some(Kind::Word));
        assert_eq!(kind_for("rtf"), Some(Kind::Word));
        assert_eq!(kind_for("xlsx"), Some(Kind::Excel));
        assert_eq!(kind_for("pptx"), Some(Kind::PowerPoint));
        assert_eq!(kind_for("pdf"), None);
        let lines = job_lines(&[(Kind::Word, PathBuf::from("C:\\Документы\\отчёт.docx"), PathBuf::from("C:\\tmp\\doc-00000.pdf"))]);
        assert_eq!(lines, "word\tC:\\Документы\\отчёт.docx\tC:\\tmp\\doc-00000.pdf\n");
        assert_eq!(explain(Kind::Excel, "Retrieving the COM class factory failed: 80040154 Class not registered"), "Microsoft Excel isn't installed");
        assert_eq!(explain(Kind::Word, "Неверный пароль"), "the document is password-protected");
    }
}
