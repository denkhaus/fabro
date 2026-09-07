//! Standalone workflow-graph validator.
//!
//! Parses one or more Graphviz workflow graphs, resolves workflow-relative
//! `@`-file references against each graph's directory (see
//! [`fabro_validate::file_refs`]), and runs the built-in lint rules without
//! building the full CLI or any Rust test harness. This is the engine behind
//! `just validate-workflows`.
//!
//! Exit status is non-zero when any Error-severity diagnostic is produced.

#![allow(
    clippy::print_stdout,
    reason = "CLI validator prints its report to stdout by design."
)]
#![allow(
    clippy::print_stderr,
    reason = "CLI validator prints diagnostics to stderr by design."
)]
#![allow(
    clippy::exit,
    reason = "The CLI exits explicitly with the computed process status."
)]

use std::path::Path;

use fabro_graphviz::parser;
use fabro_validate::file_refs::{resolve_file_refs, strip_templated_stylesheet};
use fabro_validate::{Diagnostic, Severity, validate};

#[expect(
    clippy::disallowed_methods,
    reason = "sync CLI validator reads graph files synchronously"
)]
fn validate_graph_file(path: &Path) -> Result<bool, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|err| format!("{}: failed to read: {err}", path.display()))?;
    let mut graph = parser::parse(&source)
        .map_err(|err| format!("{}: failed to parse: {err}", path.display()))?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    resolve_file_refs(&mut graph, base_dir);
    strip_templated_stylesheet(&mut graph);
    let diagnostics = validate(&graph, &[]);
    let ok = print_diagnostics(path, &diagnostics);
    Ok(ok)
}

fn print_diagnostics(path: &Path, diagnostics: &[Diagnostic]) -> bool {
    let mut ok = true;
    for d in diagnostics {
        let severity = match d.severity {
            Severity::Error => {
                ok = false;
                "error"
            }
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let location = d
            .node_id
            .as_deref()
            .map(|node| format!(" node {node}"))
            .unwrap_or_default();
        eprintln!("{severity}: {}{location}: {}", d.rule, d.message);
        if let Some(fix) = &d.fix {
            eprintln!("  help: {fix}");
        }
    }
    let status = if ok { "OK" } else { "FAILED" };
    println!(
        "{}: {status} ({} diagnostics)",
        path.display(),
        diagnostics.len()
    );
    ok
}

fn main() {
    let paths: Vec<std::path::PathBuf> = std::env::args()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();
    if paths.is_empty() {
        eprintln!("usage: fabro-validate <graph.fabro> [...]");
        std::process::exit(2);
    }
    let mut ok = true;
    for path in &paths {
        match validate_graph_file(path) {
            Ok(graph_ok) => ok &= graph_ok,
            Err(err) => {
                eprintln!("error: {err}");
                ok = false;
            }
        }
    }
    if !ok {
        std::process::exit(1);
    }
}
