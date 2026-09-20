//! The order one pass streams its new items in.

use std::collections::BTreeSet;

use fabro_store::platform_records::StoredPlatformRecord;
use petri_execution::events::{EventSource, RunEvent};
use petri_runtime::engine::Event;

use crate::projection::{FiringKey, Item};

/// The order one pass streams its new items in, and how many platform
/// records it holds back for a later pass.
///
/// Every item is first ordered by `recorded_at` (stable: the coordinator
/// log before an execution log before a platform record on a tie, and each
/// log's own order kept). A platform record that carries a Petri position
/// (a checkpoint, keyed on `(execution, firing)`) is then placed by that
/// position, not by its clock, because the server stamps the record and the
/// worker stamps Petri's records and the two clocks can tie or invert:
///
/// - before the firing's first `routing.resolved` event in the pass, which is
///   right after the firing's finish (its `step.finished` and the
///   `visit.completed` attached to it) and before the next firing's
///   `visit.started`, which is attached to that routing record;
/// - else after the last event of the firing in the pass;
/// - else, when the firing finished in an earlier pass, before the first event
///   of a later firing (a larger firing id) in the same execution, or where its
///   `recorded_at` put it;
/// - else the record is held back, with every platform record after it, and the
///   pass consumes platform records only up to it. The hook that writes a
///   checkpoint record runs after the driver appended the attempt's finish, but
///   the driver's store writer flushes that record on its own schedule, so the
///   platform record can be committed before its firing's `step.finished`;
///   holding it keeps the stream's order the same live and on a rebuild.
///   Nothing is held once the run has recorded its finish.
///
/// The rule reads only the pass's own items and the firings already
/// finished, so a record is never streamed before its firing's finish and
/// never after the firing's routes.
pub(crate) fn order_items<'a>(
    events: &'a [RunEvent],
    platform_records: &'a [StoredPlatformRecord],
    finished_before: &BTreeSet<FiringKey>,
    run_finished: bool,
) -> (Vec<Item<'a>>, usize) {
    let finished_in_pass = |at: FiringKey| {
        events.iter().any(|event| {
            FiringKey::of_event(event) == Some(at)
                && matches!(event.engine(), Some(Event::StepFinished { .. }))
        })
    };
    let finished = |at: FiringKey| finished_in_pass(at) || finished_before.contains(&at);
    // Platform records are consumed in seq order: the first one whose firing
    // has not finished holds itself and everything after it.
    let consumed = if run_finished {
        platform_records.len()
    } else {
        platform_records
            .iter()
            .position(|record| {
                record
                    .position
                    .is_some_and(|position| !finished(FiringKey::from(position)))
            })
            .unwrap_or(platform_records.len())
    };
    let held = platform_records.len() - consumed;
    let platform_records = &platform_records[..consumed];

    let mut items: Vec<(u64, u8, Item<'a>)> =
        Vec::with_capacity(events.len() + platform_records.len());
    for event in events {
        let rank = match event.id.source {
            EventSource::Coordinator => 0,
            EventSource::Execution { .. } => 1,
        };
        items.push((event.recorded_at, rank, Item::Petri(event)));
    }
    for record in platform_records {
        items.push((record.recorded_at, 2, Item::Platform(record)));
    }
    items.sort_by_key(|(recorded_at, rank, _)| (*recorded_at, *rank));

    let item_firing = |item: &Item<'a>| match item {
        Item::Petri(event) => FiringKey::of_event(event),
        Item::Platform(_) => None,
    };
    let is_routing = |item: &Item<'a>| {
        matches!(
            item,
            Item::Petri(event) if matches!(event.engine(), Some(Event::RoutingResolved { .. }))
        )
    };
    // The key of each item: its index in clock order, and whether it sits
    // before (0), at (1) or after (2) that index.
    let mut keys: Vec<(usize, u8)> = (0..items.len()).map(|index| (index, 1)).collect();
    for (index, (_, _, item)) in items.iter().enumerate() {
        let Item::Platform(record) = item else {
            continue;
        };
        let Some(position) = record.position else {
            continue;
        };
        let at = FiringKey::from(position);
        let first_routing = items
            .iter()
            .position(|(_, _, other)| item_firing(other) == Some(at) && is_routing(other));
        let last_of_firing = items
            .iter()
            .rposition(|(_, _, other)| item_firing(other) == Some(at));
        let first_later = items.iter().position(|(_, _, other)| {
            item_firing(other)
                .is_some_and(|key| key.execution == at.execution && key.firing > at.firing)
        });
        keys[index] = if let Some(before) = first_routing {
            (before, 0)
        } else if let Some(after) = last_of_firing {
            (after, 2)
        } else if let Some(before) = first_later {
            (before, 0)
        } else {
            (index, 1)
        };
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|index| keys[*index]);
    let mut ordered: Vec<Option<Item<'a>>> =
        items.into_iter().map(|(_, _, item)| Some(item)).collect();
    let items = order
        .into_iter()
        .map(|index| ordered[index].take().expect("each item is placed once"))
        .collect();
    (items, held)
}

#[cfg(test)]
mod tests {
    use fabro_store::PlatformRecord;
    use fabro_store::platform_records::{CheckpointRecord, StagePosition};
    use petri_execution::events::{Context, EventId, NodeRef, Record, RecordOrigin, Subject};
    use petri_execution::{ExecutionId, StoredEngineRecord};
    use petri_runtime::driver::BranchRole;
    use petri_runtime::engine::{DecisionId, EventOrigin, RouteApplied};
    use petri_runtime::ir::{Attempt, FiringId, NodeId, Outcome, Status};

    use super::*;
    use crate::projector::stream;

    /// A firing's engine event at `seq`, recorded at `at`.
    fn engine_event(seq: u64, firing: u64, at: u64, body: Event) -> RunEvent {
        RunEvent {
            id:          EventId {
                source: EventSource::Execution {
                    execution: ExecutionId::new(0),
                },
                seq,
                index: 0,
            },
            origin:      RecordOrigin::External,
            context:     Context {
                invocation: None,
                execution:  Some(ExecutionId::new(0)),
                parent:     None,
            },
            subject:     Some(Subject {
                node:       NodeRef {
                    id:   NodeId::new(1),
                    name: format!("n{firing}").into(),
                    kind: "attractor/command".into(),
                    meta: serde_json::Value::Null,
                },
                firing:     Some(FiringId::new(firing)),
                visit:      Some(1),
                attempt:    Some(Attempt::FIRST),
                generation: None,
                branch:     BranchRole::None,
            }),
            observed_at: None,
            recorded_at: at,
            record:      Some(Record::Engine(StoredEngineRecord {
                seq,
                origin: EventOrigin::External,
                recorded_at: at,
                body,
            })),
            derived:     None,
        }
    }

    fn finished(seq: u64, firing: u64, at: u64) -> RunEvent {
        engine_event(seq, firing, at, Event::StepFinished {
            firing:  FiringId::new(firing),
            attempt: Attempt::FIRST,
            outcome: Outcome::new(Status::Success, serde_json::Value::Null),
        })
    }

    fn routing(seq: u64, firing: u64, at: u64) -> RunEvent {
        engine_event(seq, firing, at, Event::RoutingResolved {
            decision_id: DecisionId::route(FiringId::new(firing), Attempt::FIRST),
            groups:      Vec::new(),
        })
    }

    fn applied(seq: u64, firing: u64, at: u64) -> RunEvent {
        engine_event(seq, firing, at, Event::RouteApplied {
            applied: RouteApplied::None {
                firing: FiringId::new(firing),
                group:  0,
            },
        })
    }

    fn started(seq: u64, firing: u64, at: u64) -> RunEvent {
        engine_event(seq, firing, at, Event::StepStarted {
            firing:  FiringId::new(firing),
            attempt: Attempt::FIRST,
        })
    }

    fn checkpoint(seq: u64, firing: u64, at: u64) -> StoredPlatformRecord {
        StoredPlatformRecord {
            seq,
            recorded_at: at,
            record: PlatformRecord::Checkpoint(CheckpointRecord {
                execution: 0,
                firing,
                attempt: Some(1),
                workspace: None,
                git_commit_sha: Some("abc".to_string()),
                diff_summary: None,
                patch_blob: None,
                operation: None,
            }),
            position: Some(StagePosition {
                execution: 0,
                firing,
            }),
        }
    }

    fn names(items: &[Item<'_>]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Petri(event) => stream::event_id_text(&event.id),
                Item::Platform(record) => format!("platform {}", record.seq),
            })
            .collect()
    }

    /// A firing's events, then a later firing's events, then a checkpoint
    /// for the first firing stamped later than all of them: the stream puts
    /// the checkpoint right after the first firing's finish, before its
    /// routes and before the later firing.
    #[test]
    fn a_positioned_record_follows_its_firings_finish_whatever_its_clock_says() {
        let events = vec![
            finished(10, 1, 100),
            routing(11, 1, 101),
            applied(12, 1, 102),
            started(13, 2, 103),
            finished(14, 2, 104),
        ];
        let records = vec![checkpoint(1, 1, 250)];
        let (items, held) = order_items(&events, &records, &BTreeSet::new(), false);
        assert_eq!(held, 0);
        assert_eq!(names(&items), vec![
            "execution 0/10/0",
            "platform 1",
            "execution 0/11/0",
            "execution 0/12/0",
            "execution 0/13/0",
            "execution 0/14/0",
        ]);
    }

    /// With the firing finished in an earlier pass, the record goes before
    /// the first event of a later firing; a record with no position keeps
    /// its clock order.
    #[test]
    fn a_positioned_record_precedes_later_firings_and_an_unpositioned_one_keeps_its_clock() {
        let events = vec![started(13, 2, 103), finished(14, 2, 104)];
        let records = vec![checkpoint(1, 1, 250)];
        let finished_before: BTreeSet<FiringKey> = [FiringKey::new(0, 1)].into_iter().collect();
        let (items, held) = order_items(&events, &records, &finished_before, false);
        assert_eq!(held, 0);
        assert_eq!(names(&items), vec![
            "platform 1",
            "execution 0/13/0",
            "execution 0/14/0",
        ]);

        let unpositioned = StoredPlatformRecord {
            position: None,
            ..checkpoint(2, 1, 250)
        };
        let unpositioned = [unpositioned];
        let (items, held) = order_items(&events, &unpositioned, &BTreeSet::new(), false);
        assert_eq!(held, 0);
        assert_eq!(names(&items), vec![
            "execution 0/13/0",
            "execution 0/14/0",
            "platform 2",
        ]);
    }

    /// A record whose firing has no finish yet, in the stream or in the pass,
    /// is held back with everything after it until the finish arrives, or
    /// until the run has finished.
    #[test]
    fn a_positioned_record_is_held_until_its_firings_finish_is_in_the_stream() {
        let events = vec![started(13, 2, 103), finished(14, 2, 104)];
        let records = vec![
            checkpoint(1, 2, 50),
            checkpoint(2, 3, 60),
            checkpoint(3, 2, 70),
        ];
        let (items, held) = order_items(&events, &records, &BTreeSet::new(), false);
        assert_eq!(
            held, 2,
            "the record for firing 3 holds itself and the one after it"
        );
        assert_eq!(names(&items), vec![
            "execution 0/13/0",
            "execution 0/14/0",
            "platform 1",
        ]);

        // Once the run finished, a record for a firing that never finished
        // keeps its clock order; the firing's own records still follow its
        // finish, in their seq order.
        let (items, held) = order_items(&events, &records, &BTreeSet::new(), true);
        assert_eq!(held, 0, "nothing is held once the run finished");
        assert_eq!(names(&items), vec![
            "platform 2",
            "execution 0/13/0",
            "execution 0/14/0",
            "platform 1",
            "platform 3",
        ]);
    }
}
