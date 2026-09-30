//! Manual replay vehicle (ADR-0022 wave 2, fabro-d4c6): re-asks the S1
//! verdict pre-screen over the recorded review corpus and writes the
//! agreement + confidence report.
//!
//! NOT part of the quality gate: a live run needs `OPENROUTER_API_KEY`
//! from the environment (never logged, never echoed); without a key the
//! tool still extracts the corpus and writes a report whose agreement
//! fields are null (extract-only mode). The gate exercises the harness
//! through the scripted-twin unit tests in `judgment_replay`.
//!
//! Usage:
//! ```text
//! cargo run -p fabro-llm --example judgment_replay -- \
//!     [--journal-dir .fabro/journal] [--limit 100] \
//!     [--report-out docs/lab/judgment-replay-report.json]
//! ```

use std::path::PathBuf;

use clap::Parser;
use fabro_http::http_client;
use fabro_llm::judgment::{JudgmentClient, JudgmentEndpoint};
use fabro_llm::judgment_replay::{ReplayReport, load_corpus, replay};
use fabro_static::EnvVars;
use lithos_llm::middleware::RetryPolicy;

/// A no-retry policy: a manual replay run degrades a failing cycle instead
/// of burning the operator's rate budget on it.
fn manual_policy() -> RetryPolicy {
    RetryPolicy::exponential()
        .max_attempts(1)
        .initial_delay(std::time::Duration::from_millis(1))
        .max_delay(std::time::Duration::from_millis(2))
}

#[derive(Debug, Parser)]
struct Args {
    /// Journal directory holding one `<run_id>.jsonl` per run.
    #[arg(long, default_value = ".fabro/journal")]
    journal_dir: PathBuf,
    /// How many cycles to replay (deterministic selection, 50-100
    /// recommended).
    #[arg(long, default_value = "100")]
    limit:       usize,
    /// Where to write the JSON report; "-" writes stdout.
    #[arg(long, default_value = "-")]
    report_out:  PathBuf,
    /// Ops seam for scripted doubles; default is the pinned OpenRouter
    /// System One surface.
    #[arg(long)]
    endpoint:    Option<String>,
    /// Version-pinned model override (default: the crate's pinned id).
    #[arg(long)]
    model:       Option<String>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "manual vehicle reads the operator key from env once at startup; the library never reads env itself"
)]
#[expect(
    clippy::print_stdout,
    reason = "report JSON is the tool's stdout contract"
)]
#[expect(clippy::print_stderr, reason = "progress notes go to stderr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cycles = load_corpus(&args.journal_dir, Some(args.limit))?;
    eprintln!(
        "corpus: {} cycles extracted from {} (limit {})",
        cycles.len(),
        args.journal_dir.display(),
        args.limit
    );

    let default_endpoint = JudgmentEndpoint::default();
    let endpoint = JudgmentEndpoint {
        base_url: args.endpoint.unwrap_or(default_endpoint.base_url),
        model:    args.model.unwrap_or(default_endpoint.model),
    };
    let api_key = std::env::var(EnvVars::OPENROUTER_API_KEY)
        .ok()
        .filter(|key| !key.is_empty());
    if api_key.is_none() {
        eprintln!(
            "OPENROUTER_API_KEY not set: extract-only run, agreement fields \
             will be null"
        );
    }
    let http = http_client()?;
    let client = JudgmentClient::new(http, endpoint.clone(), manual_policy(), None);

    let outcomes = replay(&client, api_key.as_deref(), &cycles).await;
    let report = ReplayReport::from_outcomes(&outcomes);
    let json = serde_json::to_string_pretty(&report)?;
    if args.report_out.as_os_str() == "-" {
        println!("{json}");
    } else {
        std::fs::write(&args.report_out, format!("{json}\n"))?;
        eprintln!("report written to {}", args.report_out.display());
    }
    Ok(())
}
