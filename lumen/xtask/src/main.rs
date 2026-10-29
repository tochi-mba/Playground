//! Developer tasks: `cargo xtask ci` runs the same gate as the CI workflow, locally.
//!
//! Steps: fmt check, clippy (all targets, all features, warnings denied), rustdoc, cargo-deny,
//! tests (nextest when installed, `cargo test` otherwise) and, when `cargo-llvm-cov` is
//! installed, a coverage report with the same floors CI enforces.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::{bail, Context};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimum line coverage for the whole workspace, in percent.
pub const COVERAGE_FLOOR_TOTAL: f64 = 85.0;
/// Minimum line coverage for `lumen-core`, in percent.
pub const COVERAGE_FLOOR_CORE: f64 = 95.0;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn run(step: &str, mut cmd: Command) -> anyhow::Result<()> {
    println!("==> {step}");
    let status = cmd
        .current_dir(root())
        .status()
        .with_context(|| format!("running {step}"))?;
    if !status.success() {
        bail!("{step} failed ({status})");
    }
    Ok(())
}

fn cargo(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO"));
    c.args(args);
    c
}

fn has_subcommand(name: &str) -> bool {
    Command::new(env!("CARGO"))
        .args([name, "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn ci() -> anyhow::Result<()> {
    run("fmt", cargo(&["fmt", "--all", "--check"]))?;
    run(
        "clippy",
        cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    let mut doc = cargo(&["doc", "--workspace", "--no-deps", "--all-features"]);
    doc.env("RUSTDOCFLAGS", "-D warnings");
    run("doc", doc)?;
    if has_subcommand("deny") {
        run("deny", cargo(&["deny", "check"]))?;
    } else {
        println!("==> deny skipped (cargo-deny not installed)");
    }
    if has_subcommand("nextest") {
        run(
            "test",
            cargo(&["nextest", "run", "--workspace", "--all-features"]),
        )?;
        run(
            "doctest",
            cargo(&["test", "--workspace", "--all-features", "--doc"]),
        )?;
    } else {
        run("test", cargo(&["test", "--workspace", "--all-features"]))?;
    }
    if has_subcommand("llvm-cov") {
        coverage()?;
    } else {
        println!("==> coverage skipped (cargo-llvm-cov not installed)");
    }
    println!("==> all green");
    Ok(())
}

/// Runs cargo-llvm-cov and enforces the coverage floors.
fn coverage() -> anyhow::Result<()> {
    println!("==> coverage");
    let output = Command::new(env!("CARGO"))
        .args([
            "llvm-cov",
            "--workspace",
            "--all-features",
            "--json",
            "--summary-only",
        ])
        .current_dir(root())
        .output()
        .context("running cargo llvm-cov")?;
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        bail!("cargo llvm-cov failed");
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let report = parse_coverage(&json)?;
    println!(
        "    total lines: {:.2}%  lumen-core: {:.2}%",
        report.total, report.core
    );
    check_floors(&report)
}

/// Line coverage percentages extracted from `cargo llvm-cov --json --summary-only`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverageReport {
    /// Whole workspace.
    pub total: f64,
    /// `lumen-core` only.
    pub core: f64,
}

/// Parses the llvm-cov JSON export without a JSON dependency: it needs only the total line
/// percentage and the per-file line percentages for files under `crates/lumen-core/`.
pub fn parse_coverage(json: &str) -> anyhow::Result<CoverageReport> {
    let total = find_percent(json, "\"totals\"").context("no totals in coverage json")?;
    // Per-file entries look like {"filename":"...","summary":{..."lines":{"count":N,"covered":M,"percent":P}}}.
    let (mut covered, mut count) = (0.0, 0.0);
    let mut rest = json;
    while let Some(i) = rest.find("\"filename\":\"") {
        let after = &rest[i + 12..];
        let Some(end) = after.find('"') else { break };
        let filename = &after[..end];
        let section_end = after.find("\"filename\":\"").unwrap_or(after.len());
        let section = &after[..section_end];
        if filename.contains("crates/lumen-core/") || filename.contains("crates\\lumen-core\\") {
            if let Some(lines) = section.find("\"lines\":{") {
                let l = &section[lines..];
                covered += number_after(l, "\"covered\":").unwrap_or(0.0);
                count += number_after(l, "\"count\":").unwrap_or(0.0);
            }
        }
        rest = after;
    }
    let core = if count > 0.0 {
        covered / count * 100.0
    } else {
        0.0
    };
    Ok(CoverageReport { total, core })
}

fn find_percent(json: &str, key: &str) -> Option<f64> {
    let i = json.find(key)?;
    let section = &json[i..];
    let lines = section.find("\"lines\":{")?;
    number_after(&section[lines..], "\"percent\":")
}

fn number_after(s: &str, key: &str) -> Option<f64> {
    let i = s.find(key)? + key.len();
    let rest = &s[i..];
    let end = rest
        .find(|c: char| {
            !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+')
        })
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Fails when either floor is missed.
pub fn check_floors(r: &CoverageReport) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    if r.total < COVERAGE_FLOOR_TOTAL {
        problems.push(format!(
            "workspace line coverage {:.2}% is below the {COVERAGE_FLOOR_TOTAL}% floor",
            r.total
        ));
    }
    if r.core < COVERAGE_FLOOR_CORE {
        problems.push(format!(
            "lumen-core line coverage {:.2}% is below the {COVERAGE_FLOOR_CORE}% floor",
            r.core
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        bail!("{}", problems.join("; "))
    }
}

fn main() -> anyhow::Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "ci" => ci(),
        "coverage" => coverage(),
        "check-coverage" => {
            let path = std::env::args()
                .nth(2)
                .context("usage: xtask check-coverage <llvm-cov.json>")?;
            let json = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
            let report = parse_coverage(&json)?;
            println!(
                "total lines: {:.2}%  lumen-core: {:.2}%",
                report.total, report.core
            );
            check_floors(&report)
        }
        _ => bail!("usage: cargo xtask <ci|coverage|check-coverage FILE>"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"data":[{"files":[
      {"filename":"/w/crates/lumen-core/src/geom.rs","summary":{"functions":{"count":1},"lines":{"count":100,"covered":99,"percent":99.0}}},
      {"filename":"/w/crates/lumen-core/src/warp.rs","summary":{"lines":{"count":100,"covered":93,"percent":93.0}}},
      {"filename":"/w/crates/lumen-app/src/main.rs","summary":{"lines":{"count":50,"covered":10,"percent":20.0}}}
    ],"totals":{"lines":{"count":250,"covered":202,"percent":80.8}}}]}"#;

    #[test]
    fn parses_totals_and_core_subset() {
        let r = parse_coverage(SAMPLE).unwrap();
        assert_eq!(r.total, 80.8);
        assert_eq!(r.core, 96.0);
        assert!(parse_coverage("{}").is_err());
        assert_eq!(
            parse_coverage(r#"{"totals":{"lines":{"percent":50}}}"#)
                .unwrap()
                .core,
            0.0
        );
    }

    #[test]
    fn floors() {
        assert!(check_floors(&CoverageReport {
            total: 85.0,
            core: 95.0
        })
        .is_ok());
        let e = check_floors(&CoverageReport {
            total: 84.9,
            core: 90.0,
        })
        .unwrap_err()
        .to_string();
        assert!(e.contains("workspace") && e.contains("lumen-core"));
        assert!(check_floors(&CoverageReport {
            total: 99.0,
            core: 94.9
        })
        .is_err());
    }

    #[test]
    fn number_parsing() {
        assert_eq!(
            number_after(r#""percent":12.5,"x""#, "\"percent\":"),
            Some(12.5)
        );
        assert_eq!(number_after("nothing", "\"percent\":"), None);
        assert_eq!(number_after("\"percent\":abc", "\"percent\":"), None);
    }
}
