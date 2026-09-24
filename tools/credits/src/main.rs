//! `credits [--check]`: writes `docs/credits-crates.md`, the Rust crates in Forge's build with
//! their version, licence, authors and repository, from `cargo metadata`. `--check` writes
//! nothing and exits with 1 when the file is out of date (CI runs it).
//!
//! "In the build" means every crate a workspace member reaches through normal or build
//! dependencies, on every platform and with every feature (Tracy, DLSS), so the list does not
//! depend on the machine that generates it. Dev-only crates (tests, benchmarks) are left out.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(about = "List the Rust crates in Forge's build")]
struct Args {
    /// Compare instead of writing: exit with 1 when the file is out of date.
    #[arg(long)]
    check: bool,
}

/// One crate name, possibly at several versions.
#[derive(Default)]
struct Entry {
    versions: BTreeSet<String>,
    licenses: BTreeSet<String>,
    authors: BTreeSet<String>,
    repository: Option<String>,
    /// The workspace members that depend on it directly (empty for a transitive crate).
    used_by: BTreeSet<String>,
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
        ])
        .current_dir(&root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).context("parsing metadata")?;
    let text = render(&collect(&metadata)?);

    let path = root.join("docs/credits-crates.md");
    if args.check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current.replace('\r', "") != text {
            eprintln!(
                "docs/credits-crates.md is out of date: run `cargo run -p credits` and credit \
                 the new crates' people in CREDITS.md where they matter"
            );
            return Ok(ExitCode::from(1));
        }
        println!("docs/credits-crates.md is up to date");
    } else {
        std::fs::write(&path, text).context("writing docs/credits-crates.md")?;
        println!("wrote docs/credits-crates.md");
    }
    Ok(ExitCode::SUCCESS)
}

/// The external crates reachable from the workspace, by name.
fn collect(metadata: &Value) -> Result<BTreeMap<String, Entry>> {
    let array = |v: &Value, key: &str| -> Result<Vec<Value>> {
        v[key]
            .as_array()
            .cloned()
            .with_context(|| format!("metadata has no `{key}` array"))
    };
    let members: BTreeSet<String> = array(metadata, "workspace_members")?
        .iter()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect();
    let packages: BTreeMap<String, Value> = array(metadata, "packages")?
        .into_iter()
        .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p)))
        .collect();
    let nodes: BTreeMap<String, Value> = array(&metadata["resolve"], "nodes")?
        .into_iter()
        .filter_map(|n| Some((n["id"].as_str()?.to_owned(), n)))
        .collect();
    let name = |id: &str| packages[id]["name"].as_str().unwrap_or("?").to_owned();

    // Normal (kind null) and build dependencies of `id`; a crate listed only under
    // [dev-dependencies] has only the "dev" kind and is left out.
    let deps = |id: &str| -> Vec<String> {
        nodes[id]["deps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|k| matches!(k["kind"].as_str(), None | Some("build")))
            })
            .filter_map(|d| d["pkg"].as_str().map(str::to_owned))
            .collect()
    };

    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = members.iter().cloned().collect();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let from_member = members.contains(&id);
        for dep in deps(&id) {
            if from_member && !members.contains(&dep) {
                entries
                    .entry(name(&dep))
                    .or_default()
                    .used_by
                    .insert(name(&id));
            }
            stack.push(dep);
        }
        if from_member {
            continue;
        }
        let package = &packages[&id];
        let entry = entries.entry(name(&id)).or_default();
        entry
            .versions
            .insert(package["version"].as_str().unwrap_or("?").to_owned());
        entry.licenses.insert(match package["license"].as_str() {
            Some(license) => license.to_owned(),
            None if package["license_file"].is_string() => "see its licence file".to_owned(),
            None => "not stated".to_owned(),
        });
        for author in package["authors"].as_array().into_iter().flatten() {
            if let Some(author) = author.as_str() {
                entry.authors.insert(without_email(author));
            }
        }
        if entry.repository.is_none() {
            entry.repository = package["repository"].as_str().map(str::to_owned);
        }
    }
    Ok(entries)
}

/// "Name <mail>" → "Name": the list names people, it does not republish their addresses.
fn without_email(author: &str) -> String {
    let name = match author.find('<') {
        Some(at) => &author[..at],
        None => author,
    };
    name.trim().to_owned()
}

/// Escapes what would break a Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn join(set: &BTreeSet<String>, separator: &str) -> String {
    set.iter()
        .map(|s| cell(s))
        .collect::<Vec<_>>()
        .join(separator)
}

fn render(entries: &BTreeMap<String, Entry>) -> String {
    let (direct, transitive): (Vec<_>, Vec<_>) =
        entries.iter().partition(|(_, e)| !e.used_by.is_empty());
    let row = |name: &str, e: &Entry| {
        let repository = e
            .repository
            .as_deref()
            .map(|r| format!("<{}>", cell(r)))
            .unwrap_or_default();
        format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(name),
            join(&e.versions, ", "),
            join(&e.licenses, "; "),
            join(&e.authors, ", "),
            repository
        )
    };
    let header = "| Crate | Version | Licence | Authors | Repository |\n|---|---|---|---|---|\n";

    let mut out = String::from(
        "# Rust crates in Forge's build\n\n\
         Generated by `cargo run -p credits` from `cargo metadata`: every crate the workspace \
         reaches through normal and build dependencies, on every platform and with every \
         feature, without the dev-only ones. CI fails when this file is out of date. The \
         authors are the ones each crate declares; its repository lists everyone who \
         contributed. The people and projects outside crates.io are in \
         [`CREDITS.md`](../CREDITS.md).\n\n",
    );
    out.push_str(&format!(
        "## Used directly by Forge ({})\n\n{header}",
        direct.len()
    ));
    for (name, e) in &direct {
        out.push_str(&row(name, e));
    }
    out.push_str("\nWhere Forge uses them:\n\n");
    for (name, e) in &direct {
        out.push_str(&format!("- `{}`: {}\n", name, join(&e.used_by, ", ")));
    }
    out.push_str(&format!(
        "\n## Their dependencies ({})\n\n{header}",
        transitive.len()
    ));
    for (name, e) in &transitive {
        out.push_str(&row(name, e));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_dropped_and_cells_escaped() {
        assert_eq!(without_email("Jane Doe <jane@example.org>"), "Jane Doe");
        assert_eq!(
            without_email("The Rust Project Developers"),
            "The Rust Project Developers"
        );
        assert_eq!(cell("MIT | Apache"), "MIT \\| Apache");
    }
}
