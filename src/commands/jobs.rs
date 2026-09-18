use {
    anyhow::{anyhow, Context, Result},
    log::debug,
    std::{env, fs, path::Path, process::Command},
};

/// Memory budget per job, carried over from the 4gb/thread limit in agave's
/// `ci/common/limit-threads.sh`.
const BYTES_PER_JOB: u64 = 4 * 1024 * 1024 * 1024;

const MEMINFO_PATH: &str = "/proc/meminfo";

pub fn run() -> Result<()> {
    println!("{}", jobs()?);

    Ok(())
}

/// Number of parallel jobs a host can afford, capped by both memory and CPUs.
///
/// A pre-set `JOBS` wins outright so callers can pin the value; `CI_HOST_SLOTS`
/// splits the budget between jobs sharing the host.
pub fn jobs() -> Result<usize> {
    if let Some(jobs) = env_count("JOBS")? {
        debug!("using JOBS override: {jobs}");
        return Ok(jobs);
    }

    let memory = total_memory_bytes()?;
    let by_memory = usize::try_from(div_round(memory, BYTES_PER_JOB))?;
    let cpus = cpu_count()?;
    let mut jobs = by_memory.min(cpus);
    debug!("memory allows {by_memory} job(s), cpus allow {cpus}");

    let slots = env_count("CI_HOST_SLOTS")?.unwrap_or(1);
    if slots > 1 {
        jobs = jobs.div_ceil(slots);
        debug!("split across {slots} host slot(s): {jobs} job(s)");
    }

    Ok(jobs.max(1))
}

fn env_count(name: &str) -> Result<Option<usize>> {
    match env::var(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(
            value
                .trim()
                .parse()
                .context(format!("failed to parse {name}=`{value}`"))?,
        )),
        Err(_) => Ok(None),
    }
}

fn total_memory_bytes() -> Result<u64> {
    if Path::new(MEMINFO_PATH).is_file() {
        let meminfo =
            fs::read_to_string(MEMINFO_PATH).context(format!("failed to read {MEMINFO_PATH}"))?;

        return parse_meminfo(&meminfo);
    }

    sysctl_memsize()
}

/// `MemTotal` is reported in kB.
fn parse_meminfo(meminfo: &str) -> Result<u64> {
    let kilobytes = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .ok_or_else(|| anyhow!("no MemTotal in {MEMINFO_PATH}"))?
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty MemTotal in {MEMINFO_PATH}"))?;

    kilobytes
        .parse::<u64>()
        .context(format!("failed to parse MemTotal `{kilobytes}`"))?
        .checked_mul(1024)
        .ok_or_else(|| anyhow!("MemTotal `{kilobytes}` kB overflows"))
}

fn sysctl_memsize() -> Result<u64> {
    let output = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .context("failed to run `sysctl -n hw.memsize`")?;

    if !output.status.success() {
        return Err(anyhow!(
            "`sysctl -n hw.memsize` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let memsize = String::from_utf8(output.stdout).context("`sysctl` output is not utf-8")?;

    memsize
        .trim()
        .parse()
        .context(format!("failed to parse hw.memsize `{}`", memsize.trim()))
}

fn cpu_count() -> Result<usize> {
    Ok(std::thread::available_parallelism()
        .context("failed to determine cpu count")?
        .get())
}

/// Rounds to nearest, as the shell version's `printf "%.0f"` did.
fn div_round(numerator: u64, denominator: u64) -> u64 {
    numerator
        .saturating_add(denominator / 2)
        .checked_div(denominator)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn meminfo_is_parsed_as_kilobytes() {
        let meminfo = "\
MemTotal:       32819516 kB
MemFree:         1234567 kB
";
        assert_eq!(parse_meminfo(meminfo).unwrap(), 32819516 * 1024);
    }

    #[test]
    fn missing_memtotal_is_an_error() {
        assert!(parse_meminfo("MemFree: 1234 kB\n").is_err());
    }

    #[test]
    fn memory_is_rounded_to_nearest_job() {
        assert_eq!(div_round(16 * GIB, BYTES_PER_JOB), 4);
        assert_eq!(div_round(30 * GIB, BYTES_PER_JOB), 8);
        assert_eq!(div_round(29 * GIB, BYTES_PER_JOB), 7);
        assert_eq!(div_round(GIB, BYTES_PER_JOB), 0);
    }
}
