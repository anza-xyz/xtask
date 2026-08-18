use {
    anyhow::{anyhow, Result},
    log::debug,
    std::{
        path::{Path, PathBuf},
        process::Command,
    },
};

pub fn get_git_root_path() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| anyhow!("failed to get git root path, error: {e}"))?;
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(root))
}

pub fn ensure_rev_exists(rev: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", rev])
        .output()
        .map_err(|e| anyhow!("failed to resolve `{rev}`, error: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "cannot resolve `{rev}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

pub fn show_file_at_rev(rev: &str, path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .output()
        .map_err(|e| anyhow!("failed to read {path} at {rev}, error: {e}"))?;
    if !output.status.success() {
        debug!(
            "{path} not found at {rev}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Ok(None);
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

pub fn changed_files_since(rev: &str) -> Result<Vec<PathBuf>> {
    // Without `-z` git C-quotes special and non-ASCII paths.
    let output = Command::new("git")
        .args(["diff", "--name-only", "-z", rev])
        .output()
        .map_err(|e| anyhow!("failed to diff against {rev}, error: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "failed to diff against {rev}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

pub fn repo_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| anyhow!("{} is outside {}", path.display(), root.display()))?;

    Ok(relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
mod tests {
    use {super::*, pretty_assertions::assert_eq, scopeguard::defer, serial_test::serial, std::fs};

    #[test]
    #[serial]
    fn test_get_git_root_path() {
        let temp_dir = tempfile::tempdir().unwrap();

        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(temp_dir.path()).unwrap();
        Command::new("git").args(["init"]).output().unwrap();

        let root_path = get_git_root_path().unwrap();

        let canonicalized_root_path = fs::canonicalize(root_path).unwrap();
        let canonicalized_temp_dir_path = fs::canonicalize(temp_dir.path()).unwrap();

        assert_eq!(canonicalized_root_path, canonicalized_temp_dir_path);
    }

    /// Repository with a single commit adding `kept.txt`, `dir/nested.txt` and a
    /// path git would C-quote.
    fn init_repo(dir: &Path) {
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };

        git(&["init"]);
        fs::write(dir.join("kept.txt"), "kept\n").unwrap();
        fs::create_dir(dir.join("dir")).unwrap();
        fs::write(dir.join("dir/nested.txt"), "nested\n").unwrap();
        fs::write(dir.join("wéird name.txt"), "quoted\n").unwrap();
        git(&["add", "--", "kept.txt", "dir", "wéird name.txt"]);
        git(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "commit",
            "-m",
            "base",
        ]);
    }

    #[test]
    #[serial]
    fn test_ensure_rev_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        init_repo(temp_dir.path());

        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(temp_dir.path()).unwrap();

        ensure_rev_exists("HEAD").unwrap();

        let err = ensure_rev_exists("no-such-rev").unwrap_err().to_string();
        assert!(err.contains("cannot resolve `no-such-rev`"), "{err}");
    }

    #[test]
    #[serial]
    fn test_show_file_at_rev() {
        let temp_dir = tempfile::tempdir().unwrap();
        init_repo(temp_dir.path());

        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Changes after the commit must not leak into what the revision holds.
        fs::write(temp_dir.path().join("kept.txt"), "changed\n").unwrap();

        assert_eq!(
            show_file_at_rev("HEAD", "kept.txt").unwrap(),
            Some("kept\n".to_string())
        );
        assert_eq!(show_file_at_rev("HEAD", "absent.txt").unwrap(), None);
    }

    #[test]
    #[serial]
    fn test_changed_files_since() {
        let temp_dir = tempfile::tempdir().unwrap();
        init_repo(temp_dir.path());

        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(temp_dir.path()).unwrap();

        assert!(changed_files_since("HEAD").unwrap().is_empty());

        fs::write(temp_dir.path().join("dir/nested.txt"), "edited\n").unwrap();
        fs::remove_file(temp_dir.path().join("kept.txt")).unwrap();
        fs::write(temp_dir.path().join("wéird name.txt"), "edited\n").unwrap();
        // Untracked files are not part of the diff being verified.
        fs::write(temp_dir.path().join("untracked.txt"), "new\n").unwrap();

        assert_eq!(
            changed_files_since("HEAD").unwrap(),
            vec![
                PathBuf::from("dir/nested.txt"),
                PathBuf::from("kept.txt"),
                PathBuf::from("wéird name.txt"),
            ]
        );

        let err = changed_files_since("no-such-rev").unwrap_err().to_string();
        assert!(err.contains("failed to diff against no-such-rev"), "{err}");
    }

    #[test]
    fn test_repo_relative_path() {
        let root = Path::new("/repo");

        assert_eq!(
            repo_relative_path(root, Path::new("/repo/dir/Cargo.toml")).unwrap(),
            "dir/Cargo.toml"
        );
        assert_eq!(
            repo_relative_path(root, Path::new("/repo/Cargo.toml")).unwrap(),
            "Cargo.toml"
        );

        let err = repo_relative_path(root, Path::new("/elsewhere/Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is outside /repo"), "{err}");
    }
}
