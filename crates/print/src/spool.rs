//! The system print spooler. On macOS and Linux this is CUPS: printers come from `lpstat` — its
//! queues, and the driverless destinations it can print to without one — and jobs are piped to
//! `lp` with the job options (copies, collation, duplex, colour). On Windows printers come from
//! `Win32_Printer`, and the caller renders the print-ready PDF's sheets to images that the .NET
//! print system draws ([`submit_sheets`]), with the same job options. Other platforms report that
//! printing isn't available yet; the print-ready PDF can still be saved.

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
    /// The printer driver's own job options (a PPD keyword and the chosen value), such as the
    /// paper tray or paper type; see [`printer_options`]. CUPS only.
    pub options: Vec<(String, String)>,
}

impl Default for Job {
    fn default() -> Self {
        Job { printer: None, copies: 1, collate: true, duplex: Duplex::Off, grayscale: false, title: "Sintec.PDF".into(), options: Vec::new() }
    }
}

/// The `lpstat -d` line's destination name, when the spooler has a default.
fn default_destination(out: &str) -> Option<&str> {
    out.lines().find_map(|l| l.strip_prefix("system default destination:")).map(str::trim)
}

/// Parse `lpstat -p -d` output.
pub fn parse_lpstat(out: &str) -> Vec<Printer> {
    let default = default_destination(out);
    out.lines()
        .filter_map(|l| l.strip_prefix("printer "))
        .filter_map(|l| l.split_whitespace().next())
        .map(|n| Printer { name: n.to_string(), default: default == Some(n) })
        .collect()
}

/// Parse `lpstat -e` output: destination names, one per line.
pub fn parse_lpstat_e(out: &str) -> Vec<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// The `lp` arguments for a job. There is no file argument: `lp` reads the job from stdin.
pub fn lp_args(job: &Job) -> Vec<String> {
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
    for (key, value) in &job.options {
        // Driver options are PPD keywords; anything else is not something lp can be given.
        if is_ppd_keyword(key) && is_ppd_keyword(value) && !HANDLED_OPTIONS.contains(&key.as_str()) {
            a.extend(["-o".to_string(), format!("{key}={value}")]);
        }
    }
    a
}

/// One of the printer driver's job options (a PPD `OpenUI` entry), such as the paper tray.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterOption {
    /// The PPD keyword (`InputSlot`).
    pub key: String,
    /// What the driver calls it (`Paper tray`); the keyword when it gives no name.
    pub label: String,
    /// The driver's group (`Paper Handling`); empty outside a group.
    pub group: String,
    /// The choices: keyword and label (`Tray2`, `Tray 2`).
    pub choices: Vec<(String, String)>,
    /// The choice the printer uses when a job doesn't say.
    pub default: String,
}

/// Options PdfCraft's Print dialog sets itself (paper, two-sided, collation), so the driver's
/// copies of them are neither shown nor sent.
pub const HANDLED_OPTIONS: &[&str] = &["PageSize", "PageRegion", "Duplex", "Collate", "ImageableArea", "PaperDimension"];

/// Caps on what a PPD may make the dialog hold (PPDs are text files from the printer vendor).
const MAX_OPTIONS: usize = 400;
const MAX_CHOICES: usize = 400;

/// A PPD keyword: printable ASCII without spaces, `/`, `:` or `=`.
fn is_ppd_keyword(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b':' | b'=' | b'"'))
}

/// `Name/Label` → (`Name`, `Label`), the label falling back to the name.
fn name_label(s: &str) -> (&str, &str) {
    match s.split_once('/') {
        Some((n, l)) if !l.trim().is_empty() => (n.trim(), l.trim()),
        Some((n, _)) => (n.trim(), n.trim()),
        None => (s.trim(), s.trim()),
    }
}

/// The job options a PPD offers, in file order, without the installable-hardware group and the
/// options PdfCraft sets itself ([`HANDLED_OPTIONS`]). Lenient: malformed lines are skipped.
pub fn parse_ppd(ppd: &str) -> Vec<PrinterOption> {
    let mut out: Vec<PrinterOption> = Vec::new();
    let (mut group, mut group_key) = (String::new(), String::new());
    let mut open: Option<PrinterOption> = None;
    for line in ppd.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("*OpenGroup:") {
            let (k, l) = name_label(rest);
            (group_key, group) = (k.to_string(), l.to_string());
        } else if line.starts_with("*CloseGroup") {
            (group_key, group) = (String::new(), String::new());
        } else if let Some(rest) = line.strip_prefix("*OpenUI") {
            let Some((head, _kind)) = rest.trim().rsplit_once(':') else { continue };
            let (key, label) = name_label(head.trim().trim_start_matches('*'));
            open = is_ppd_keyword(key).then(|| PrinterOption {
                key: key.to_string(),
                label: label.to_string(),
                group: group.clone(),
                choices: Vec::new(),
                default: String::new(),
            });
        } else if line.starts_with("*CloseUI") {
            if let Some(o) = open.take()
                && !o.choices.is_empty()
                && group_key != "InstallableOptions"
                && !HANDLED_OPTIONS.contains(&o.key.as_str())
                && out.len() < MAX_OPTIONS
            {
                out.push(o);
            }
        } else if let Some(o) = open.as_mut() {
            let Some(rest) = line.strip_prefix('*') else { continue };
            if let Some(value) = rest.strip_prefix("Default").and_then(|r| r.strip_prefix(o.key.as_str())).and_then(|r| r.strip_prefix(':')) {
                o.default = value.trim().to_string();
            } else if let Some(rest) = rest.strip_prefix(o.key.as_str()).and_then(|r| r.strip_prefix(' ')) {
                // `*InputSlot Tray2/Tray 2: "<< ... >>"`; the code after the colon is the driver's.
                let head = rest.split_once(':').map_or(rest, |(h, _)| h);
                let (choice, label) = name_label(head);
                if is_ppd_keyword(choice) && o.choices.len() < MAX_CHOICES && o.choices.iter().all(|(c, _)| c != choice) {
                    o.choices.push((choice.to_string(), label.to_string()));
                }
            }
        }
    }
    for o in &mut out {
        if !o.choices.iter().any(|(c, _)| *c == o.default) {
            o.default = o.choices.first().map(|(c, _)| c.clone()).unwrap_or_default();
        }
    }
    out
}

/// The current choice of each option in `lpoptions -p NAME -l` output (`Key/Label: a *b c`): the
/// queue's defaults, including the user's own (`~/.cups/lpoptions`).
pub fn parse_lpoptions(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let (head, choices) = l.split_once(':')?;
            let (key, _) = name_label(head);
            let current = choices.split_whitespace().find_map(|c| c.strip_prefix('*'))?;
            Some((key.to_string(), current.to_string()))
        })
        .collect()
}

/// A PPD's text: UTF-8, or ISO Latin-1 (the PPD default) when it isn't valid UTF-8.
pub fn ppd_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// The driver's job options for a printer (CUPS: its PPD, with the queue's current defaults);
/// empty when it has none or on other platforms. Reads files and runs `lpoptions`, so call it
/// when the user asks, not every frame.
pub fn printer_options(printer: &str) -> Vec<PrinterOption> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        // Queue names never contain `/`; anything else is not a queue and not a path to follow.
        if !is_ppd_keyword(printer) {
            return Vec::new();
        }
        const PPD_CAP: u64 = 16 << 20;
        let ppd = std::fs::File::open(format!("/etc/cups/ppd/{printer}.ppd")).ok().and_then(|f| {
            use std::io::Read as _;
            let mut bytes = Vec::new();
            f.take(PPD_CAP).read_to_end(&mut bytes).ok().map(|_| bytes)
        });
        let mut options = ppd.map(|b| parse_ppd(&ppd_text(&b))).unwrap_or_default();
        let current = std::process::Command::new("lpoptions")
            .args(["-p", printer, "-l"])
            .output()
            .map(|o| parse_lpoptions(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        for (key, value) in current {
            if let Some(o) = options.iter_mut().find(|o| o.key == key)
                && o.choices.iter().any(|(c, _)| *c == value)
            {
                o.default = value;
            }
        }
        options
    }
    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    {
        let _ = printer;
        Vec::new()
    }
}

/// Whether [`open_printer_preferences`] can show the driver's own settings window (Windows).
pub const HAS_PRINTER_PREFERENCES: bool = cfg!(windows);

/// Open the printer driver's Printing Preferences window (Windows: `printui.dll`). What the user
/// sets there is saved as that printer's defaults for this Windows user, which every job starts
/// from; the Print dialog's copies, two-sided, grayscale and paper still apply on top.
pub fn open_printer_preferences(printer: &str) -> Result<(), PrintError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // printui takes the name in quotes; Windows printer names can't contain one anyway.
        if printer.is_empty() || printer.contains('"') {
            return Err(PrintError::Spool("not a printer name".into()));
        }
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let rundll = std::path::Path::new(&root).join("System32").join("rundll32.exe");
        std::process::Command::new(rundll)
            .raw_arg("printui.dll,PrintUIEntry")
            .raw_arg("/e")
            .raw_arg("/n")
            .raw_arg(format!("\"{printer}\""))
            .spawn()
            .map(|_| ())
            .map_err(|e| PrintError::Spool(format!("the printer's preferences could not be opened: {e}")))
    }
    #[cfg(not(windows))]
    {
        let _ = printer;
        Err(PrintError::Spool("printer preferences open only on Windows; use the printer's options instead".into()))
    }
}

/// `lpstat`, forced to print untranslated messages so [`parse_lpstat`] and [`parse_lpstat_e`] can
/// read them.
///
/// `LC_ALL`/`LANG=C` is enough on Linux. macOS CUPS ignores them and follows the user's
/// interface language (`AppleLanguages`) unless `SOFTWARE` is set, in which case it uses `LANG`.
fn lpstat_with(args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new("lpstat");
    c.args(args).env("LC_ALL", "C").env("LANG", "C").env("SOFTWARE", "Sintec.PDF");
    c
}

/// `lpstat -p -d`: the spooler's queues and the default destination.
pub fn lpstat_command() -> std::process::Command {
    lpstat_with(&["-p", "-d"])
}

/// `lpstat -e`: every destination CUPS can print to, including driverless network printers no
/// permanent queue exists for (a queue is built only when a job needs one). GTK's and macOS's
/// print dialogs list them the same way. CUPS ≥ 1.7 (2013).
///
/// Gated like its only caller, `printers`' unix block: off unix this is dead code, and the
/// Windows clippy gate builds with `-D warnings`.
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn lpstat_e_command() -> std::process::Command {
    lpstat_with(&["-e"])
}

/// The printers the system knows (empty when there are none or no spooler): its queues, then the
/// driverless destinations CUPS can print to without one.
pub fn printers() -> Vec<Printer> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        let Some(queues) = lpstat_command().output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()) else {
            return Vec::new();
        };
        // A spooler without `-e`, or one whose discovery times out, only loses the driverless names.
        let available = lpstat_e_command().output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        printers_parsed(&queues, available.as_deref())
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

/// Run a PowerShell script hidden (no console window). The script goes in on standard input
/// (`-Command -`), wrapped in `& { }` so it parses as one block, as a script file would: never as a
/// base64 `-EncodedCommand`, which antivirus behaviour engines (Kaspersky's PDM) read as malware.
/// Inline scripts aren't subject to the execution policy that can block `.ps1` files on managed
/// PCs. The script must be ASCII (PowerShell reads standard input in the console code page);
/// data such as paths goes in `env`.
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

/// Run a printing script hidden, with the job file in its environment.
#[cfg(windows)]
fn powershell(script: &str, job_file: Option<&std::path::Path>) -> std::io::Result<std::process::Output> {
    match job_file {
        Some(f) => run_powershell(script, &[("SINTEC_PRINT_JOB", f.as_os_str())]),
        None => run_powershell(script, &[]),
    }
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

/// The Print dialog's printers from lpstat's two answers: the spooler's queues (`-p -d`), then
/// the driverless destinations of `available` (`-e`) that have no queue — CUPS builds a temporary
/// queue when `lp` sends them a job. Each destination once, queues first; the default still comes
/// from `-d`, so a driverless default keeps its marker. `None` when the spooler doesn't know `-e`
/// (CUPS < 1.7, 2013), which loses only the driverless names.
pub fn printers_parsed(queues: &str, available: Option<&str>) -> Vec<Printer> {
    let mut printers = parse_lpstat(queues);
    let default = default_destination(queues);
    if let Some(names) = available {
        for name in parse_lpstat_e(names) {
            if !printers.iter().any(|p| p.name == name) {
                printers.push(Printer { default: default == Some(name.as_str()), name });
            }
        }
    }
    printers
}

/// Send a print-ready PDF to the spooler. Returns the spooler's message (the job id).
pub fn submit(pdf: &[u8], job: &Job) -> Result<String, PrintError> {
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    {
        submit_via(std::process::Command::new("lp"), pdf, job)
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

/// [`submit`] with the spooler command given, so tests can stand in for `lp`.
///
/// The job goes to `lp` on stdin, never through a file: a predictable job folder in a shared
/// temp directory lets another local user read printed documents or swap the file before `lp`
/// reads it.
#[cfg(any(test, all(unix, not(target_arch = "wasm32"))))]
pub(crate) fn submit_via(mut lp: std::process::Command, pdf: &[u8], job: &Job) -> Result<String, PrintError> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = lp
        .args(lp_args(job))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PrintError::Spool(format!("the print spooler is not available: {e}")))?;
    let stdin = child.stdin.take();
    // Feed the job on its own thread while collecting the output, so neither side can fill a
    // pipe and wait on the other. Dropping `stdin` at the end closes it: the end of the job.
    let fed_and_out = std::thread::scope(|s| {
        let feeder = std::thread::Builder::new().name("print job".into()).spawn_scoped(s, move || match stdin {
            Some(mut stdin) => stdin.write_all(pdf),
            None => Err(std::io::Error::other("no pipe to the spooler")),
        });
        match feeder {
            Ok(feeder) => {
                let out = child.wait_with_output();
                Ok((feeder.join(), out))
            }
            Err(e) => {
                // No thread to feed it: stop `lp` rather than leave it with an empty job.
                let _ = child.kill();
                let _ = child.wait();
                Err(PrintError::Spool(format!("the print job could not be sent to the spooler: {e}")))
            }
        }
    });
    let (fed, out) = fed_and_out?;
    let out = out.map_err(|e| PrintError::Spool(format!("the print spooler is not available: {e}")))?;
    if !out.status.success() {
        // `lp` may refuse before reading the job (an unknown printer); its message says why,
        // and the broken pipe that leaves behind does not.
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(PrintError::Spool(if err.is_empty() { "the print job was refused".into() } else { err }));
    }
    match fed {
        Ok(Ok(())) => Ok(job_message(&out.stdout)),
        Ok(Err(e)) => Err(PrintError::Spool(format!("the print job could not be sent to the spooler: {e}"))),
        Err(_) => Err(PrintError::Spool("the print job could not be sent to the spooler".into())),
    }
}

/// `lp`'s reply ("request id is Office-12"), without the "(0 file(s))" CUPS adds when the job came
/// on stdin: it reads as if nothing was sent.
#[cfg(any(test, all(unix, not(target_arch = "wasm32"))))]
pub(crate) fn job_message(stdout: &[u8]) -> String {
    let s = String::from_utf8_lossy(stdout);
    let s = s.trim();
    s.strip_suffix("(0 file(s))").map_or(s, str::trim_end).to_string()
}
