//! Release and source Flatpaks must allow release checks and host update helpers.
fn finish_args(manifest: &str) -> Vec<&str> {
    manifest
        .split("finish-args:")
        .nth(1)
        .unwrap_or_default()
        .lines()
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('#'))
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|arg| arg.trim_matches(['\'', '"']))
        .collect()
}

#[test]
fn flatpak_update_permissions() {
    let manifests = [
        include_str!("../../../packaging/linux/flatpak/io.github.sintec_llc.sintec_pdf.bundle.yml"),
        include_str!("../../../packaging/linux/flatpak/io.github.sintec_llc.sintec_pdf.yml"),
    ];
    let args = manifests.map(finish_args);
    assert_eq!(args[0], args[1], "source and release permissions must agree");
    for permissions in args {
        for required in ["--share=network", "--talk-name=org.freedesktop.Flatpak"] {
            assert!(permissions.contains(&required), "missing update permission: {required}");
        }
    }
}

/// Both manifests carry Sintec.PDF's own app id, so a Flatpak never replaces or shares data with
/// an upstream PdfCraft, and the files they install are named after it.
#[test]
fn flatpak_manifests_use_the_sintec_app_id() {
    const ID: &str = "io.github.sintec_llc.sintec_pdf";
    for manifest in [
        include_str!("../../../packaging/linux/flatpak/io.github.sintec_llc.sintec_pdf.bundle.yml"),
        include_str!("../../../packaging/linux/flatpak/io.github.sintec_llc.sintec_pdf.yml"),
    ] {
        assert!(manifest.lines().any(|l| l == format!("id: {ID}")), "manifest id");
        assert!(manifest.lines().any(|l| l == "command: sintec-pdf"), "manifest command");
        assert!(!manifest.contains("ai.storyteller"), "upstream's id must not come back");
    }
}
