//! Output formatters (SARIF 2.1.0 and Markdown) over the serializable result structs.
//!
//! JSON and text output stay in `config.rs`; nothing here changes the JSON shape of the results.

use std::collections::HashMap;
use std::fmt::Write as _;

use clap::ValueEnum;
use serde_json::{json, Value};

use crate::ops::{BlastRadiusResult, CallersResult, DeadCodeResult, NodeSummary, PrResult};
use crate::pr::{bare_name, split_id};

/// Output format selected with `--format` (or `--json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Text,
    Json,
    Sarif,
    Markdown,
}

/// Hidden marker on the first line so a CI step can find and update its own PR comment.
pub const MARKER: &str = "<!-- ferrograph -->";

/// Default for `--markdown-limit`.
pub const DEFAULT_MARKDOWN_LIMIT: usize = 50;

/// GitHub rejects comments over 65,536 characters; stay well below.
const MAX_MARKDOWN_CHARS: usize = 60_000;

const RULE_ID: &str = "ferrograph/dead-code";
const FINGERPRINT_KEY: &str = "ferrographIdentity/v1";

struct Finding {
    path: String,
    name: String,
    line: u32,
    col: u32,
    message: String,
}

fn finding(id: &str, name: &str, message: String) -> Finding {
    let (path, line, col) = split_id(id);
    Finding {
        path,
        name: name.to_string(),
        line,
        col,
        message,
    }
}

/// Build a SARIF 2.1.0 log. The fingerprint is `path:name:occurrence` (occurrence counted in
/// source order among the reported findings with the same path and name), never the line, so
/// code-scanning alerts survive unrelated edits that shift lines.
// shortcut: occurrence is counted among reported findings, not all functions, so it can shift
// if an earlier same-name function stops being reported; upgrade by passing all functions in.
fn sarif(mut findings: Vec<Finding>) -> Value {
    findings.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let results: Vec<Value> = findings
        .iter()
        .map(|f| {
            let n = seen.entry((f.path.clone(), f.name.clone())).or_insert(0);
            let fingerprint = format!("{}:{}:{n}", f.path, f.name);
            *n += 1;
            json!({
                "ruleId": RULE_ID,
                "level": "warning",
                "message": { "text": f.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.path, "uriBaseId": "%SRCROOT%" },
                        "region": { "startLine": f.line.max(1), "startColumn": f.col.max(1) }
                    }
                }],
                "partialFingerprints": { FINGERPRINT_KEY: fingerprint }
            })
        })
        .collect();
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "ferrograph",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/GarthDB/ferrograph",
                "rules": [{
                    "id": RULE_ID,
                    "name": "DeadCode",
                    "shortDescription": { "text": "Function is not reachable from any entry point" },
                    "defaultConfiguration": { "level": "warning" },
                    "helpUri": "https://github.com/GarthDB/ferrograph#dead-code-entry-points-and-ignores"
                }]
            }},
            "results": results
        }]
    })
}

/// SARIF for `ferrograph dead`.
#[must_use]
pub fn sarif_dead(r: &DeadCodeResult) -> Value {
    sarif(
        r.dead_nodes
            .iter()
            .map(|n| {
                let name = n.payload.as_deref().map_or("<unknown>", bare_name);
                finding(
                    &n.id,
                    name,
                    format!("Function `{name}` is dead code (unreachable from any entry point)"),
                )
            })
            .collect(),
    )
}

/// SARIF for `ferrograph pr`: the newly dead functions.
#[must_use]
pub fn sarif_pr(r: &PrResult) -> Value {
    sarif(
        r.newly_dead
            .iter()
            .map(|f| {
                let why = if f.status == "added_dead" {
                    "added but never called"
                } else {
                    "no longer called after this change"
                };
                finding(&f.id, &f.name, format!("Function `{}` is {why}", f.name))
            })
            .collect(),
    )
}

/// Escape text for a Markdown table cell (also neutralises HTML, so names cannot inject markup).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '|' | '`' | '*' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

fn location(id: &str) -> String {
    let (path, line, _) = split_id(id);
    esc(&format!("{path}:{line}"))
}

fn node_name(n: &NodeSummary) -> String {
    esc(n.payload.as_deref().map_or(n.id.as_str(), bare_name))
}

fn table(out: &mut String, header: &[&str], rows: &[Vec<String>], limit: usize) {
    if rows.is_empty() {
        out.push_str("_None._\n");
        return;
    }
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let _ = writeln!(out, "|{}|", vec!["---"; header.len()].join("|"));
    for r in rows.iter().take(limit) {
        let _ = writeln!(out, "| {} |", r.join(" | "));
    }
    if rows.len() > limit {
        let _ = writeln!(out, "\n... and {} more", rows.len() - limit);
    }
}

fn finish(mut out: String) -> String {
    if out.len() > MAX_MARKDOWN_CHARS {
        let mut cut = MAX_MARKDOWN_CHARS;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(nl) = out[..cut].rfind('\n') {
            cut = nl;
        }
        out.truncate(cut);
        out.push_str(
            "\n\n_Output truncated to fit GitHub's comment size limit; use `--format json` for the full list._\n",
        );
    }
    out
}

fn node_rows(nodes: &[NodeSummary]) -> Vec<Vec<String>> {
    nodes
        .iter()
        .map(|n| vec![node_name(n), location(&n.id), esc(&n.node_type)])
        .collect()
}

/// Markdown for `ferrograph dead`.
#[must_use]
pub fn markdown_dead(r: &DeadCodeResult, limit: usize) -> String {
    let mut out = format!("{MARKER}\n### ferrograph: {} dead node(s)\n\n", r.count);
    let rows: Vec<Vec<String>> = r
        .dead_nodes
        .iter()
        .map(|n| vec![node_name(n), location(&n.id)])
        .collect();
    table(&mut out, &["Function", "Location"], &rows, limit);
    let _ = write!(out, "\n_{}_\n", r.caveat);
    finish(out)
}

/// Count with a `+` when the true value is only a lower bound.
#[must_use]
pub fn count_label(count: usize, truncated: bool) -> String {
    format!("{count}{}", if truncated { "+" } else { "" })
}

/// Markdown for `ferrograph pr`.
#[must_use]
pub fn markdown_pr(r: &PrResult, limit: usize) -> String {
    let mut out = format!(
        "{MARKER}\n### ferrograph: {} newly dead, blast radius {}\n\n",
        r.newly_dead_count,
        count_label(r.blast.count, false)
    );
    out.push_str("**Newly dead functions**\n\n");
    let rows: Vec<Vec<String>> = r
        .newly_dead
        .iter()
        .map(|f| {
            vec![
                esc(&f.name),
                esc(&format!("{}:{}", f.file, f.line)),
                f.status.to_string(),
            ]
        })
        .collect();
    table(&mut out, &["Function", "Location", "Kind"], &rows, limit);
    out.push_str("\n**Changed functions**\n\n");
    let rows: Vec<Vec<String>> = r
        .changed_functions
        .iter()
        .map(|f| {
            vec![
                esc(&f.name),
                esc(&format!("{}:{}", f.file, f.start_line)),
                count_label(f.blast_count, false),
            ]
        })
        .collect();
    table(&mut out, &["Function", "Location", "Blast"], &rows, limit);
    finish(out)
}

/// Markdown for `ferrograph blast`.
#[must_use]
pub fn markdown_blast(r: &BlastRadiusResult, limit: usize) -> String {
    let mut out = format!(
        "{MARKER}\n### ferrograph: blast radius of {} is {} node(s)\n\n",
        esc(&r.from_node_id),
        count_label(r.count, false)
    );
    table(
        &mut out,
        &["Node", "Location", "Type"],
        &node_rows(&r.reachable_nodes),
        limit,
    );
    finish(out)
}

/// Markdown for `ferrograph callers`.
#[must_use]
pub fn markdown_callers(r: &CallersResult, limit: usize) -> String {
    let mut out = format!(
        "{MARKER}\n### ferrograph: {} caller(s) of {}\n\n",
        count_label(r.count, false),
        esc(&r.node_id)
    );
    table(
        &mut out,
        &["Node", "Location", "Type"],
        &node_rows(&r.callers),
        limit,
    );
    finish(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{NewlyDeadFunction, PrBlast};

    fn node(id: &str, payload: &str) -> NodeSummary {
        NodeSummary {
            id: id.into(),
            node_type: "function".into(),
            payload: Some(payload.into()),
        }
    }

    fn dead(nodes: Vec<NodeSummary>) -> DeadCodeResult {
        DeadCodeResult {
            count: nodes.len(),
            dead_nodes: nodes,
            source: "computed".into(),
            caveat: "caveat",
        }
    }

    #[test]
    fn sarif_structure() {
        let v = sarif_dead(&dead(vec![node("./src/a.rs#10:5", "pub::foo")]));
        assert_eq!(v["version"], "2.1.0");
        assert!(v["$schema"].as_str().unwrap().contains("sarif-2.1.0"));
        let run = &v["runs"][0];
        assert_eq!(run["tool"]["driver"]["name"], "ferrograph");
        assert_eq!(run["tool"]["driver"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(run["tool"]["driver"]["rules"][0]["id"], RULE_ID);
        let r = &run["results"][0];
        assert_eq!(r["ruleId"], RULE_ID);
        assert_eq!(r["level"], "warning");
        assert!(r["message"]["text"].as_str().unwrap().contains("`foo`"));
        let p = &r["locations"][0]["physicalLocation"];
        assert_eq!(p["artifactLocation"]["uri"], "src/a.rs");
        assert_eq!(p["artifactLocation"]["uriBaseId"], "%SRCROOT%");
        assert_eq!(p["region"]["startLine"], 10);
        assert_eq!(p["region"]["startColumn"], 5);
    }

    #[test]
    fn fingerprint_ignores_line_shifts_and_counts_duplicates() {
        let fp = |nodes| {
            sarif_dead(&dead(nodes))["runs"][0]["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r["partialFingerprints"][FINGERPRINT_KEY]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        let before = fp(vec![
            node("./src/a.rs#10:5", "foo"),
            node("./src/a.rs#30:5", "foo"),
        ]);
        let after = fp(vec![
            node("./src/a.rs#40:5", "foo"),
            node("./src/a.rs#22:5", "foo"),
        ]);
        assert_eq!(before, ["src/a.rs:foo:0", "src/a.rs:foo:1"]);
        assert_eq!(before, after);
    }

    #[test]
    fn sarif_pr_lists_newly_dead() {
        let r = PrResult {
            base: "main".into(),
            merge_base: "m".into(),
            head: "h".into(),
            changed_files: vec![],
            newly_dead: vec![NewlyDeadFunction {
                id: "./src/a.rs#3:1".into(),
                file: "src/a.rs".into(),
                name: "foo".into(),
                line: 3,
                status: "added_dead",
            }],
            newly_dead_count: 1,
            changed_functions: vec![],
            changed_without_function: vec![],
            blast: PrBlast {
                count: 0,
                shown: 0,
                nodes: vec![],
            },
            coupling: vec![],
            warnings: vec![],
        };
        let v = sarif_pr(&r);
        assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 1);
        let md = markdown_pr(&r, 50);
        assert!(md.starts_with(MARKER));
        assert!(md.contains("| foo | src/a.rs:3 | added_dead |"));
    }

    #[test]
    fn markdown_escapes_and_caps_rows() {
        let nodes: Vec<_> = (0..5)
            .map(|i| node(&format!("./src/a.rs#{i}:1"), "a|b`c<d>"))
            .collect();
        let md = markdown_dead(&dead(nodes), 2);
        assert!(md.starts_with("<!-- ferrograph -->\n"));
        assert!(md.contains("a\\|b\\`c&lt;d&gt;"));
        assert_eq!(md.matches("\n| a").count(), 2);
        assert!(md.contains("... and 3 more"));
    }

    #[test]
    fn markdown_is_truncated_below_github_limit() {
        let long = "x".repeat(400);
        let nodes: Vec<_> = (0..1000)
            .map(|i| node(&format!("./src/a.rs#{i}:1"), &long))
            .collect();
        let md = markdown_dead(&dead(nodes), 1000);
        assert!(md.len() < 61_000, "{}", md.len());
        assert!(md.contains("truncated"));
    }
}
