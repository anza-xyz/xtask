use {
    crate::utils::bump::bump_targets,
    anyhow::{anyhow, Context, Result},
    clap::{Args, ValueEnum},
    log::{debug, info},
    semver::Version,
    std::{fs, process::Command},
    toml_edit::{value, DocumentMut},
};

#[derive(Args)]
pub struct CommandArgs {
    #[arg(value_enum)]
    pub level: BumpLevel,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum BumpLevel {
    #[value(help = "Bump major: x.y.z -> x+1.0.0-alpha.0")]
    Major,
    #[value(help = "Bump minor: x.y.z -> x.y+1.0-alpha.0")]
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

    let members =
        crate::utils::get_workspace_members().context("failed to resolve workspace members")?;

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

        let targets = bump_targets(&cargo_toml, &doc, &members, &current_version, &new_version);
        if targets.is_empty() {
            info!("  no version fields to bump");
            continue;
        }

        for (path, (old, new)) in &targets {
            set_version(&mut doc, path, new)
                .context(format!("failed to bump {path} in {}", cargo_toml.display()))?;
            info!("  bumped {path} from {old} to {new}");
        }

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

        info!("running `cargo tree` in {}", dir.display());
        let output = Command::new("cargo")
            .arg("tree")
            .current_dir(dir)
            .output()
            .context(format!("failed to run `cargo tree` in {}", dir.display()))?;
        if !output.status.success() {
            return Err(anyhow!("{}", String::from_utf8_lossy(&output.stderr)));
        }
    }

    Ok(())
}

/// `bump_targets` only reports paths it read out of this document, so a missing
/// one means the manifest changed between the two.
fn set_version(doc: &mut DocumentMut, path: &str, new: &str) -> Result<()> {
    let mut item = doc.as_item_mut();
    for segment in path.split('.') {
        item = item
            .get_mut(segment)
            .ok_or_else(|| anyhow!("no `{segment}` at `{path}`"))?;
    }
    *item = value(new);

    Ok(())
}

pub fn bump_version(level: &BumpLevel, current: &Version) -> Result<Version> {
    let mut new_version = current.clone();
    match level {
        BumpLevel::Major => {
            new_version.major = new_version.major.saturating_add(1);
            new_version.minor = 0;
            new_version.patch = 0;
            new_version.pre = semver::Prerelease::new("alpha.0").unwrap();
        }
        BumpLevel::Minor => {
            new_version.minor = new_version.minor.saturating_add(1);
            new_version.patch = 0;
            new_version.pre = semver::Prerelease::new("alpha.0").unwrap();
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
            Version::parse("2.0.0-alpha.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("1.1.0").unwrap()).unwrap(),
            Version::parse("2.0.0-alpha.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("1.1.1").unwrap()).unwrap(),
            Version::parse("2.0.0-alpha.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Major, &Version::parse("4.4.0-beta.3").unwrap()).unwrap(),
            Version::parse("5.0.0-alpha.0").unwrap()
        );
    }
    #[test]
    fn test_bump_version_minor() {
        assert_eq!(
            bump_version(&BumpLevel::Minor, &Version::parse("1.0.0").unwrap()).unwrap(),
            Version::parse("1.1.0-alpha.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Minor, &Version::parse("1.2.1").unwrap()).unwrap(),
            Version::parse("1.3.0-alpha.0").unwrap()
        );

        assert_eq!(
            bump_version(&BumpLevel::Minor, &Version::parse("4.3.0-alpha.3").unwrap()).unwrap(),
            Version::parse("4.4.0-alpha.0").unwrap()
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
}
