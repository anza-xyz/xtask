use {
    anyhow::Result,
    ignore::WalkBuilder,
    std::path::{Path, PathBuf},
};

pub fn find_files_by_name(root: &Path, filename: &str) -> Result<Vec<PathBuf>> {
    let mut results = vec![];

    for entry in WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        // filter_entry prunes the walk, so a large target/ is never descended
        // into even when it is not gitignored.
        .filter_entry(|entry| {
            !entry
                .path()
                .components()
                .any(|c| c.as_os_str() == "target" || c.as_os_str() == ".git")
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == filename)
    {
        results.push(entry.path().to_path_buf());
    }

    Ok(results)
}

pub fn find_all_cargo_tomls() -> Result<Vec<PathBuf>> {
    find_files_by_name(&super::git::get_git_root_path()?, "Cargo.toml")
}

pub fn find_all_cargo_locks() -> Result<Vec<PathBuf>> {
    find_files_by_name(&super::git::get_git_root_path()?, "Cargo.lock")
}

#[cfg(test)]
mod tests {
    use {
        super::*, pretty_assertions::assert_eq, scopeguard::defer, serial_test::serial,
        std::collections::HashSet,
    };

    #[test]
    #[serial]
    fn test_find_cargo_files() {
        let root_dir = tempfile::tempdir().unwrap();
        let root_dir_path = root_dir.path();
        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(root_dir_path).unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .output()
            .unwrap();

        std::fs::write(
            root_dir_path.join("Cargo.toml"),
            "[workspace.package]\nversion = \"3.1.0\"",
        )
        .unwrap();
        std::fs::write(root_dir_path.join("Cargo.lock"), "").unwrap();
        std::fs::create_dir_all(root_dir_path.join("foo")).unwrap();
        std::fs::write(
            root_dir_path.join("foo/Cargo.toml"),
            "[package]\nname = \"foo\"",
        )
        .unwrap();
        std::fs::write(root_dir_path.join("foo/Cargo.lock"), "").unwrap();

        std::fs::create_dir_all(root_dir_path.join("bar")).unwrap();
        std::fs::write(
            root_dir_path.join("bar/Cargo.toml"),
            "[package]\nname = \"bar\"",
        )
        .unwrap();
        std::fs::write(root_dir_path.join("bar/Cargo.lock"), "").unwrap();

        {
            let files = find_all_cargo_tomls().unwrap();
            assert_eq!(files.len(), 3);

            let expected_files: HashSet<_> = [
                std::fs::canonicalize(root_dir_path.join("Cargo.toml")).unwrap(),
                std::fs::canonicalize(root_dir_path.join("foo/Cargo.toml")).unwrap(),
                std::fs::canonicalize(root_dir_path.join("bar/Cargo.toml")).unwrap(),
            ]
            .iter()
            .cloned()
            .collect();

            let actual_files: HashSet<_> = files.iter().cloned().collect();

            assert_eq!(expected_files, actual_files);
        }

        {
            let files = find_all_cargo_locks().unwrap();
            assert_eq!(files.len(), 3);

            let expected_files: HashSet<_> = [
                std::fs::canonicalize(root_dir_path.join("Cargo.lock")).unwrap(),
                std::fs::canonicalize(root_dir_path.join("foo/Cargo.lock")).unwrap(),
                std::fs::canonicalize(root_dir_path.join("bar/Cargo.lock")).unwrap(),
            ]
            .iter()
            .cloned()
            .collect();

            let actual_files: HashSet<_> = files.iter().cloned().collect();

            assert_eq!(expected_files, actual_files);
        }
    }

    #[test]
    #[serial]
    fn test_find_cargo_files_skips_ignored() {
        let root_dir = tempfile::tempdir().unwrap();
        let root_dir_path = root_dir.path();
        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(root_dir_path).unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .output()
            .unwrap();

        std::fs::write(root_dir_path.join(".gitignore"), "vendor/\n").unwrap();
        std::fs::write(
            root_dir_path.join("Cargo.toml"),
            "[workspace.package]\nversion = \"3.1.0\"",
        )
        .unwrap();
        std::fs::create_dir_all(root_dir_path.join("vendor")).unwrap();
        std::fs::write(
            root_dir_path.join("vendor/Cargo.toml"),
            "[package]\nname = \"vendored\"",
        )
        .unwrap();

        let files = find_all_cargo_tomls().unwrap();
        assert_eq!(
            files,
            vec![std::fs::canonicalize(root_dir_path.join("Cargo.toml")).unwrap()]
        );
    }
}
