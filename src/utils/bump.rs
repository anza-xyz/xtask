//! Which fields a version bump may touch, and how to check a bump touched only
//! those.

use {
    anyhow::{anyhow, Context, Result},
    semver::Version,
    std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
    },
    toml_edit::{DocumentMut, Item, Table, Value},
};

/// Bumps a dependency requirement only when it pins `current` itself, matching
/// the version after any comparison operator.
///
/// Substring matching rewrote `=12.0.1` on a bump of `2.0.1`, leaving a pin to a
/// version the crate never publishes.
pub fn bumped_requirement(requirement: &str, current: &str, new: &str) -> Option<String> {
    let version = requirement.trim_start_matches(['=', '^', '~', '>', '<', ' ']);
    if version != current {
        return None;
    }
    let operator = requirement.strip_suffix(version)?;

    Some(format!("{operator}{new}"))
}

pub fn verify_changes(
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

pub fn verify_lock_changes(
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

    // Rebuild the lock we expect: workspace crates move current -> new, every
    // other package (source, checksum, dependency edges) stays identical.
    let mut expected: BTreeMap<(String, String), String> = BTreeMap::new();
    for ((name, version), body) in &before_pkgs {
        let key = if crates.contains(name.as_str()) && version == &current {
            (name.clone(), new.clone())
        } else {
            (name.clone(), version.clone())
        };
        expected.insert(key, body.clone());
    }

    if expected == after_pkgs {
        return Ok(());
    }

    let mut errors = vec![];
    let keys: BTreeSet<&(String, String)> = expected.keys().chain(after_pkgs.keys()).collect();
    for key @ (name, version) in keys {
        match (expected.get(key), after_pkgs.get(key)) {
            (Some(want), Some(got)) if want != got => {
                errors.push(format!("  unexpected change to package `{name} {version}`"));
            }
            (Some(_), None) => {
                errors.push(format!("  missing package `{name} {version}` after bump"));
            }
            (None, Some(_)) => {
                errors.push(format!("  unexpected package `{name} {version}`"));
            }
            _ => {}
        }
    }

    Err(anyhow!(
        "version bump touched unexpected content in {}:\n{}",
        file.display(),
        errors.join("\n")
    ))
}

fn parse_lock_packages(content: &str) -> Result<BTreeMap<(String, String), String>> {
    let doc = content.parse::<DocumentMut>()?;

    let mut packages = BTreeMap::new();
    if let Some(Item::ArrayOfTables(entries)) = doc.get("package") {
        for entry in entries.iter() {
            let name = entry.get("name").and_then(Item::as_str);
            let version = entry.get("version").and_then(Item::as_str);
            let (Some(name), Some(version)) = (name, version) else {
                continue;
            };

            let mut fields = vec![];
            for (key, item) in entry.iter() {
                if key == "name" || key == "version" {
                    continue;
                }
                fields.push(format!("{key}={}", item.to_string().trim()));
            }
            fields.sort();

            packages.insert((name.to_string(), version.to_string()), fields.join("\n"));
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

#[cfg(test)]
mod tests {
    use super::*;

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
            err.contains("missing package `serde 1.0.150` after bump"),
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
            err.contains("missing package `foo 1.1.0` after bump"),
            "{err}"
        );
        assert!(err.contains("unexpected package `foo 1.0.0`"), "{err}");
    }

    #[test]
    fn test_verify_lock_changes_detects_moved_dependency_edge() {
        let before = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"tokio\"\nversion = \"1.52.3\"\ndependencies = [\"windows-sys 0.61.0\"]\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.45.0\"\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.61.0\"\n";
        let after = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.1.0\"\n\n[[package]]\nname = \"tokio\"\nversion = \"1.52.3\"\ndependencies = [\"windows-sys 0.45.0\"]\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.45.0\"\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.61.0\"\n";
        let err = verify_lock_changes(
            before,
            after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("unexpected change to package `tokio 1.52.3`"),
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

    #[test]
    fn test_bumped_requirement_pins_current() {
        assert_eq!(
            bumped_requirement("=1.2.3", "1.2.3", "1.2.4"),
            Some("=1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("1.2.3", "1.2.3", "1.2.4"),
            Some("1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("^1.2.3", "1.2.3", "1.2.4"),
            Some("^1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("~1.2.3", "1.2.3", "1.2.4"),
            Some("~1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement(">=1.2.3", "1.2.3", "1.2.4"),
            Some(">=1.2.4".to_string())
        );
    }

    #[test]
    fn test_bumped_requirement_leaves_other_versions_alone() {
        // "12.0.1" contains "2.0.1", so substring matching used to rewrite this
        // into a pin the crate never publishes.
        assert_eq!(bumped_requirement("=12.0.1", "2.0.1", "2.0.2"), None);
        assert_eq!(bumped_requirement("=1.2.30", "1.2.3", "1.2.4"), None);
        assert_eq!(bumped_requirement("=2.0.0", "1.2.3", "1.2.4"), None);
        // A range is not a pin.
        assert_eq!(bumped_requirement(">=1.2.3, <2", "1.2.3", "1.2.4"), None);
    }
}
