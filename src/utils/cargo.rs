use {
    anyhow::{anyhow, Context, Result},
    cargo_metadata::{Metadata, MetadataCommand, PackageId},
    log::warn,
    std::{
        collections::{BTreeSet, HashSet},
        fs,
        path::{Path, PathBuf},
    },
    toml_edit::Document,
};

#[derive(Debug, Default)]
pub struct WorkspaceMembers {
    pub names: BTreeSet<String>,
    pub manifests: BTreeSet<PathBuf>,
    pub roots: BTreeSet<PathBuf>,
}

impl WorkspaceMembers {
    pub fn contains_name(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub fn contains_manifest(&self, manifest: &Path) -> bool {
        self.manifests.contains(&normalize(manifest))
    }

    pub fn is_root(&self, manifest: &Path) -> bool {
        self.roots.contains(&normalize(manifest))
    }
}

/// Probes only manifests declaring `[workspace]`: cargo reports an unclaimed
/// package as its own single-member workspace, which would make every package
/// in the repo a member.
pub fn get_workspace_members() -> Result<WorkspaceMembers> {
    let mut members = WorkspaceMembers::default();
    let mut seen_roots = HashSet::new();

    for manifest in super::fs::find_all_cargo_tomls()? {
        if !declares_workspace(&manifest)? {
            continue;
        }

        let metadata = match MetadataCommand::new()
            .no_deps()
            .manifest_path(&manifest)
            .exec()
        {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!("skipping {}: {err}", manifest.display());
                continue;
            }
        };

        if !seen_roots.insert(metadata.workspace_root.clone()) {
            continue;
        }

        let ids = member_ids(&metadata);
        for pkg in metadata.packages.iter().filter(|pkg| ids.contains(&pkg.id)) {
            members.names.insert(pkg.name.to_string());
            members
                .manifests
                .insert(normalize(pkg.manifest_path.as_std_path()));
        }
        members.roots.insert(normalize(&manifest));
    }

    Ok(members)
}

fn member_ids(metadata: &Metadata) -> HashSet<&PackageId> {
    metadata.workspace_members.iter().collect()
}

fn declares_workspace(manifest: &Path) -> Result<bool> {
    let content =
        fs::read_to_string(manifest).context(format!("failed to read {}", manifest.display()))?;
    let doc = content
        .parse::<Document<String>>()
        .context(format!("failed to parse {}", manifest.display()))?;

    Ok(doc.get("workspace").is_some())
}

fn normalize(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn get_all_crates() -> Result<Vec<String>> {
    let cargo_tomls = super::fs::find_all_cargo_tomls()?;
    let mut crates = vec![];
    for cargo_toml in cargo_tomls {
        let content = fs::read_to_string(cargo_toml)?;
        let doc = content.parse::<Document<String>>()?;
        let Some(name) = doc
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(|name| name.as_str())
        else {
            continue;
        };
        crates.push(name.to_string());
    }
    Ok(crates)
}

pub fn get_current_version() -> Result<String> {
    let git_root = super::git::get_git_root_path()?;
    let cargo_toml = git_root.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml)
        .context(format!("failed to read {}", cargo_toml.display()))?;

    manifest_version(&content).context(format!(
        "failed to get version from {}",
        cargo_toml.display()
    ))
}

/// Falls back to `package.version` so single-crate repositories work too, not
/// just workspaces that share an inherited version.
pub fn manifest_version(content: &str) -> Result<String> {
    let doc = content.parse::<Document<String>>()?;
    let version = doc
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .or_else(|| {
            doc.get("package")
                .and_then(|package| package.get("version"))
        })
        .and_then(|version| version.as_str())
        .ok_or_else(|| anyhow!("no workspace.package.version or package.version"))?;

    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use {
        super::*, pretty_assertions::assert_eq, scopeguard::defer, serial_test::serial,
        std::collections::HashSet,
    };

    #[test]
    #[serial]
    fn test_cargo_functions() {
        let root_dir = tempfile::tempdir().unwrap();
        let root_dir_path = root_dir.path();
        // Restore before the tempdir is dropped: a process whose working
        // directory no longer exists cannot run cargo at all.
        let original_dir = std::env::current_dir().unwrap();
        defer! { std::env::set_current_dir(&original_dir).unwrap(); }
        std::env::set_current_dir(root_dir_path).unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .output()
            .unwrap();

        std::fs::write(
            root_dir_path.join("Cargo.toml"),
            "[workspace.package]\nversion = \"3.1.0\"\n\n[members]\nfoo = { path = \"foo\" }\nbar = { path = \"bar\" }",
        )
        .unwrap();

        std::fs::create_dir_all(root_dir_path.join("foo")).unwrap();
        std::fs::write(
            root_dir_path.join("foo/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = { workspace = true }",
        )
        .unwrap();

        std::fs::create_dir_all(root_dir_path.join("bar")).unwrap();
        std::fs::write(
            root_dir_path.join("bar/Cargo.toml"),
            "[package]\nname = \"bar\"\nversion = { workspace = true }",
        )
        .unwrap();

        {
            let crates = get_all_crates().unwrap();
            assert_eq!(crates.len(), 2);
            let expected_crates: HashSet<String> =
                ["foo", "bar"].iter().map(|s| s.to_string()).collect();
            let actual_crates: HashSet<String> = crates.iter().map(|s| s.to_string()).collect();
            assert_eq!(expected_crates, actual_crates);
        }

        {
            let version = get_current_version().unwrap();
            assert_eq!(version, "3.1.0");
        }
    }

    #[test]
    #[serial]
    fn test_get_current_version_single_crate() {
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
            "[package]\nname = \"solo\"\nversion = \"0.2.2\"\n",
        )
        .unwrap();

        assert_eq!(get_current_version().unwrap(), "0.2.2");
    }

    #[test]
    #[serial]
    fn test_get_workspace_members() {
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
            "[workspace]\nmembers = [\"foo\"]\nexclude = [\"stray\"]\n\n[workspace.package]\nversion = \"3.1.0\"\n",
        )
        .unwrap();

        std::fs::create_dir_all(root_dir_path.join("foo/src")).unwrap();
        std::fs::write(
            root_dir_path.join("foo/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = { workspace = true }\n",
        )
        .unwrap();
        std::fs::write(root_dir_path.join("foo/src/lib.rs"), "").unwrap();

        std::fs::create_dir_all(root_dir_path.join("stray/src")).unwrap();
        std::fs::write(
            root_dir_path.join("stray/Cargo.toml"),
            "[package]\nname = \"stray\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(root_dir_path.join("stray/src/lib.rs"), "").unwrap();

        let members = get_workspace_members().unwrap();

        // `stray` is a package no workspace claims, so probing its manifest
        // would report it as its own single-member workspace.
        assert_eq!(members.names, ["foo".to_string()].into_iter().collect());
        assert!(members.contains_manifest(&root_dir_path.join("foo/Cargo.toml")));
        assert!(!members.contains_manifest(&root_dir_path.join("stray/Cargo.toml")));
        assert!(members.is_root(&root_dir_path.join("Cargo.toml")));
        assert!(!members.is_root(&root_dir_path.join("stray/Cargo.toml")));
    }

    #[test]
    fn test_manifest_version() {
        assert_eq!(
            manifest_version("[workspace.package]\nversion = \"3.1.0\"\n").unwrap(),
            "3.1.0"
        );

        // A single-crate repository has no workspace section at all.
        assert_eq!(
            manifest_version("[package]\nname = \"foo\"\nversion = \"0.2.2\"\n").unwrap(),
            "0.2.2"
        );

        // The workspace version wins when both are present.
        assert_eq!(
            manifest_version(
                "[workspace.package]\nversion = \"3.1.0\"\n\n[package]\nname = \"foo\"\nversion = \"0.2.2\"\n"
            )
            .unwrap(),
            "3.1.0"
        );

        // An inherited version is not a version anyone can bump here.
        assert_eq!(
            manifest_version("[package]\nname = \"foo\"\nversion = { workspace = true }\n")
                .unwrap_err()
                .to_string(),
            "no workspace.package.version or package.version"
        );

        assert!(manifest_version("[package\n").is_err());
    }

    #[test]
    fn test_declares_workspace() {
        let dir = tempfile::tempdir().unwrap();

        let root = dir.path().join("root.toml");
        std::fs::write(&root, "[workspace]\nmembers = [\"foo\"]\n").unwrap();
        assert!(declares_workspace(&root).unwrap());

        let package = dir.path().join("package.toml");
        std::fs::write(&package, "[package]\nname = \"foo\"\n").unwrap();
        assert!(!declares_workspace(&package).unwrap());

        let malformed = dir.path().join("malformed.toml");
        std::fs::write(&malformed, "[package\n").unwrap();
        assert!(declares_workspace(&malformed).is_err());

        assert!(declares_workspace(&dir.path().join("missing.toml")).is_err());
    }
}
