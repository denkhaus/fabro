//! Lossless blob offload for oversized run-event payloads (fabro-5082).
//!
//! `bound_run_event` shrinks oversized events lossily so the run-store
//! append cannot fail with HTTP 413 (fabro-a723). This module runs first
//! wherever a run store with blob access is available — the
//! [`crate::event::RunEventSink::Store`] arm of the sink layer: bulky
//! `checkpoint.completed` payloads (diff, context values, node-outcome
//! payloads) are persisted as content-addressed blobs and replaced by
//! inline `blob://sha256/...` references, so the audit trail stays lossless
//! and readers that already parse blob refs hydrate the full payload.
//! Truncation remains only as the fallback for blob-write failures and
//! events that still exceed the budget after offload.

use ::fabro_types::{EventBody, RunEvent, format_blob_ref, parse_blob_ref, run_event_body_budget};
use serde_json::Value;

use crate::artifact::{BLOB_OFFLOAD_THRESHOLD, offload_value};
use crate::runtime_store::RunStoreHandle;

/// Summary of one event's offload pass, for logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RunEventOffload {
    /// Number of payload fields replaced by blob references.
    pub values: usize,
    /// Original serialized size of the offloaded payloads.
    pub bytes:  usize,
}

/// Smallest payload the budget pass considers offloading: below this the
/// inline blob reference is not meaningfully smaller than the value itself.
const MIN_OFFLOADABLE_BYTES: usize = 512;

/// Offload the bulky payloads of an oversized event into `run_store`,
/// replacing them with inline blob references until the serialized event
/// fits [`run_event_body_budget`].
///
/// Returns `Ok(None)` when the event already fits the budget or carries no
/// offloadable payloads; the lossy `bound_run_event` fallback still applies
/// afterwards. Blob-write failures abort the pass and surface as `Err` so
/// the caller can log the fallback explicitly.
pub(crate) async fn offload_oversized_run_event(
    event: &mut RunEvent,
    run_store: &RunStoreHandle,
) -> anyhow::Result<Option<RunEventOffload>> {
    let budget = run_event_body_budget();
    if serialized_len(event) <= budget {
        return Ok(None);
    }
    let EventBody::CheckpointCompleted(props) = &mut event.body else {
        return Ok(None);
    };

    let mut summary = RunEventOffload::default();

    // Pass 1: offload every payload over the artifact blob threshold — the
    // same ceiling context updates already offload at.
    if props
        .diff
        .as_ref()
        .is_some_and(|diff| diff.len() > BLOB_OFFLOAD_THRESHOLD)
    {
        // Write the blob first: a failed write must leave the payload in
        // place for the truncation fallback, not drop it.
        let diff = props.diff.as_deref().expect("checked above");
        let reference = blob_ref_for(diff.as_bytes(), run_store).await?;
        summary.values += 1;
        summary.bytes += diff.len();
        props.diff = Some(reference);
    }
    for value in props.context_values.values_mut() {
        offload_value_counted(value, run_store, &mut summary).await?;
    }
    for outcome in props.node_outcomes.values_mut() {
        for value in outcome.context_updates.values_mut() {
            offload_value_counted(value, run_store, &mut summary).await?;
        }
        if outcome
            .notes
            .as_ref()
            .is_some_and(|notes| notes.len() > BLOB_OFFLOAD_THRESHOLD)
        {
            let notes = outcome.notes.as_deref().expect("checked above");
            let reference = blob_ref_for(notes.as_bytes(), run_store).await?;
            summary.values += 1;
            summary.bytes += notes.len();
            outcome.notes = Some(reference);
        }
    }

    // Pass 2: many medium-sized entries can exceed the budget together even
    // though each fits the per-entry threshold. Keep offloading the largest
    // remaining payload until the event fits or nothing is left to offload.
    while serialized_len(event) > budget {
        let Some((locator, bytes)) = largest_offloadable_payload(event) else {
            break;
        };
        let reference = blob_ref_for(&bytes, run_store).await?;
        apply_reference(event, &locator, reference);
        summary.values += 1;
        summary.bytes += bytes.len();
    }

    Ok((summary.values > 0).then_some(summary))
}

/// Persist `bytes` as a blob and return the inline reference for it.
async fn blob_ref_for(bytes: &[u8], run_store: &RunStoreHandle) -> anyhow::Result<String> {
    let blob_hash = run_store
        .write_blob(bytes)
        .await
        .map_err(|err| anyhow::anyhow!("run event blob write failed: {err}"))?;
    Ok(format_blob_ref(&blob_hash))
}

/// [`crate::artifact::offload_value`] with offload accounting. The value
/// threshold check lives inside the shared helper.
async fn offload_value_counted(
    value: &mut Value,
    run_store: &RunStoreHandle,
    summary: &mut RunEventOffload,
) -> anyhow::Result<()> {
    let before = serde_json::to_vec(&*value)
        .map_err(|err| anyhow::anyhow!("run event payload serialize failed: {err}"))?
        .len();
    offload_value(value, run_store)
        .await
        .map_err(|err| anyhow::anyhow!("run event blob write failed: {err}"))?;
    if value
        .as_str()
        .is_some_and(|text| parse_blob_ref(text).is_some())
    {
        summary.values += 1;
        summary.bytes += before;
    }
    Ok(())
}

/// Address of one offloadable payload inside a `checkpoint.completed` body.
enum Locator {
    Diff,
    ContextValue(String),
    OutcomeUpdate { node: String, key: String },
    OutcomeNotes(String),
}

/// Serialized bytes a locator's payload would contribute, or `None` for
/// values that can never reach the threshold.
fn serialized_payload(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
        Value::String(text) if text.len().saturating_mul(6) + 2 <= MIN_OFFLOADABLE_BYTES => None,
        _ => serde_json::to_vec(value).ok(),
    }
}

fn string_payload_is_offloadable(text: &str) -> bool {
    text.len() >= MIN_OFFLOADABLE_BYTES && parse_blob_ref(text).is_none()
}

/// Find the largest payload that pass 2 can still offload, with the exact
/// bytes the blob store should persist for it.
fn largest_offloadable_payload(event: &RunEvent) -> Option<(Locator, Vec<u8>)> {
    let EventBody::CheckpointCompleted(props) = &event.body else {
        return None;
    };
    let mut best: Option<(usize, Locator, Vec<u8>)> = None;
    let mut consider = |size: usize, locator: Locator, bytes: Vec<u8>| {
        if size >= MIN_OFFLOADABLE_BYTES && best.as_ref().is_none_or(|(best, _, _)| size > *best) {
            best = Some((size, locator, bytes));
        }
    };

    if let Some(diff) = props.diff.as_ref() {
        if string_payload_is_offloadable(diff) {
            consider(diff.len(), Locator::Diff, diff.clone().into_bytes());
        }
    }
    for (key, value) in &props.context_values {
        let already_offloaded = value
            .as_str()
            .is_some_and(|text| parse_blob_ref(text).is_some());
        if already_offloaded {
            continue;
        }
        if let Some(bytes) = serialized_payload(value) {
            consider(bytes.len(), Locator::ContextValue(key.clone()), bytes);
        }
    }
    for (node, outcome) in &props.node_outcomes {
        for (key, value) in &outcome.context_updates {
            let already_offloaded = value
                .as_str()
                .is_some_and(|text| parse_blob_ref(text).is_some());
            if already_offloaded {
                continue;
            }
            if let Some(bytes) = serialized_payload(value) {
                consider(
                    bytes.len(),
                    Locator::OutcomeUpdate {
                        node: node.clone(),
                        key:  key.clone(),
                    },
                    bytes,
                );
            }
        }
        if let Some(notes) = outcome.notes.as_ref() {
            if string_payload_is_offloadable(notes) {
                consider(
                    notes.len(),
                    Locator::OutcomeNotes(node.clone()),
                    notes.clone().into_bytes(),
                );
            }
        }
    }
    best.map(|(_, locator, bytes)| (locator, bytes))
}

/// Replace the payload addressed by `locator` with the inline blob
/// `reference`.
fn apply_reference(event: &mut RunEvent, locator: &Locator, reference: String) {
    let EventBody::CheckpointCompleted(props) = &mut event.body else {
        return;
    };
    match locator {
        Locator::Diff => props.diff = Some(reference),
        Locator::ContextValue(key) => {
            if let Some(value) = props.context_values.get_mut(key) {
                *value = Value::String(reference);
            }
        }
        Locator::OutcomeUpdate { node, key } => {
            if let Some(outcome) = props.node_outcomes.get_mut(node) {
                if let Some(value) = outcome.context_updates.get_mut(key) {
                    *value = Value::String(reference);
                }
            }
        }
        Locator::OutcomeNotes(node) => {
            if let Some(outcome) = props.node_outcomes.get_mut(node) {
                outcome.notes = Some(reference);
            }
        }
    }
}

fn serialized_len(event: &RunEvent) -> usize {
    serde_json::to_vec(event).map_or(0, |body| body.len())
}
