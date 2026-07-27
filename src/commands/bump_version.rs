use {
    anyhow::{anyhow, Context, Result},
    clap::{Args, ValueEnum},
    log::{debug, info},
    semver::Version,
    std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
        process::Command,
    },
    toml_edit::{value, DocumentMut, Item, Table, Value},
};

#[derive(Args)]
pub struct CommandArgs {
    #[arg(value_enum)]
    pub level: BumpLevel,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum BumpLevel {
    #[value(help = "Bump major: x.y.z -> x+1.0.0")]
    Major,
    #[value(help = "Bump minor: x.y.z -> x.y+1.0")]
    Minor,
    #[value(help = "Bump patch: x.y.z -> x.y.z+1")]
    Patch,
    #[value(
        help = "Bump prerelease suffix: x.y.z-<tag>.n -> x.y.z-<tag>.n+1 (e.g. alpha/beta/rc)"
    )]
    PreRelease,
    #[value(
        help = "Promote prerelease stage: alpha.n -> beta.0, beta.n -> rc.0, rc.n -> '' (removed rc prerelease)"
    )]
    PromotePreRelease,
    #[value(
        help = "Bump prerelease if present; otherwise bump patch (x.y.z-<tag>.n -> x.y.z-<tag>.n+1, x.y.z -> x.y.z+1)"
    )]
    PatchOrPreRelease,
}

pub fn run(args: CommandArgs) -> Result<()> {
    let current_version_str =
        crate::utils::get_current_version().context("failed to get current version")?;
    let current_version = Version::parse(&current_version_str)?;

    let new_version = bump_version(&args.level, &current_version)?;

    let all_crates = crate::utils::get_all_crates().context("failed to get all crates")?;

    let all_cargo_tomls =
        crate::utils::find_all_cargo_tomls().context("failed to find all cargo.toml files")?;
    info!("found {} cargo.toml files", all_cargo_tomls.len());
    for cargo_toml in all_cargo_tomls {
        info!("processing {}", cargo_toml.display());

        let content = fs::read_to_string(&cargo_toml)
            .context(format!("failed to read {}", cargo_toml.display()))?;
        let mut doc = content
            .parse::<DocumentMut>()
            .context(format!("failed to parse {}", cargo_toml.display()))?;

        let original = doc.clone();
        let mut intended: BTreeMap<String, (String, String)> = BTreeMap::new();

        if let Some(workspace_package_version_str) = doc
            .get("workspace")
            .and_then(|workspace| workspace.get("package"))
            .and_then(|package| package.get("version"))
            .and_then(|version| version.as_str())
        {
            if workspace_package_version_str == current_version.to_string() {
                doc["workspace"]["package"]["version"] = value(new_version.to_string());
                intended.insert(
                    "workspace.package.version".to_string(),
                    (current_version.to_string(), new_version.to_string()),
                );
                info!("  bumped workspace.package.version from {current_version} to {new_version}",);
            }
        }

        if let Some(package_version_str) = doc
            .get("package")
            .and_then(|package| package.get("version"))
            .and_then(|version| version.as_str())
        {
            if package_version_str == current_version.to_string() {
                doc["package"]["version"] = value(new_version.to_string());
                intended.insert(
                    "package.version".to_string(),
                    (current_version.to_string(), new_version.to_string()),
                );
                info!("  bumped package.version from {current_version} to {new_version}",);
            }
        }

        if let Some(dependencies) = doc
            .get("workspace")
            .and_then(|ws| ws.get("dependencies"))
            .and_then(|deps| deps.as_table())
        {
            // Avoid borrowing `doc` while iterating
            let keys: Vec<String> = dependencies.iter().map(|(k, _)| k.to_string()).collect();

            for name in keys {
                if all_crates.contains(&name) {
                    if let Some(version) = doc["workspace"]["dependencies"]
                        .get(&name)
                        .and_then(|v| v.get("version"))
                        .and_then(|v| v.as_str())
                    {
                        if !version.contains(&current_version.to_string()) {
                            continue;
                        }
                        let old_version = version.to_string();
                        let bumped_version = old_version
                            .replace(&current_version.to_string(), &new_version.to_string());
                        doc["workspace"]["dependencies"][&name]["version"] = value(&bumped_version);
                        intended.insert(
                            format!("workspace.dependencies.{name}.version"),
                            (old_version.clone(), bumped_version.clone()),
                        );
                        info!(
                            "  bumped workspace.dependencies.{name}.version from {old_version} to \
                             {bumped_version}",
                        );
                    }
                }
            }
        }

        verify_changes(&original, &doc, &intended, &cargo_toml).context(format!(
            "unexpected changes while bumping {}",
            cargo_toml.display()
        ))?;

        // write the updated document back to the file
        debug!("writing {}", cargo_toml.display());
        fs::write(&cargo_toml, doc.to_string())
            .context(format!("failed to write {}", cargo_toml.display()))?;
    }

    let all_cargo_locks =
        crate::utils::find_all_cargo_locks().context("failed to find all Cargo.lock files")?;
    info!("found {} Cargo.lock files", all_cargo_locks.len());
    for cargo_lock in all_cargo_locks {
        let dir = cargo_lock.parent().context(format!(
            "failed to get {}'s parent directory",
            cargo_lock.display()
        ))?;

        let before = fs::read_to_string(&cargo_lock)
            .context(format!("failed to read {}", cargo_lock.display()))?;

        info!("running `cargo tree` in {}", dir.display());
        let output = Command::new("cargo")
            .arg("tree")
            .current_dir(dir)
            .output()
            .context(format!("failed to run `cargo tree` in {}", dir.display()))?;
        if !output.status.success() {
            return Err(anyhow!("{}", String::from_utf8_lossy(&output.stderr)));
        }

        let after = fs::read_to_string(&cargo_lock)
            .context(format!("failed to read {}", cargo_lock.display()))?;

        verify_lock_changes(
            &before,
            &after,
            &all_crates,
            &current_version,
            &new_version,
            &cargo_lock,
        )
        .context(format!(
            "unexpected changes while bumping {}",
            cargo_lock.display()
        ))?;
    }

    Ok(())
}

fn verify_changes(
    original: &DocumentMut,
    modified: &DocumentMut,
    intended: &BTreeMap<String, (String, String)>,
    file: &Path,
) -> Result<()> {
    let before = flatten_leaves(original);
    let after = flatten_leaves(modified);

    let mut actual: BTreeMap<String, (String, String)> = BTreeMap::new();
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for path in paths {
        let old = before.get(path);
        let new = after.get(path);
        if old != new {
            actual.insert(
                path.clone(),
                (
                    old.cloned().unwrap_or_default(),
                    new.cloned().unwrap_or_default(),
                ),
            );
        }
    }

    if &actual == intended {
        return Ok(());
    }

    let mut errors = vec![];
    for (path, (old, new)) in &actual {
        match intended.get(path) {
            None => errors.push(format!(
                "  unexpected change at `{path}`: {old:?} -> {new:?}"
            )),
            Some(expected) if expected != &(old.clone(), new.clone()) => errors.push(format!(
                "  wrong change at `{path}`: expected {expected:?}, got {:?}",
                (old, new)
            )),
            _ => {}
        }
    }
    for path in intended.keys() {
        if !actual.contains_key(path) {
            errors.push(format!("  expected change at `{path}` did not happen"));
        }
    }

    Err(anyhow!(
        "version bump touched unexpected content in {}:\n{}",
        file.display(),
        errors.join("\n")
    ))
}

fn verify_lock_changes(
    before: &str,
    after: &str,
    all_crates: &[String],
    current: &Version,
    new: &Version,
    file: &Path,
) -> Result<()> {
    let before_pkgs = parse_lock_packages(before)
        .context(format!("failed to parse {} before bump", file.display()))?;
    let after_pkgs = parse_lock_packages(after)
        .context(format!("failed to parse {} after bump", file.display()))?;

    let crates: BTreeSet<&str> = all_crates.iter().map(String::as_str).collect();
    let current = current.to_string();
    let new = new.to_string();

    // The only allowed change: workspace crates locked at the old version move to
    // the new version. Anything else (e.g. a transitive dep whose version jumps
    // because of a flexible range) is an unexpected change.
    let mut expected_removed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut expected_added: BTreeSet<(String, String)> = BTreeSet::new();
    for (name, version) in &before_pkgs {
        if crates.contains(name.as_str()) && version == &current {
            expected_removed.insert((name.clone(), current.clone()));
            expected_added.insert((name.clone(), new.clone()));
        }
    }

    let actual_removed: BTreeSet<(String, String)> =
        before_pkgs.difference(&after_pkgs).cloned().collect();
    let actual_added: BTreeSet<(String, String)> =
        after_pkgs.difference(&before_pkgs).cloned().collect();

    if actual_removed == expected_removed && actual_added == expected_added {
        return Ok(());
    }

    let mut errors = vec![];
    for (name, version) in actual_added.difference(&expected_added) {
        errors.push(format!("  unexpected package `{name} {version}`"));
    }
    for (name, version) in actual_removed.difference(&expected_removed) {
        errors.push(format!("  unexpectedly removed package `{name} {version}`"));
    }
    for (name, version) in expected_added.difference(&actual_added) {
        errors.push(format!(
            "  expected `{name}` to be bumped to `{version}` but it was not"
        ));
    }

    Err(anyhow!(
        "version bump touched unexpected content in {}:\n{}",
        file.display(),
        errors.join("\n")
    ))
}

fn parse_lock_packages(content: &str) -> Result<BTreeSet<(String, String)>> {
    let doc = content.parse::<DocumentMut>()?;

    let mut packages = BTreeSet::new();
    if let Some(Item::ArrayOfTables(entries)) = doc.get("package") {
        for entry in entries.iter() {
            let name = entry.get("name").and_then(Item::as_str);
            let version = entry.get("version").and_then(Item::as_str);
            if let (Some(name), Some(version)) = (name, version) {
                packages.insert((name.to_string(), version.to_string()));
            }
        }
    }

    Ok(packages)
}

fn flatten_leaves(doc: &DocumentMut) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    walk_table(doc.as_table(), String::new(), &mut out);
    out
}

fn walk_table(table: &Table, prefix: String, out: &mut BTreeMap<String, String>) {
    for (key, item) in table.iter() {
        walk_item(item, join(&prefix, key), out);
    }
}

fn walk_item(item: &Item, path: String, out: &mut BTreeMap<String, String>) {
    match item {
        Item::Value(v) => walk_value(v, path, out),
        Item::Table(t) => walk_table(t, path, out),
        Item::ArrayOfTables(arr) => {
            for (i, t) in arr.iter().enumerate() {
                walk_table(t, format!("{path}[{i}]"), out);
            }
        }
        Item::None => {}
    }
}

fn walk_value(v: &Value, path: String, out: &mut BTreeMap<String, String>) {
    match v {
        Value::InlineTable(t) => {
            for (key, val) in t.iter() {
                walk_value(val, join(&path, key), out);
            }
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                walk_value(val, format!("{path}[{i}]"), out);
            }
        }
        scalar => {
            let repr = scalar
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| scalar.to_string().trim().to_string());
            out.insert(path, repr);
        }
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

pub fn bump_version(level: &BumpLevel, current: &Version) -> Result<Version> {
    let mut new_version = current.clone();
    match level {
        BumpLevel::Major => {
            new_version.major = new_version.major.saturating_add(1);
            new_version.minor = 0;
            new_version.patch = 0;
        }
        BumpLevel::Minor => {
            new_version.minor = new_version.minor.saturating_add(1);
            new_version.patch = 0;
        }
        BumpLevel::Patch => {
            new_version.patch = new_version.patch.saturating_add(1);
        }
        BumpLevel::PreRelease => {
            if let Some((prefix, number_str)) = current.pre.as_str().split_once('.') {
                if let Ok(number) = number_str.parse::<u64>() {
                    let next = number.saturating_add(1);
                    if let Ok(next_pre) = semver::Prerelease::new(&format!("{prefix}.{next}")) {
                        new_version.pre = next_pre;
                    }
                } else {
                    return Err(anyhow!("unexpected prerelease format: {}", current.pre));
                }
            } else {
                return Err(anyhow!("unexpected prerelease format: {}", current.pre));
            }
        }
        BumpLevel::PromotePreRelease => {
            if let Some((prefix, _)) = current.pre.as_str().split_once('.') {
                match prefix {
                    "alpha" => {
                        new_version.pre = semver::Prerelease::new("beta.0").unwrap();
                    }
                    "beta" => {
                        new_version.pre = semver::Prerelease::new("rc.0").unwrap();
                    }
                    "rc" => {
                        new_version.pre = semver::Prerelease::new("").unwrap();
                    }
                    _ => {
                        return Err(anyhow!("unexpected prerelease format: {}, only alpha, beta, and rc are supported", current.pre));
                    }
                }
            } else {
                return Err(anyhow!("unexpected prerelease format: {}", current.pre));
            }
        }
        BumpLevel::PatchOrPreRelease => {
            if current.pre.is_empty() {
                new_version = bump_version(&BumpLevel::Patch, current)?;
            } else {
                new_version = bump_version(&BumpLevel::PreRelease, current)?;
            }
        }
    }

    Ok(new_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bump_version_major() {
        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("1.0.0").unwrap()).unwrap(),
            Version::parse("2.0.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("1.1.0").unwrap()).unwrap(),
            Version::parse("2.0.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("1.1.1").unwrap()).unwrap(),
            Version::parse("2.0.0").unwrap()
        );
    }
    #[test]
    fn test_bump_version_minor() {
        assert_eq!(
            bump_version(&BumpLevel::Minor, &Version::parse("1.0.0").unwrap()).unwrap(),
            Version::parse("1.1.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Minor, &Version::parse("1.2.1").unwrap()).unwrap(),
            Version::parse("1.3.0").unwrap()
        );
    }

    #[test]
    fn test_bump_version_patch() {
        assert_eq!(
            bump_version(&BumpLevel::Patch, &Version::parse("1.0.0").unwrap()).unwrap(),
            Version::parse("1.0.1").unwrap()
        );
    }

    #[test]
    fn test_bump_version_prerelease() {
        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-alpha.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-alpha.1").unwrap()
        );
        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-alpha.1").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-alpha.2").unwrap()
        );
        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-beta.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-beta.1").unwrap()
        );
        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-rc.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-rc.1").unwrap()
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-alpha123").unwrap()
            )
            .unwrap_err()
            .to_string(),
            "unexpected prerelease format: alpha123",
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PreRelease,
                &Version::parse("1.2.3-alpha.custom").unwrap()
            )
            .unwrap_err()
            .to_string(),
            "unexpected prerelease format: alpha.custom",
        );
    }

    #[test]
    fn test_bump_version_promote_prerelease() {
        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-alpha.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-beta.0").unwrap()
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-alpha.1").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-beta.0").unwrap()
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-beta.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-rc.0").unwrap()
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-rc.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3").unwrap()
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-alpha123").unwrap()
            )
            .unwrap_err()
            .to_string(),
            "unexpected prerelease format: alpha123",
        );

        assert_eq!(
            bump_version(
                &BumpLevel::PromotePreRelease,
                &Version::parse("1.2.3-custom.1").unwrap()
            )
            .unwrap_err()
            .to_string(),
            "unexpected prerelease format: custom.1, only alpha, beta, and rc are supported"
        );
    }

    #[test]
    fn test_bump_version_patch_or_prerelease() {
        assert_eq!(
            bump_version(
                &BumpLevel::PatchOrPreRelease,
                &Version::parse("1.2.3-alpha.0").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.3-alpha.1").unwrap()
        );
        assert_eq!(
            bump_version(
                &BumpLevel::PatchOrPreRelease,
                &Version::parse("1.2.3").unwrap()
            )
            .unwrap(),
            Version::parse("1.2.4").unwrap()
        );
    }

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    fn intent(pairs: &[(&str, &str, &str)]) -> BTreeMap<String, (String, String)> {
        pairs
            .iter()
            .map(|(p, o, n)| (p.to_string(), (o.to_string(), n.to_string())))
            .collect()
    }

    #[test]
    fn test_verify_changes_ok() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nname = \"foo\"\nversion = \"1.1.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        assert!(verify_changes(&original, &modified, &intended, Path::new("Cargo.toml")).is_ok());
    }

    #[test]
    fn test_verify_changes_ignores_formatting() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\n# comment\nname   =   \"foo\"\nversion = \"1.0.0\"\n");
        assert!(verify_changes(
            &original,
            &modified,
            &BTreeMap::new(),
            Path::new("Cargo.toml")
        )
        .is_ok());
    }

    #[test]
    fn test_verify_changes_detects_stray_edit() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nname = \"bar\"\nversion = \"1.1.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unexpected change at `package.name`"), "{err}");
    }

    #[test]
    fn test_verify_changes_detects_skipped_bump() {
        let original = doc("[package]\nversion = \"1.0.0\"\n");
        let modified = original.clone();
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("expected change at `package.version` did not happen"),
            "{err}"
        );
    }

    #[test]
    fn test_verify_changes_detects_wrong_value() {
        let original = doc("[package]\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nversion = \"2.0.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("wrong change at `package.version`"), "{err}");
    }

    fn lock(packages: &[(&str, &str)]) -> String {
        let mut out = String::from("version = 3\n");
        for (name, version) in packages {
            out.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
            ));
        }
        out
    }

    fn crates(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_verify_lock_changes_ok() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.150")]);
        assert!(verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }

    #[test]
    fn test_verify_lock_changes_detects_transitive_jump() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.200")]);
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unexpected package `serde 1.0.200`"), "{err}");
        assert!(
            err.contains("unexpectedly removed package `serde 1.0.150`"),
            "{err}"
        );
    }

    #[test]
    fn test_verify_lock_changes_detects_new_package() {
        let before = lock(&[("foo", "1.0.0")]);
        let after = lock(&[("foo", "1.1.0"), ("newdep", "0.1.0")]);
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unexpected package `newdep 0.1.0`"), "{err}");
    }

    #[test]
    fn test_verify_lock_changes_detects_skipped_bump() {
        let before = lock(&[("foo", "1.0.0")]);
        let after = before.clone();
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("expected `foo` to be bumped to `1.1.0` but it was not"),
            "{err}"
        );
    }

    #[test]
    fn test_verify_lock_changes_ignores_non_workspace_crate() {
        // A crate that shares the old version string but isn't a workspace member
        // must not be touched, and its presence must not be required to change.
        let before = lock(&[("foo", "1.0.0"), ("other", "1.0.0")]);
        let after = lock(&[("foo", "1.1.0"), ("other", "1.0.0")]);
        assert!(verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }
}
