use {
    serial_test::serial,
    std::{fs, path::Path},
};

#[test]
#[serial]
fn test_dev_deps_check_detects_workspace_true() {
    // Workspace 1: has issue - b uses workspace = true for a in dev-deps
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-1"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should fail because a is a workspace member
    assert!(
        !output.status.success(),
        "Should detect dev-dependency using workspace = true for workspace member"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("a") && stderr.contains("dev-dependencies should use path"),
        "Error message should mention a and suggest using path. Got: {}",
        stderr
    );
    assert!(
        stderr.contains("b/Cargo.toml"),
        "Should mention b location. Got: {}",
        stderr
    );
}

#[test]
#[serial]
fn test_dev_deps_check_allows_external_workspace_deps() {
    // Workspace 2: crate-a uses workspace = true for external dep (serde)
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-2"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should succeed because serde is not a workspace member
    assert!(
        output.status.success(),
        "Should allow external dependencies with workspace = true. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[serial]
fn test_dev_deps_check_multiple_issues() {
    // Workspace 3: multiple issues - 3 total
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-3"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should fail and report all 3 issues
    assert!(!output.status.success(), "Should detect multiple issues");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Found 3 dev-dependencies"),
        "Should report 3 issues. Got: {}",
        stderr
    );

    // Verify all issues are reported
    assert!(
        stderr.contains("b/Cargo.toml") && stderr.contains("`a`"),
        "Should mention b with a. Got: {}",
        stderr
    );
    assert!(
        stderr.contains("c/Cargo.toml") && stderr.contains("`a`"),
        "Should mention c with a. Got: {}",
        stderr
    );
    assert!(
        stderr.contains("c/Cargo.toml") && stderr.contains("`b`"),
        "Should mention c with b. Got: {}",
        stderr
    );
}

#[test]
#[serial]
fn test_dev_deps_check_no_issues() {
    // Workspace 2: no issues - correct usage
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-2"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should succeed - no issues
    assert!(
        output.status.success(),
        "Should pass when no issues found. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[serial]
fn test_dev_deps_check_ignores_regular_dependencies() {
    // Workspace 4: has regular dependency with workspace = true (should NOT be flagged)
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-4"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should succeed - regular dependencies should not be checked
    assert!(
        output.status.success(),
        "Should ignore regular dependencies with workspace = true. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[serial]
fn test_dev_deps_check_detects_table_format() {
    // Workspace 5: uses table format [dev-dependencies.a] instead of inline
    let current_file_path_str = file!();
    let workspace_path = fs::canonicalize(
        Path::new(current_file_path_str)
            .parent()
            .unwrap()
            .join("dummy-workspace-dev-deps-5"),
    )
    .unwrap();

    let output = assert_cmd::cargo::cargo_bin_cmd!()
        .args([
            "dev-deps",
            "check",
            "--manifest-path",
            workspace_path.join("Cargo.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should fail - table format should also be detected
    assert!(
        !output.status.success(),
        "Should detect table format dev-dependency using workspace = true"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("b/Cargo.toml") && stderr.contains("`a`"),
        "Should mention c with b. Got: {}",
        stderr
    );
}
