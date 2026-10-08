//! The system print spooler. On macOS and Linux this is CUPS: printers come from `lpstat`, jobs
//! go to `lp` with the job options (copies, collation, duplex, colour). On Windows printers come
//! from `Win32_Printer`, and the caller renders the print-ready PDF's sheets to images that the
//! .NET print system draws ([`submit_sheets`]), with the same job options. Other platforms report
//! that printing isn't available yet; the print-ready PDF can still be saved.

use crate::PrintError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Duplex {
    #[default]
    Off,
    LongEdge,
    ShortEdge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// `None` = the system default printer.
    pub printer: Option<String>,
    pub copies: u32,
    pub collate: bool,
    pub duplex: Duplex,
    pub grayscale: bool,
    pub title: String,
}

impl Default for Job {
    fn default() -> Self {
        Job { printer: None, copies: 1, collate: true, duplex: Duplex::Off, grayscale: false, title: "Sintec.PDF".into() }
    }
}

/// Parse `lpstat -p -d` output.
pub fn parse_lpstat(out: &str) -> Vec<Printer> {
    let default = out.lines().find_map(|l| l.strip_prefix("system default destination:")).map(|s| s.trim().to_string());
    out.lines()
        .filter_map(|l| l.strip_prefix("printer "))
        .filter_map(|l| l.split_whitespace().next())
        .map(|n| Printer { name: n.to_string(), default: default.as_deref() == Some(n) })
        .collect()
}

/// The `lp` arguments for a job printing `file`.
pub fn lp_args(job: &Job, file: &str) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(p) = &job.printer {
        a.extend(["-d".to_string(), p.clone()]);
    }
    a.extend(["-n".to_string(), job.copies.clamp(1, 999).to_string()]);
    a.extend(["-t".to_string(), job.title.clone()]);
    let mut opt = |o: &str| a.extend(["-o".to_string(), o.to_string()]);
    opt(if job.collate { "collate=true" } else { "collate=false" });
    opt(match job.duplex {
        Duplex::Off => "sides=one-sided",
        Duplex::LongEdge => "sides=two-sided-long-edge",
        Duplex::ShortEdge => "sides=two-sided-short-edge",
    });
    if job.grayscale {
        opt("print-color-mode=monochrome");
    }
    // The sheets are already laid out at their final size.
    opt("fit-to-page=false");
    a.push("--".into());
    a.push(file.to_string());
    a
}

/// `lpstat -p -d`, forced to print untranslated messages so [`parse_lpstat`] can read them.
///
/// `LC_ALL`/`LANG=C` is enough on Linux. macOS CUPS ignores them and follows the user's
/// interface language (`AppleLanguages`) unless `SOFTWARE` is set, in which case it uses `LANG`.
pub fn lpstat_command() -> std::process::Command {
    let mut c = std::process::Command::new("lpstat");
    c.args(["-p", "-d"]).env("LC_ALL", "C").env("LANG", "C").env("SOFTWARE", "Sintec.PDF");
    c
}

/// The printers the system knows (empty when there are none or no spooler).
pub fn printers() -> Vec<Printer> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        match lpstat_command().output() {
            Ok(o) => parse_lpstat(&String::from_utf8_lossy(&o.stdout)),
            Err(_) => Vec::new(),
        }
    }
    #[cfg(windows)]
    {
        match powershell(WIN_LIST_PRINTERS, None) {
            Ok(o) if o.status.success() => parse_win_printers(&String::from_utf8_lossy(&o.stdout)),
            _ => Vec::new(),
        }
    }
    #[cfg(not(any(all(unix, not(target_arch = "wasm32")), windows)))]
    {
        Vec::new()
    }
}

/// Whether this platform's spooler takes sheets rendered to images ([`submit_sheets`]) instead of
/// the print-ready PDF ([`submit`]). On Windows there is no built-in way to hand a PDF to a
/// printer, so the caller renders each sheet and the .NET print system draws it.
pub const NEEDS_RASTER: bool = cfg!(windows);

/// Resolution sheets are rendered at for [`submit_sheets`].
pub const RASTER_DPI: f32 = 300.0;

/// One sheet of the print-ready PDF, rendered to a PNG, with its size in points.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterSheet {
    pub png: Vec<u8>,
    pub width_pt: f32,
    pub height_pt: f32,
}

/// Parse the Windows printer list (`<1|0>\t<name>` per line, 1 = the default printer).
pub fn parse_win_printers(out: &str) -> Vec<Printer> {
    out.lines()
        .filter_map(|l| {
            let (flag, name) = l.trim_end_matches('\r').split_once('\t')?;
            let name = name.trim();
            (!name.is_empty()).then(|| Printer { name: name.to_string(), default: flag.trim() == "1" })
        })
        .collect()
}

/// A JSON string literal.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The job description the Windows print script reads: `pages` are `(png path, width pt,
/// height pt)`; `to_file` prints through the driver into a file (e.g. Microsoft Print to PDF).
pub fn win_job_json(job: &Job, pages: &[(String, f32, f32)], to_file: Option<&str>) -> String {
    let duplex = match job.duplex {
        Duplex::Off => "Simplex",
        // .NET names duplex by how the sheet turns: Vertical = long edge for portrait sheets.
        Duplex::LongEdge => "Vertical",
        Duplex::ShortEdge => "Horizontal",
    };
    let pages: Vec<String> =
        pages.iter().map(|(f, w, h)| format!("{{\"file\":{},\"w\":{},\"h\":{}}}", json_str(f), w.max(1.0), h.max(1.0))).collect();
    format!(
        "{{\"printer\":{},\"title\":{},\"copies\":{},\"collate\":{},\"duplex\":\"{duplex}\",\"color\":{},\"file\":{},\"pages\":[{}]}}",
        job.printer.as_deref().map_or("null".to_string(), json_str),
        json_str(&job.title),
        job.copies.clamp(1, 999),
        job.collate,
        !job.grayscale,
        to_file.map_or("null".to_string(), json_str),
        pages.join(",")
    )
}

/// Lists printers as `<1|0>\t<name>` lines, UTF-8.
pub const WIN_LIST_PRINTERS: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
Get-CimInstance Win32_Printer | ForEach-Object { $(if ($_.Default) { '1' } else { '0' }) + "`t" + $_.Name }"#;

/// Prints the sheets described by the JSON file in `$env:SINTEC_PRINT_JOB` through
/// System.Drawing.Printing (the .NET print system every Windows 10/11 has). Each sheet goes on the
/// driver's paper size that matches it (or a custom size), landscape when it is wider than tall,
/// drawn at its exact physical size from the sheet's corner. No progress dialog is shown.
pub const WIN_PRINT_SCRIPT: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
try {
  Add-Type -AssemblyName System.Drawing
  $job = Get-Content -LiteralPath $env:SINTEC_PRINT_JOB -Raw -Encoding UTF8 | ConvertFrom-Json
  $doc = New-Object System.Drawing.Printing.PrintDocument
  $doc.DocumentName = $job.title
  $ps = $doc.PrinterSettings
  if ($job.printer) { $ps.PrinterName = $job.printer }
  if (-not $ps.IsValid) { [Console]::Error.WriteLine("printer not found: " + $ps.PrinterName); exit 3 }
  if ($job.file) { $ps.PrintToFile = $true; $ps.PrintFileName = $job.file }
  $ps.Copies = [int16]$job.copies
  $ps.Collate = [bool]$job.collate
  if ($job.duplex -ne 'Simplex' -and $ps.CanDuplex) { $ps.Duplex = [Enum]::Parse([System.Drawing.Printing.Duplex], $job.duplex) }
  $doc.DefaultPageSettings.Color = [bool]$job.color
  $doc.PrintController = New-Object System.Drawing.Printing.StandardPrintController
  $global:sizes = @($ps.PaperSizes | Where-Object { $_.Kind -ne 'Custom' })
  # The driver's paper that matches a sheet (sizes in 1/100 inch), or a custom size.
  $global:paperFor = {
    param($p)
    $w = [double]$p.w / 72 * 100; $h = [double]$p.h / 72 * 100
    $short = [Math]::Min($w, $h); $long = [Math]::Max($w, $h)
    $best = $null; $bestErr = 1e9
    foreach ($z in $global:sizes) {
      $zs = [Math]::Min($z.Width, $z.Height); $zl = [Math]::Max($z.Width, $z.Height)
      $err = [Math]::Abs($zs - $short) + [Math]::Abs($zl - $long)
      if ($err -lt $bestErr) { $best = $z; $bestErr = $err }
    }
    if ($best -eq $null -or $bestErr -gt 25) { $best = New-Object System.Drawing.Printing.PaperSize('Sheet', [int][Math]::Round($short), [int][Math]::Round($long)) }
    , @($best, ($w -gt $h))
  }
  # Many drivers take the paper for the whole job from the defaults and ignore per-page changes,
  # so the first sheet sets them; QueryPageSettings still switches for mixed-size jobs.
  $first = & $global:paperFor $job.pages[0]
  $doc.DefaultPageSettings.PaperSize = $first[0]
  $doc.DefaultPageSettings.Landscape = $first[1]
  $global:sheet = 0
  $doc.add_QueryPageSettings({
    param($s, $e)
    $pick = & $global:paperFor $job.pages[$global:sheet]
    $e.PageSettings.PaperSize = $pick[0]
    $e.PageSettings.Landscape = $pick[1]
    $e.PageSettings.Color = [bool]$job.color
  })
  $doc.add_PrintPage({
    param($s, $e)
    $p = $job.pages[$global:sheet]
    $img = [System.Drawing.Image]::FromFile($p.file)
    try {
      $g = $e.Graphics
      $g.PageUnit = [System.Drawing.GraphicsUnit]::Display
      $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
      $w = [double]$p.w / 72 * 100; $h = [double]$p.h / 72 * 100
      # The graphics origin is the corner of the printable area: step back by the hard margins so
      # the sheet lands at its true position on the paper.
      $g.DrawImage($img, [single](-$e.PageSettings.HardMarginX), [single](-$e.PageSettings.HardMarginY), [single]$w, [single]$h)
    } finally { $img.Dispose() }
    $global:sheet++
    $e.HasMorePages = $global:sheet -lt $job.pages.Count
  })
  $doc.Print()
  Write-Output ("{0} sheet(s)" -f $job.pages.Count)
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 2
}"#;

/// `-EncodedCommand` text: the script as base64 of UTF-16LE. Inline scripts aren't subject to the
/// execution policy that can block `.ps1` files on managed PCs.
pub fn powershell_encoded(script: &str) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                // In range: a 6-bit value indexes a 64-entry table.
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Run a PowerShell script hidden (no console window), with the job file in its environment.
#[cfg(windows)]
fn powershell(script: &str, job_file: Option<&std::path::Path>) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &powershell_encoded(script)]).creation_flags(CREATE_NO_WINDOW);
    if let Some(f) = job_file {
        c.env("SINTEC_PRINT_JOB", f);
    }
    c.output()
}

/// Print sheets rendered to images (the Windows spooler, see [`NEEDS_RASTER`]). Returns the
/// spooler's message.
pub fn submit_sheets(sheets: &[RasterSheet], job: &Job) -> Result<String, PrintError> {
    submit_sheets_to(sheets, job, None)
}

/// [`submit_sheets`], optionally printing through the driver into `to_file` instead of paper
/// (with a file-writing driver such as Microsoft Print to PDF).
pub fn submit_sheets_to(sheets: &[RasterSheet], job: &Job, to_file: Option<&std::path::Path>) -> Result<String, PrintError> {
    if sheets.is_empty() {
        return Err(PrintError::NoPages);
    }
    #[cfg(windows)]
    {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("sintec-pdf-print-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| PrintError::Spool(e.to_string()))?;
        let result = (|| {
            let mut pages = Vec::with_capacity(sheets.len());
            for (i, s) in sheets.iter().enumerate() {
                let f = dir.join(format!("sheet-{i:05}.png"));
                std::fs::write(&f, &s.png).map_err(|e| PrintError::Spool(e.to_string()))?;
                pages.push((f.to_string_lossy().into_owned(), s.width_pt, s.height_pt));
            }
            let json = dir.join("job.json");
            let to_file = to_file.map(|p| p.to_string_lossy().into_owned());
            std::fs::write(&json, win_job_json(job, &pages, to_file.as_deref())).map_err(|e| PrintError::Spool(e.to_string()))?;
            let out = powershell(WIN_PRINT_SCRIPT, Some(&json))
                .map_err(|e| PrintError::Spool(format!("the Windows print system is not available: {e}")))?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(PrintError::Spool(if err.is_empty() { "the print job was refused".into() } else { err }))
            }
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (job, to_file);
        Err(PrintError::Spool("printing rendered sheets is only needed on Windows; use submit".into()))
    }
}

/// Send a print-ready PDF to the spooler. Returns the spooler's message (the job id).
pub fn submit(pdf: &[u8], job: &Job) -> Result<String, PrintError> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        let dir = std::env::temp_dir().join(format!("pdfcraft-print-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| PrintError::Spool(e.to_string()))?;
        let file = dir.join(format!("job-{}.pdf", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
        std::fs::write(&file, pdf).map_err(|e| PrintError::Spool(e.to_string()))?;
        let out = std::process::Command::new("lp").args(lp_args(job, &file.to_string_lossy())).output();
        let _ = std::fs::remove_file(&file);
        let out = out.map_err(|e| PrintError::Spool(format!("the print spooler is not available: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(PrintError::Spool(if err.is_empty() { "the print job was refused".into() } else { err }))
        }
    }
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    {
        let _ = (pdf, job);
        Err(PrintError::Spool(if NEEDS_RASTER {
            "this platform prints rendered sheets: use submit_sheets".into()
        } else {
            "printing to a printer isn't available on this platform yet; save the print-ready PDF instead".into()
        }))
    }
}
