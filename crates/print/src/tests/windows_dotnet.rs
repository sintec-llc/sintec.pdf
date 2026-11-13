//! The .NET print path: the job description and the scripts PowerShell reads on standard input.

use crate::spool::{self, Duplex, Job};

#[test]
fn the_job_description_names_everything_the_script_reads() {
    let job = Job {
        printer: Some("Office \"Laser\"".into()),
        copies: 2,
        collate: true,
        duplex: Some(Duplex::ShortEdge),
        scaling: spool::Scaling::ShrinkToFit,
        grayscale: true,
        title: "Счёт.pdf".into(),
        ..Job::default()
    };
    let json = spool::win_job_json(&job, &[("C:\\tmp\\sheet-00000.png".into(), 595.0, 842.0)], None);
    assert_eq!(
        json,
        r#"{"printer":"Office \"Laser\"","title":"Счёт.pdf","copies":2,"collate":true,"duplex":"Horizontal","color":false,"fit":true,"file":null,"pages":[{"file":"C:\\tmp\\sheet-00000.png","w":595,"h":842}]}"#
    );
    // The defaults: the driver's own two-sided setting, the sheet at its exact size.
    let default = spool::win_job_json(&Job::default(), &[], Some("C:\\out.pdf"));
    assert!(
        default.starts_with(r#"{"printer":null,"#)
            && default.contains(r#""duplex":"Default""#)
            && default.contains(r#""fit":false"#)
            && default.contains(r#""file":"C:\\out.pdf""#),
        "{default}"
    );
    let simplex = spool::win_job_json(&Job { duplex: Some(Duplex::Off), ..Job::default() }, &[], None);
    assert!(simplex.contains(r#""duplex":"Simplex""#), "{simplex}");
}

/// The scripts go to PowerShell on standard input, which reads it in the console code page: they
/// must be ASCII (paths and names travel in the job file), and never become `-Command <script>`.
#[test]
fn powershell_scripts_are_ascii() {
    assert!(spool::WIN_PRINT_SCRIPT.contains("SINTEC_PRINT_JOB") && spool::WIN_PRINT_SCRIPT.contains("StandardPrintController"));
    assert!(spool::WIN_PRINT_SCRIPT.is_ascii(), "print script");
    assert!(spool::WIN_LIST_PRINTERS.is_ascii(), "printer list script");
}

#[test]
fn rendering_sheets_without_any_is_refused() {
    assert!(matches!(spool::submit_sheets(&[], &Job::default()), Err(crate::PrintError::NoPages)));
}
