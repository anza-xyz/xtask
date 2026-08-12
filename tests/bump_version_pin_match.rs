use {
    scopeguard::defer,
    serial_test::serial,
    std::{fs, path::Path, process::Command},
};

/// `b` is pinned `=12.0.1` while the workspace is at `2.0.1`. Substring matching
/// used to rewrite that pin to `=12.0.2`, a version `b` never publishes.
#[test]
#[serial]
fn test_bump_version_leaves_unrelated_pins_alone() {
    let root_path = fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dummy-workspace-pin-match"),
    )
    .unwrap();
    std::env::set_current_dir(&root_path).unwrap();

    Command::new("git").args(["init"]).output().unwrap();
    defer! {
        fs::remove_dir_all(root_path.join(".git")).unwrap();
        Command::new("git").args(["checkout", "."]).output().unwrap();
    }

    // Guards against committing the fixture in a bumped state, which a failed
    // run can leave behind.
    let before = fs::read_to_string(root_path.join("Cargo.toml")).unwrap();
    assert!(
        before.contains(r#"version = "2.0.1""#),
        "fixture should start at 2.0.1: {before}"
    );

    let output = assert_cmd::cargo::cargo_bin_cmd!("cargo-anza-xtask")
        .args(["anza-xtask", "bump-version", "patch"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bump version should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let root_manifest = fs::read_to_string(root_path.join("Cargo.toml")).unwrap();
    assert!(
        root_manifest.contains(r#"version = "2.0.2""#),
        "workspace.package.version should be bumped: {root_manifest}"
    );
    assert!(
        root_manifest.contains(r#"a = { path = "a", version = "=2.0.2" }"#),
        "a pins the workspace version and should be bumped: {root_manifest}"
    );
    assert!(
        root_manifest.contains(r#"b = { path = "b", version = "=12.0.1" }"#),
        "b pins an unrelated version and should not be touched: {root_manifest}"
    );

    let b_manifest = fs::read_to_string(root_path.join("b/Cargo.toml")).unwrap();
    assert!(
        b_manifest.contains(r#"version = "12.0.1""#),
        "b's own version should not be touched: {b_manifest}"
    );

    let lock = fs::read_to_string(root_path.join("Cargo.lock")).unwrap();
    assert!(
        lock.contains(r#"version = "2.0.2""#),
        "lock should be updated"
    );
    assert!(
        lock.contains(r#"version = "12.0.1""#),
        "b should stay at its own version in the lock"
    );
}
