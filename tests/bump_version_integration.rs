use {
    scopeguard::defer,
    serial_test::serial,
    std::{fs, path::Path, process::Command},
};

/// A repository with a single crate has no `[workspace.package]`, which used to
/// make `bump-version` bail out before touching anything.
#[test]
#[serial]
fn test_bump_version_single_crate() {
    let root_dir = tempfile::tempdir().unwrap();
    let root_path = fs::canonicalize(root_dir.path()).unwrap();
    let original_dir = std::env::current_dir().unwrap();
    defer! { std::env::set_current_dir(&original_dir).unwrap(); }
    std::env::set_current_dir(&root_path).unwrap();

    Command::new("git").args(["init"]).output().unwrap();

    fs::write(
        root_path.join("Cargo.toml"),
        "[package]\nname = \"solo\"\nversion = \"1.2.3\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir(root_path.join("src")).unwrap();
    fs::write(root_path.join("src/lib.rs"), "").unwrap();

    let lockfile = Command::new("cargo")
        .args(["generate-lockfile"])
        .output()
        .unwrap();
    assert!(
        lockfile.status.success(),
        "generate-lockfile should succeed: {}",
        String::from_utf8_lossy(&lockfile.stderr)
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

    let manifest = fs::read_to_string(root_path.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains(r#"version = "1.2.4""#),
        "package.version should be bumped: {manifest}"
    );

    let lock = fs::read_to_string(root_path.join("Cargo.lock")).unwrap();
    assert!(
        lock.contains(r#"version = "1.2.4""#),
        "lock should be updated: {lock}"
    );
}

#[test]
#[serial]
fn test_bump_version() {
    // get current file path and direct to the playground directory
    let current_file_path_str = file!();
    let root_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace"),
    )
    .unwrap();
    std::env::set_current_dir(&root_path).unwrap();

    // git init is a hack for the bump version command to work
    Command::new("git").args(["init"]).output().unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!("cargo-anza-xtask")
        .args(["anza-xtask", "bump-version", "patch"])
        .unwrap();
    assert!(
        output.status.success(),
        "bump version command should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    defer! {
        fs::remove_dir_all(root_path.join(".git")).unwrap();
        Command::new("git").args(["checkout", "."]).output().unwrap();
    }

    // verfify root/Cargo.toml
    let root_cargo_toml_content = fs::read_to_string(root_path.join("Cargo.toml")).unwrap();
    assert!(
        root_cargo_toml_content.contains(r#"version = "1.2.4""#),
        "workspace.package.version should be bumped to 1.2.4"
    );
    assert!(
        root_cargo_toml_content.contains(r#"a = { path = "a", version = "=1.2.4" }"#),
        "workspace.dependencies.crate-a should be bumped to 1.2.4"
    );
    assert!(
        root_cargo_toml_content.contains(r#"b = { path = "b", version = "=1.2.4" }"#),
        "workspace.dependencies.crate-b should be bumped to 1.2.4"
    );
    assert!(
        root_cargo_toml_content.contains(r#"byte-slice-cast = "=1.2.3""#),
        "non-workspace members' version should not be bumped"
    );
    assert!(
        root_cargo_toml_content.contains(r#"cc = "1.2.3""#),
        "non-workspace members' version should not be bumped"
    );
    assert!(
        root_cargo_toml_content.contains(r#"scopeguard = "1.2.0""#),
        "non-workspace members' version should not be bumped"
    );

    // verify root/Cargo.lock
    let root_cargo_lock_content = fs::read_to_string(root_path.join("Cargo.lock")).unwrap();
    assert!(
        root_cargo_lock_content.contains(r#"version = "1.2.4""#),
        "Cargo.lock should be updated"
    );

    // verify root/d/Cargo.toml
    let d_cargo_toml_content = fs::read_to_string(root_path.join("d/Cargo.toml")).unwrap();
    assert!(
        d_cargo_toml_content.contains(r#"version = "1.2.4""#),
        "d/Cargo.toml should be updated"
    );

    // verify root/d/Cargo.lock
    let d_cargo_lock_content = fs::read_to_string(root_path.join("d/Cargo.lock")).unwrap();
    assert!(
        d_cargo_lock_content.contains(r#"version = "1.2.4""#),
        "d/Cargo.lock should be updated"
    );

    // verify root/sub/Cargo.toml
    let sub_cargo_toml_content = fs::read_to_string(root_path.join("sub/Cargo.toml")).unwrap();
    assert!(
        sub_cargo_toml_content.contains(r#"version = "1.2.4""#),
        "sub/Cargo.toml should be updated"
    );
    assert!(
        sub_cargo_toml_content.contains(r#"c = { path = "c", version = "=1.2.4" }"#),
        "sub/Cargo.toml should be updated"
    );

    // verify root/sub/Cargo.lock
    let sub_cargo_lock_content = fs::read_to_string(root_path.join("sub/Cargo.lock")).unwrap();
    assert!(
        sub_cargo_lock_content.contains(r#"version = "1.2.4""#),
        "sub/Cargo.lock should be updated"
    );
}
