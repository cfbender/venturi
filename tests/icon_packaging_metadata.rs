use std::fs;
use std::path::Path;

fn read_repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn deb_assets_include_app_icon_svg() {
    let cargo_toml = read_repo_file("Cargo.toml");
    assert!(cargo_toml.contains(
        "[\"data/org.venturi.Venturi.svg\", \"usr/share/icons/hicolor/scalable/apps/\", \"644\"]"
    ));
}

#[test]
fn duplicate_brand_logo_svg_is_removed() {
    let duplicate = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/venturi-logo.svg");
    assert!(
        !duplicate.exists(),
        "duplicate icon should be removed: {}",
        duplicate.display()
    );
}
