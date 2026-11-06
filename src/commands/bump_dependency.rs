use {
    crate::{commands::update_crate, utils},
    anyhow::{Context, Result},
    cargo_lock::{Dependency, Lockfile, Name},
    log::{info, warn},
    std::{collections::HashMap, fs},
};

pub use update_crate::CommandArgs;

pub fn run(update: CommandArgs) -> Result<()> {
    let snapshots = utils::find_files_by_name(&update.root_path, "Cargo.lock")?
        .into_iter()
        .filter(|lock_file| !utils::is_excluded(lock_file, &update.exclude_paths))
        .map(|lock_file| {
            let snapshot = Lockfile::load(&lock_file)
                .context(format!("failed to parse {}", lock_file.display()))?;
            Ok((lock_file, snapshot))
        })
        .collect::<Result<Vec<_>>>()?;
    let (package, from, to) = (update.package.clone(), &update.from, &update.to);

    // Updating the locks while the manifests still request the old version
    // swaps just this one package. Semver-major bumps fail here, and the
    // `cargo tree` after the manifest edit resolves them instead.
    let spec = format!("{package}:{from}");
    let update_lock = ["update", "-p", &spec, "--precise", to];
    for (lock_file, _) in &snapshots {
        if let Err(err) = utils::run_in_lock_dir(lock_file, &update_lock) {
            warn!("[{}] {err:#}", lock_file.display());
        }
    }
    update_crate::run(update)?;
    for (lock_file, _) in &snapshots {
        utils::run_in_lock_dir(lock_file, &["tree"])?;
    }

    let verify = ["metadata", "--locked", "--format-version", "1"];
    for (lock_file, snapshot) in &snapshots {
        let path = lock_file.display();
        let mut lockfile = Lockfile::load(lock_file).context(format!("failed to parse {path}"))?;
        let restored = restore_edges(snapshot, &mut lockfile, &package);
        if !restored.is_empty() {
            info!("[{path}] restored {} edge(s):", restored.len());
            for edge in &restored {
                info!("  {edge}");
            }
            fs::write(lock_file, lockfile.to_string())
                .context(format!("failed to write {path}"))?;
        }

        // Normalizes the formatting and drops packages the restored edges no
        // longer reach.
        utils::run_in_lock_dir(lock_file, &["tree"])?;
        utils::run_in_lock_dir(lock_file, &verify)?;
    }

    Ok(())
}

/// Puts back the snapshot version of each dependency edge that an unchanged
/// package re-resolved, as long as the lock still holds that version.
fn restore_edges(snapshot: &Lockfile, lockfile: &mut Lockfile, package: &str) -> Vec<String> {
    let locked: Vec<_> = lockfile.packages.iter().map(Dependency::from).collect();
    let snapshot_deps: HashMap<_, _> = snapshot
        .packages
        .iter()
        .map(|p| (Dependency::from(p), &p.dependencies))
        .collect();
    let mut restored = vec![];

    for current in &mut lockfile.packages {
        let Some(old_deps) = snapshot_deps.get(&Dependency::from(&*current)) else {
            continue;
        };
        let label = format!("{} {}", current.name, current.version);

        for index in 0..current.dependencies.len() {
            let deps = &current.dependencies;
            let new_dep = &deps[index];
            let name = &new_dep.name;
            if name.as_str() == package || single_dependency(deps, name).is_none() {
                continue;
            }
            let Some(old_dep) = single_dependency(old_deps, name) else {
                continue;
            };
            if old_dep == new_dep || !locked.contains(old_dep) {
                continue;
            }
            let (new, old) = (&new_dep.version, &old_dep.version);
            restored.push(format!("{label}: {name} {new} -> {old}"));
            current.dependencies[index] = old_dep.clone();
        }
    }

    restored
}

/// Returns the dependency called `name` if `deps` lists it exactly once. A
/// package can depend on one crate several times under different names, and
/// those edges can't be matched up between the two locks.
fn single_dependency<'a>(deps: &'a [Dependency], name: &Name) -> Option<&'a Dependency> {
    let mut matching = deps.iter().filter(|dep| &dep.name == name);
    let dep = matching.next()?;
    matching.next().is_none().then_some(dep)
}

#[cfg(test)]
mod tests {
    use {super::*, pretty_assertions::assert_eq};

    fn package(name: &str, version: &str, deps: &[&str]) -> String {
        let source = "registry+https://github.com/rust-lang/crates.io-index";
        let deps: String = deps.iter().map(|dep| format!(" \"{dep}\",\n")).collect();
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"{source}\"\ndependencies = [\n{deps}]\n"
        )
    }

    fn lock(packages: &[String]) -> Lockfile {
        format!("version = 4\n\n{}", packages.join("\n"))
            .parse()
            .unwrap()
    }

    #[test]
    fn test_restores_flipped_edges() {
        let snapshot = lock(&[
            package("bumped", "1.0.0", &[]),
            package("rustix", "1.1.4", &["windows-sys 0.61.0"]),
            package("socket2", "0.6.0", &["rand_core"]),
            package("rand_core", "0.6.4", &[]),
            package("windows-sys", "0.52.0", &[]),
            package("windows-sys", "0.61.0", &[]),
        ]);
        let mut lockfile = lock(&[
            package(
                "bumped",
                "1.0.1",
                &["rand_core 0.5.1", "windows-sys 0.52.0"],
            ),
            package("rustix", "1.1.4", &["windows-sys 0.52.0"]),
            package("socket2", "0.6.0", &["rand_core 0.5.1"]),
            package("rand_core", "0.5.1", &[]),
            package("rand_core", "0.6.4", &[]),
            package("windows-sys", "0.52.0", &[]),
            package("windows-sys", "0.61.0", &[]),
        ]);

        let restored = restore_edges(&snapshot, &mut lockfile, "bumped");

        assert_eq!(
            restored,
            [
                "rustix 1.1.4: windows-sys 0.52.0 -> 0.61.0",
                "socket2 0.6.0: rand_core 0.5.1 -> 0.6.4",
            ]
        );
        assert_eq!(
            lockfile,
            lock(&[
                package(
                    "bumped",
                    "1.0.1",
                    &["rand_core 0.5.1", "windows-sys 0.52.0"]
                ),
                package("rustix", "1.1.4", &["windows-sys 0.61.0"]),
                package("socket2", "0.6.0", &["rand_core 0.6.4"]),
                package("rand_core", "0.5.1", &[]),
                package("rand_core", "0.6.4", &[]),
                package("windows-sys", "0.52.0", &[]),
                package("windows-sys", "0.61.0", &[]),
            ])
        );
    }

    #[test]
    fn test_keeps_edges() {
        let snapshot = lock(&[
            // the snapshot target leaves the lock
            package("tempfile", "3.0.0", &["rand 0.8.5"]),
            // the bumped package
            package("user", "1.0.0", &["bumped 1.0.0"]),
            // a renamed dependency, listed twice in either lock
            package("renamed", "1.0.0", &["getrandom 0.1.0", "getrandom 0.2.0"]),
            package("feature-renamed", "1.0.0", &["getrandom 0.2.0"]),
            // the package version changes
            package("rustix", "1.1.3", &["windows-sys 0.61.0"]),
            package("bumped", "1.0.0", &[]),
            package("getrandom", "0.1.0", &[]),
            package("getrandom", "0.2.0", &[]),
            package("rand", "0.8.5", &[]),
            package("rand", "0.9.0", &[]),
            package("windows-sys", "0.52.0", &[]),
            package("windows-sys", "0.61.0", &[]),
        ]);
        let lockfile = lock(&[
            package("tempfile", "3.0.0", &["rand 0.8.6"]),
            package("user", "1.0.0", &["bumped 2.0.0"]),
            package("renamed", "1.0.0", &["getrandom 0.1.0", "getrandom 0.3.0"]),
            package(
                "feature-renamed",
                "1.0.0",
                &["getrandom 0.1.0", "getrandom 0.3.0"],
            ),
            package("rustix", "1.1.4", &["windows-sys 0.52.0"]),
            package("bumped", "1.0.0", &[]),
            package("bumped", "2.0.0", &[]),
            package("getrandom", "0.1.0", &[]),
            package("getrandom", "0.2.0", &[]),
            package("getrandom", "0.3.0", &[]),
            package("rand", "0.8.6", &[]),
            package("rand", "0.9.0", &[]),
            package("windows-sys", "0.52.0", &[]),
            package("windows-sys", "0.61.0", &[]),
        ]);

        let mut restored_lockfile = lockfile.clone();
        let restored = restore_edges(&snapshot, &mut restored_lockfile, "bumped");

        assert_eq!(restored, Vec::<String>::new());
        assert_eq!(restored_lockfile, lockfile);
    }
}
