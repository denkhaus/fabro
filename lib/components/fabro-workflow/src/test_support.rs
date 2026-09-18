use std::sync::Arc;

use fabro_types::ModelRef;
use lithos_llm::catalog::{ModelId, builtin};
use lithos_llm::types::{Cost, CostSource, TokenCounts, Usage};

use crate::event::{Emitter, Event, append_event};

/// Construct a fully-populated `ModelUsage` for tests: `input_tokens` and
/// `output_tokens` on an OpenAI model, priced from the catalog at one micro
/// per token. Centralised so callers don't keep rebuilding the same skeleton.
#[must_use]
pub fn test_usage(
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> fabro_types::ModelUsage {
    fabro_types::ModelUsage::new(
        ModelRef::new(builtin::openai(), ModelId::new(model_id)),
        Usage {
            tokens: TokenCounts {
                input: input_tokens,
                output: output_tokens,
                ..TokenCounts::default()
            },
            cost:   Some(Cost {
                usd_micros: input_tokens.saturating_add(output_tokens),
                source:     CostSource::Catalog,
            }),
        },
    )
}

/// Append the `RunStartRequested → RunRunnable → RunStarting → RunRunning`
/// sequence so subsequent calls observe the run as live.
pub async fn mark_run_running(run_store: &fabro_store::RunDatabase, run_id: &fabro_types::RunId) {
    append_event(run_store, run_id, &Event::RunStartRequested {
        resume: false,
        actor:  None,
    })
    .await
    .expect("seed run.start_requested");
    append_event(run_store, run_id, &Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .expect("seed run.runnable");
    append_event(run_store, run_id, &Event::RunStarting)
        .await
        .expect("seed run.starting");
    append_event(run_store, run_id, &Event::RunRunning)
        .await
        .expect("seed run.running");
}

/// Record every event the emitter publishes, for assertions after a run.
pub fn collect_events(emitter: &Emitter) -> Arc<std::sync::Mutex<Vec<fabro_types::RunEvent>>> {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    emitter.on_event(move |event| captured.lock().unwrap().push(event.clone()));
    events
}
