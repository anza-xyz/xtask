use {
    anyhow::{Context, Result},
    clap::Subcommand,
    log::warn,
    std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
    },
    toml_edit::DocumentMut,
};

#[derive(clap::Args, Debug)]
pub struct CommandArgs {
    #[command(subcommand)]
    command: DevDepsCommands,
}

#[derive(Subcommand, Debug)]
enum DevDepsCommands {
    #[command(about = "Check dev-dependencies for workspace members using 'workspace = true'")]
    Check(CheckArgs),
}

#[derive(clap::Args, Debug)]
pub struct CheckArgs {
    /// Path to the workspace Cargo.toml (defaults to ./Cargo.toml)
    #[arg(long, default_value = "Cargo.toml")]
    pub manifest_path: PathBuf,
}

pub fn run(args: CommandArgs) -> Result<()> {
    match args.command {
        DevDepsCommands::Check(check_args) => run_check(check_args),
    }
}

fn run_check(args: CheckArgs) -> Result<()> {
    let workspace_root = args
        .manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));

    // 1. Parse workspace Cargo.toml to get members
    let workspace_toml_content = fs::read_to_string(&args.manifest_path)
        .with_context(|| format!("Failed to read {:?}", args.manifest_path))?;
    let workspace_doc = workspace_toml_content
        .parse::<DocumentMut>()
        .with_context(|| format!("Failed to parse {:?}", args.manifest_path))?;

    let members = workspace_doc
        .get("workspace")
        .and_then(|ws| ws.get("members"))
        .and_then(|m| m.as_array())
        .context("No workspace.members found")?;

    // 2. Build map of package names -> member paths
    let mut package_to_member = HashMap::new();
    for member in members.iter() {
        let member_path = member.as_str().context("Member path is not a string")?;
        let member_cargo_toml = workspace_root.join(member_path).join("Cargo.toml");

        if !member_cargo_toml.exists() {
            continue;
        }

        let content = fs::read_to_string(&member_cargo_toml)?;
        let doc = content.parse::<DocumentMut>()?;

        if let Some(package_name) = doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            package_to_member.insert(package_name.to_string(), member_path);
        }
    }

    // 3. Check each member's dev-dependencies
    let mut total_issues = 0;
    for member in members.iter() {
        let member_path = member.as_str().context("Member path is not a string")?;
        let member_cargo_toml = workspace_root.join(member_path).join("Cargo.toml");

        if !member_cargo_toml.exists() {
            continue;
        }

        let content = fs::read_to_string(&member_cargo_toml)?;
        let doc = content.parse::<DocumentMut>()?;

        if let Some(dev_deps) = doc.get("dev-dependencies").and_then(|dd| dd.as_table()) {
            for (dep_name, dep_value) in dev_deps.iter() {
                // Check if this dependency uses workspace = true
                if let Some(table) = dep_value.as_table_like() {
                    if table
                        .get("workspace")
                        .and_then(|w| w.as_bool())
                        .unwrap_or(false)
                    {
                        // Check if this is a workspace member
                        if package_to_member.contains_key(dep_name) {
                            warn!(
                                "{}/Cargo.toml - `{}` in dev-dependencies should use path = \"...\"",
                                member_path, dep_name
                            );
                            total_issues += 1;
                        }
                    }
                }
            }
        }
    }

    if total_issues == 0 {
        Ok(())
    } else {
        anyhow::bail!(
            "Found {} dev-dependencies using 'workspace = true' that should use path",
            total_issues
        )
    }
}
