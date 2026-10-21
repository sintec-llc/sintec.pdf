//! System print spooler supplied by craft-print. PDF composition and permission checks
//! stay in PdfCraft; these adapters retain the application's print-error surface.
//!
//! Sintec.PDF prints its own way on Windows: the caller renders the print-ready PDF's sheets to
//! images that the .NET print system draws ([`submit_sheets`]), with the same job options, and the
//! printer list comes from the same place. Both run PowerShell with the script on standard input
//! ([`run_powershell`]), never on the command line: antivirus behaviour engines (Kaspersky's PDM)
//! flag a hidden PowerShell started with a script in its arguments. craft-print's own Windows path
//! (a WinRT print script passed as `-Command <script>`) is not used.

use crate::PrintError;

pub use craft_print::{
    Duplex, HANDLED_OPTIONS, HAS_PRINTER_PREFERENCES, Job, Printer, PrinterOption, Scaling, lp_args, lpstat_command, parse_lpoptions, parse_lpstat,
    parse_lpstat_e, parse_ppd, parse_windows_printers, ppd_text, printers_parsed,
};

/// The printers the Print dialog offers (Windows: through [`run_powershell`]).
pub fn printers() -> Vec<Printer> {
    #[cfg(windows)]
    {
        match run_powershell(WIN_LIST_PRINTERS, &[]) {
            Ok(o) if o.status.success() => parse_windows_printers(&String::from_utf8_lossy(&o.stdout)),
            _ => Vec::new(),
        }
    }
    #[cfg(not(windows))]
    {
        craft_print::printers()
    }
}

/// The driver's job options for a printer (CUPS: its PPD, with the queue's current defaults);
/// empty when it has none or on other platforms. Reads files and runs `lpoptions`, so call it
/// when the user asks, not every frame.
pub fn printer_options(printer: &str) -> Vec<PrinterOption> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        printer_options_via(printer, std::path::Path::new("/etc/cups/ppd"), std::process::Command::new("lpoptions"))
    }
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    {
        craft_print::printer_options(printer)
    }
}

/// Read a destination's options with the PPD directory and command supplied, so tests need
/// neither a system queue nor a physical printer. CUPS instances share their queue's PPD but
/// have their own defaults, queried with the complete `queue/instance` destination.
#[cfg(all(unix, not(target_arch = "wasm32")))]
pub(crate) fn printer_options_via(printer: &str, ppd_dir: &std::path::Path, mut lpoptions: std::process::Command) -> Vec<PrinterOption> {
    let (queue, instance) = printer.split_once('/').map_or((printer, None), |(q, i)| (q, Some(i)));
    // Retain the conservative name checks, applied to each component. Neither component is
    // a path: reject dot components, backslashes and any additional separator before I/O.
    let valid = |name: &str| {
        !name.is_empty()
            && name.len() <= 64
            && !matches!(name, "." | "..")
            && name.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b'\\' | b':' | b'=' | b'"'))
    };
    if !valid(queue) || instance.is_some_and(|name| !valid(name)) {
        return Vec::new();
    }
    const PPD_CAP: u64 = 16 << 20;
    let ppd = std::fs::File::open(ppd_dir.join(format!("{queue}.ppd"))).ok().and_then(|f| {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        f.take(PPD_CAP).read_to_end(&mut bytes).ok().map(|_| bytes)
    });
    let mut options = ppd.map(|b| parse_ppd(&ppd_text(&b))).unwrap_or_default();
    let current = lpoptions.args(["-p", printer, "-l"]).output().map(|o| parse_lpoptions(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
    for (key, value) in current {
        if let Some(o) = options.iter_mut().find(|o| o.key == key)
            && o.choices.iter().any(|(c, _)| *c == value)
        {
            o.default = value;
        }
    }
    options
}

/// Send a print-ready PDF to the spooler (CUPS). On Windows the caller prints rendered sheets
/// instead ([`NEEDS_RASTER`], [`submit_sheets`]).
pub fn submit(pdf: &[u8], job: &Job) -> Result<String, PrintError> {
    if NEEDS_RASTER {
        let _ = (pdf, job);
        return Err(PrintError::Spool("this platform prints rendered sheets: use submit_sheets".into()));
    }
    craft_print::submit(pdf, job).map_err(|e| PrintError::Spool(e.to_string()))
}

pub fn open_printer_preferences(printer: &str) -> Result<(), PrintError> {
    craft_print::open_printer_preferences(printer).map_err(|e| PrintError::Spool(e.to_string()))
}

/// Whether this platform's spooler takes sheets rendered to images ([`submit_sheets`]) instead of
/// the print-ready PDF ([`submit`]). On Windows the caller renders each sheet and the .NET print
/// system draws it.
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
/// Two-sided `None` keeps the driver's own setting; shrink-to-fit scales a sheet larger than the
/// paper's printable area down to it.
pub fn win_job_json(job: &Job, pages: &[(String, f32, f32)], to_file: Option<&str>) -> String {
    let duplex = match job.duplex {
        None => "Default",
        Some(Duplex::Off) => "Simplex",
        // .NET names duplex by how the sheet turns: Vertical = long edge for portrait sheets.
        Some(Duplex::LongEdge) => "Vertical",
        Some(Duplex::ShortEdge) => "Horizontal",
    };
    let pages: Vec<String> =
        pages.iter().map(|(f, w, h)| format!("{{\"file\":{},\"w\":{},\"h\":{}}}", json_str(f), w.max(1.0), h.max(1.0))).collect();
    format!(
        "{{\"printer\":{},\"title\":{},\"copies\":{},\"collate\":{},\"duplex\":\"{duplex}\",\"color\":{},\"fit\":{},\"file\":{},\"pages\":[{}]}}",
        job.printer.as_deref().map_or("null".to_string(), json_str),
        json_str(&job.title),
        job.copies.clamp(1, 999),
        job.collate,
        !job.grayscale,
        job.scaling == Scaling::ShrinkToFit,
        to_file.map_or("null".to_string(), json_str),
        pages.join(",")
    )
}

/// Lists printers as craft-print's [`parse_windows_printers`] reads them: one per line, `*` before
/// the default, then a tab and `2`/`1` for duplex support. UTF-8.
pub const WIN_LIST_PRINTERS: &str = r#"[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Get-CimInstance Win32_Printer -Property Name, Default, Capabilities | ForEach-Object {
  $sides = if (@($_.Capabilities) -contains 3) { '2' } else { '1' }
  if ($_.Default) { '*' + $_.Name + [char]9 + $sides } else { $_.Name + [char]9 + $sides }
}"#;

/// Prints the sheets described by the JSON file in `$env:SINTEC_PRINT_JOB` through
/// System.Drawing.Printing (the .NET print system every Windows 10/11 has). Each sheet goes on the
/// driver's paper size that matches it (or a custom size), landscape when it is wider than tall,
/// drawn at its exact physical size from the sheet's corner, or shrunk to the printable area when
/// the job asks. No progress dialog is shown.
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
  if ($job.duplex -eq 'Simplex') { $ps.Duplex = [System.Drawing.Printing.Duplex]::Simplex }
  elseif ($job.duplex -ne 'Default' -and $ps.CanDuplex) { $ps.Duplex = [Enum]::Parse([System.Drawing.Printing.Duplex], $job.duplex) }
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
      $x = -$e.PageSettings.HardMarginX; $y = -$e.PageSettings.HardMarginY
      if ($job.fit) {
        # Shrink to fit: a sheet larger than the printable area is scaled down and centred on it.
        $area = $e.PageSettings.PrintableArea
        $aw = if ($e.PageSettings.Landscape) { $area.Height } else { $area.Width }
        $ah = if ($e.PageSettings.Landscape) { $area.Width } else { $area.Height }
        $k = [Math]::Min(1.0, [Math]::Min($aw / $w, $ah / $h))
        if ($k -lt 1.0) { $w = $w * $k; $h = $h * $k; $x = ($aw - $w) / 2; $y = ($ah - $h) / 2 }
      }
      $g.DrawImage($img, [single]$x, [single]$y, [single]$w, [single]$h)
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

/// Run a PowerShell script hidden (no console window). The script goes in on standard input
/// (`-Command -`), wrapped in `& { }` so it parses as one block, as a script file would: never as a
/// base64 `-EncodedCommand` or a script in the arguments, which antivirus behaviour engines
/// (Kaspersky's PDM) read as malware. Inline scripts aren't subject to the execution policy that
/// can block `.ps1` files on managed PCs. The script must be ASCII (PowerShell reads standard input
/// in the console code page); data such as paths goes in `env`.
#[cfg(windows)]
pub fn run_powershell(script: &str, env: &[(&str, &std::ffi::OsStr)]) -> std::io::Result<std::process::Output> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !script.is_ascii() {
        return Err(std::io::Error::other("PowerShell scripts must be ASCII"));
    }
    // System32's PowerShell, not whatever a writable folder earlier on PATH holds.
    let exe = std::env::var_os("SystemRoot").map_or_else(
        || std::path::PathBuf::from("powershell.exe"),
        |root| std::path::PathBuf::from(root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
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
    // Two newlines end the last statement; written on its own thread so a script that prints a
    // lot before reading all of its input can't deadlock against a full output pipe.
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
            let out = run_powershell(WIN_PRINT_SCRIPT, &[("SINTEC_PRINT_JOB", json.as_os_str())])
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
