use anyhow::{bail, Context, Result};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bench") => bench(),
        Some("dist") => dist(),
        Some(other) => bail!("unknown xtask: {other}"),
        None => {
            eprintln!("usage: cargo xtask <bench|dist>");
            Ok(())
        }
    }
}

fn bench() -> Result<()> {
    // Placeholder — v0.1 wires this up after the release binary stabilises.
    println!("xtask bench: coming in v0.1 after initial release-baseline capture");
    Ok(())
}

fn dist() -> Result<()> {
    let root = workspace_root()?;
    let version = workspace_version(&root.join("Cargo.toml"))?;
    let host = rust_host_triple(&root)?;
    let package_name = format!("capfind-v{version}-{host}");
    let dist_dir = root.join("target/dist");
    let staging_dir = dist_dir.join(&package_name);
    let archive = dist_dir.join(format!("{package_name}.tar.gz"));
    let checksum = dist_dir.join(format!("{package_name}.tar.gz.sha256"));

    run(
        &root,
        "cargo",
        ["build", "--release", "-p", "capfind-cli", "--locked"],
    )?;

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .with_context(|| format!("failed to clean {}", staging_dir.display()))?;
    }
    fs::create_dir_all(&staging_dir)
        .with_context(|| format!("failed to create {}", staging_dir.display()))?;

    let binary = root
        .join("target/release")
        .join(format!("capfind{}", env::consts::EXE_SUFFIX));
    fs::copy(
        &binary,
        staging_dir.join(binary.file_name().unwrap_or(OsStr::new("capfind"))),
    )
    .with_context(|| format!("failed to copy {}", binary.display()))?;

    copy_if_exists(&root.join("README.md"), &staging_dir.join("README.md"))?;

    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    if archive.exists() {
        fs::remove_file(&archive)
            .with_context(|| format!("failed to remove {}", archive.display()))?;
    }

    run(
        &dist_dir,
        "tar",
        [
            "-czf",
            archive.file_name().unwrap().to_str().unwrap(),
            package_name.as_str(),
        ],
    )?;

    let digest = sha256(&archive)?;
    fs::write(
        &checksum,
        format!(
            "{digest}  {}\n",
            archive.file_name().unwrap().to_string_lossy()
        ),
    )
    .with_context(|| format!("failed to write {}", checksum.display()))?;

    println!("created {}", archive.display());
    println!("created {}", checksum.display());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .context("xtask must live under <workspace>/xtask")
}

fn workspace_version(cargo_toml: &Path) -> Result<String> {
    let content = fs::read_to_string(cargo_toml)
        .with_context(|| format!("failed to read {}", cargo_toml.display()))?;
    parse_workspace_version(&content).context("missing [workspace.package] version")
}

fn parse_workspace_version(content: &str) -> Option<String> {
    let mut in_workspace_package = false;
    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_workspace_package = line == "[workspace.package]";
            continue;
        }
        if in_workspace_package {
            if let Some(("version", value)) =
                line.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
            {
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn rust_host_triple(root: &Path) -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(root)
        .output()
        .context("failed to run rustc -vV")?;
    if !output.status.success() {
        bail!("rustc -vV failed with status {}", output.status);
    }
    let stdout = String::from_utf8(output.stdout).context("rustc -vV output was not UTF-8")?;
    parse_host_triple(&stdout).context("rustc -vV did not print host triple")
}

fn parse_host_triple(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
}

fn copy_if_exists(from: &Path, to: &Path) -> Result<()> {
    if from.exists() {
        fs::copy(from, to)
            .with_context(|| format!("failed to copy {} to {}", from.display(), to.display()))?;
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    if let Ok(output) = Command::new("sha256sum").arg(path).output() {
        if output.status.success() {
            let stdout =
                String::from_utf8(output.stdout).context("sha256sum output was not UTF-8")?;
            return stdout
                .split_whitespace()
                .next()
                .map(str::to_string)
                .context("sha256sum output was empty");
        }
    }

    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .context("failed to run sha256sum or shasum")?;
    if !output.status.success() {
        bail!("shasum failed with status {}", output.status);
    }
    let stdout = String::from_utf8(output.stdout).context("shasum output was not UTF-8")?;
    stdout
        .split_whitespace()
        .next()
        .map(str::to_string)
        .context("shasum output was empty")
}

fn run<I, S>(cwd: &Path, program: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if !status.success() {
        bail!("{program} failed with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace_package_version() {
        let version = parse_workspace_version(
            r#"
[package]
version = "0.0.0"

[workspace.package]
version = "0.1.0"
"#,
        );
        assert_eq!(version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn parses_rust_host_triple() {
        let host = parse_host_triple("rustc 1.95.0\nhost: x86_64-unknown-linux-gnu\n");
        assert_eq!(host.as_deref(), Some("x86_64-unknown-linux-gnu"));
    }
}
