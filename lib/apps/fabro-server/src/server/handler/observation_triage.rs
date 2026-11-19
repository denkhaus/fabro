//! The observation triage read model's shared half (work#1385): the
//! linked-item resolution the feed pages and the write-door
//! re-derivation both ride, the derivation itself, and the hook every
//! write door calls to materialize the derived state beside the record.
//!
//! Two databases, one server: the observation RECORDS (and the
//! materialized triage state beside them) live in the summaries/records
//! store, while the LINKS and DECISIONS the derivation reads live in
//! the work store. The re-derive hook therefore lives HERE — the
//! handler layer holds both stores — never inside fabro-work. Every
//! write door that can change a record's derived state calls
//! [`rederive_and_materialize`] once per write: the record append, the
//! link and unlink doors, the create door's filing links, and the
//! manage door's decision writes.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_api::types::{GlobalObservationTriageState, ObservationWorkItemLinkItemState};
use fabro_store::platform_records::ObservationTriageState;
use fabro_types::RunId;
use futures_util::future::join_all;

use super::super::AppState;

/// The wire name of the `no_filing` decision — the one value the
/// derivation's linkless branch keys on; a const so the wire vocabulary
/// has one owner beside the OpenAPI enum.
pub(super) const NO_FILING_DECISION: &str = "no_filing";

/// The bounded concurrency of one record's tracker resolution joins
/// (work#1379): a page with hundreds of distinct linked items must not
/// fire unbounded concurrent tracker reads — the secondary-rate-limit
/// generator the #570 stale-degradation backoff then pays for.
pub(super) const RESOLVE_CONCURRENCY: usize = 16;

/// The derivation (work#1385): one observation record's triage state
/// from its linked items' resolved live states and its decision row.
/// Link presence beats the decision row (ADR-0044): a filing overrides
/// wherever both exist. Linkless, a `no_filing` decision reads
/// `reviewed` — the triager judged and no item was warranted — and
/// everything else reads `untriaged`. An unresolved item (the tracker
/// answered not-found, or the read failed) counts as not-closed: the
/// record keeps `filed` rather than guessing a terminal state.
pub(super) fn derive_triage(
    linked_items: &[Option<LinkedItemState>],
    decision: Option<&str>,
) -> ObservationTriageState {
    if linked_items.is_empty() {
        return if decision == Some(NO_FILING_DECISION) {
            ObservationTriageState::Reviewed
        } else {
            ObservationTriageState::Untriaged
        };
    }
    if linked_items.contains(&Some(LinkedItemState::InProgress)) {
        return ObservationTriageState::InProgress;
    }
    if linked_items
        .iter()
        .any(|state| *state != Some(LinkedItemState::Closed))
    {
        return ObservationTriageState::Filed;
    }
    ObservationTriageState::Closed
}

/// The wire triage state for a derived one (the API type stays the
/// generated enum — no parallel DTO, no bridge beyond this match).
pub(super) fn wire_triage_state(triage: ObservationTriageState) -> GlobalObservationTriageState {
    match triage {
        ObservationTriageState::Untriaged => GlobalObservationTriageState::Untriaged,
        ObservationTriageState::Filed => GlobalObservationTriageState::Filed,
        ObservationTriageState::InProgress => GlobalObservationTriageState::InProgress,
        ObservationTriageState::Closed => GlobalObservationTriageState::Closed,
        ObservationTriageState::Reviewed => GlobalObservationTriageState::Reviewed,
    }
}

/// The live item state behind one wire link, as the observations column
/// shows it: open, open with a live claim (a run is working it), closed,
/// or merged (denkhaus/work#1298: the closure rode a merged PR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkedItemState {
    Open,
    InProgress,
    Merged,
    Closed,
}

impl LinkedItemState {
    pub(super) fn into_wire(self) -> ObservationWorkItemLinkItemState {
        match self {
            Self::Open => ObservationWorkItemLinkItemState::Open,
            Self::InProgress => ObservationWorkItemLinkItemState::InProgress,
            Self::Merged => ObservationWorkItemLinkItemState::Merged,
            Self::Closed => ObservationWorkItemLinkItemState::Closed,
        }
    }
}

/// Resolve one linked item's state. The claim ledger outranks the
/// tracker read: a live claim means the item is open and being worked,
/// whatever the provider's indexing lag says.
///
/// The answer says WHY a state is missing (denkhaus/work#1364):
/// [`LinkedItemResolution::NotFound`] is the tracker's verdict that the
/// item is gone, [`LinkedItemResolution::Stale`] is a degradation — the
/// tracker could not be asked (project gone, credentials) or failed the
/// read (installation budget exhausted even after the client's
/// rate-limit retries). The feed surfaces stale as
/// `item_state_stale` instead of silently null.
#[derive(Clone, Debug)]
pub(super) enum LinkedItemResolution {
    /// The tracker answered with the item's live state and URL.
    Resolved(ResolvedLinkedItem),

    /// The tracker answered: no such item — a verdict, not a failure.
    NotFound,

    /// The read FAILED (rate limit, transport, configuration) — the
    /// null state is a degradation, not a verdict.
    Stale,
}

impl LinkedItemResolution {
    /// The state half of the resolution as the derivation reads it:
    /// an unresolved item (not-found, stale) contributes `None`, the
    /// wire's honest null.
    pub(super) fn state(&self) -> Option<LinkedItemState> {
        match self {
            Self::Resolved(item) => Some(item.state),
            Self::NotFound | Self::Stale => None,
        }
    }
}

/// A linked work item's live state and tracker URL, resolved together in one
/// tracker round-trip (denkhaus/work#1235: the client never builds forge
/// URLs — the work item is an abstract interface).
#[derive(Clone, Debug)]
pub(super) struct ResolvedLinkedItem {
    pub state: LinkedItemState,
    pub url:   String,
}

pub(super) async fn resolve_linked_item(
    state: &AppState,
    project_id: &str,
    item_key: &str,
) -> LinkedItemResolution {
    let Some(project) = state.work_store().project(project_id).await.ok().flatten() else {
        return LinkedItemResolution::Stale;
    };
    let Ok(tracker) = state.work_tracker_for_project(&project).await else {
        return LinkedItemResolution::Stale;
    };
    let item = match tracker.get(&project, item_key).await {
        Ok(Some(item)) => item,
        // The tracker answered: the item is gone — a verdict.
        Ok(None) => return LinkedItemResolution::NotFound,
        // The read failed (rate-limit budget exhausted, transport) — a
        // degradation the feed must mark, not hide (denkhaus/work#1364).
        Err(_) => return LinkedItemResolution::Stale,
    };
    let state = if matches!(
        state
            .work_store()
            .active_claim_for_item(project_id, item_key)
            .await,
        Ok(Some(_))
    ) {
        LinkedItemState::InProgress
    } else {
        match item.state.as_str() {
            // The tracker's closed verdict colors `merged` when the
            // item's closure rode a merged PR (denkhaus/work#1298): its
            // latest landed claim carries the pull-request URL. One
            // memoized read per distinct item — the resolver's cache
            // dedupes. A failed ledger read degrades to the tracker's
            // verdict — plain closed, not a stale column: the tracker
            // answered; only the merged refinement is lost.
            "closed" => {
                if matches!(
                    state
                        .work_store()
                        .landed_claim_with_pr_for_item(project_id, item_key)
                        .await,
                    Ok(Some(_))
                ) {
                    LinkedItemState::Merged
                } else {
                    LinkedItemState::Closed
                }
            }
            _ => LinkedItemState::Open,
        }
    };
    LinkedItemResolution::Resolved(ResolvedLinkedItem {
        state,
        url: item.url,
    })
}

/// The per-request batch of linked item states (denkhaus/work#1362):
/// each distinct (project, item) resolves EXACTLY ONCE per request —
/// the tracker read and the claim-ledger read behind
/// [`resolve_linked_item`] are memoized here, so a page of records
/// linking the same item costs one round-trip, not one per record per
/// link. One resolver lives for one HTTP request (or one write-door
/// derivation) and is dropped with it: no cross-request state, no
/// invalidation, no stale columns.
pub(super) struct LinkedItemResolver<'a> {
    state:               &'a AppState,
    // The memoized resolutions, keyed by distinct (project, item): the
    // page join probes and fills it directly (the module pair shares
    // one visibility scope).
    pub(super) resolved: HashMap<(String, String), LinkedItemResolution>,
}

impl<'a> LinkedItemResolver<'a> {
    pub(super) fn new(state: &'a AppState) -> Self {
        Self {
            state,
            resolved: HashMap::new(),
        }
    }

    pub(super) async fn resolve(
        &mut self,
        project_id: &str,
        item_key: &str,
    ) -> LinkedItemResolution {
        let cache_key = (project_id.to_owned(), item_key.to_owned());
        if let Some(hit) = self.resolved.get(&cache_key) {
            return hit.clone();
        }
        let resolved = resolve_linked_item(self.state, project_id, item_key).await;
        self.resolved.insert(cache_key, resolved.clone());
        resolved
    }
}

/// The write doors' re-derive hook (work#1385): read the record's links
/// and decision from the work store, resolve the linked items' live
/// states (memoized per distinct item), derive the triage state, and
/// materialize it beside the record. Every door calls this once per
/// write; the record append's door additionally guarantees an initial
/// `untriaged` row transactionally inside the store itself, so a
/// triage-filtered feed can never miss a stored record.
///
/// Fail-open by design: a failed re-derivation logs and leaves the
/// previous materialized state in place — the write itself succeeded,
/// and the corner (the state stays materialized until the next write)
/// is the documented contract, not a hidden drift. A link written for a
/// sequence that has no record yet materializes its state too — the row
/// is inert until the record exists, and a later append keeps it
/// (`DO NOTHING`), so the pre-linked sequence reads truthfully.
pub(super) async fn rederive_and_materialize(state: &Arc<AppState>, run_id: &RunId, seq: u64) {
    if let Err(error) = try_rederive_and_materialize(state, run_id, seq).await {
        tracing::warn!(
            error = ?error,
            run_id = %run_id,
            seq,
            "observation triage re-derivation failed; the record keeps its materialized \
             state until the next write touches it"
        );
    }
}

async fn try_rederive_and_materialize(
    state: &Arc<AppState>,
    run_id: &RunId,
    seq: u64,
) -> anyhow::Result<()> {
    let work = state.work_store();
    let links: Vec<fabro_work::ObservationLink> = work
        .linked_items_for_run(run_id)
        .await?
        .into_iter()
        .filter(|link| link.seq == seq)
        .collect();
    let decision = work
        .decisions_for_run(run_id)
        .await?
        .into_iter()
        .find(|decision| decision.seq == seq)
        .map(|decision| decision.decision);
    let derived = if links.is_empty() {
        // Linkless records derive from the decision row alone — no
        // tracker I/O (the presence short-circuit, work#1379, holds at
        // write time too).
        derive_triage(&[], decision.as_deref())
    } else {
        // The distinct linked items resolve once each, in bounded
        // concurrent joins; the resolver memoizes across the record.
        let mut pending: Vec<(String, String)> = Vec::new();
        for link in &links {
            let key = (link.project_id.clone(), link.item_key.clone());
            if !pending.contains(&key) {
                pending.push(key);
            }
        }
        let mut resolutions = Vec::with_capacity(pending.len());
        for chunk in pending.chunks(RESOLVE_CONCURRENCY) {
            let part =
                join_all(chunk.iter().map(|(project_id, item_key)| {
                    resolve_linked_item(state, project_id, item_key)
                }))
                .await;
            resolutions.extend(part);
        }
        let states: Vec<Option<LinkedItemState>> = resolutions
            .iter()
            .map(LinkedItemResolution::state)
            .collect();
        derive_triage(&states, decision.as_deref())
    };
    state
        .stores
        .run_summaries
        .platform_records()
        .materialize_observation_triage(run_id, seq, derived)
        .await?;
    tracing::debug!(
        run_id = %run_id,
        seq,
        triage_state = %derived,
        "observation triage state materialized"
    );
    Ok(())
}

#[cfg(test)]
mod link_resolution_tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};

    use super::{LinkedItemResolution, LinkedItemState};
    use crate::server::AppState;
    use crate::test_support::TestAppStateBuilder;

    /// A tracker whose `get` answers one scripted item — the seam the
    /// observation tables' link resolution reads through
    /// (denkhaus/work#1235: state and URL resolve together, one
    /// round-trip). `get_fails` scripts the degraded read of
    /// denkhaus/work#1364: the installation budget is exhausted and the
    /// client's retries are spent.
    struct UrlScriptedTracker {
        item:      Option<WorkItem>,
        get_fails: bool,
    }

    #[async_trait]
    impl TrackerProvider for UrlScriptedTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            _filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(Vec::new())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            if self.get_fails {
                anyhow::bail!("API rate limit exceeded for installation ID 161242007");
            }
            Ok(self.item.clone())
        }

        async fn create(
            &self,
            _project: &WorkProject,
            _draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn update(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn close(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }
    }

    fn scripted_item(state: &str, url: &str) -> WorkItem {
        WorkItem {
            item_key:   "denkhaus/work#12".to_string(),
            title:      "Scripted".to_string(),
            body:       String::new(),
            state:      state.to_string(),
            labels:     Vec::new(),
            priority:   2,
            url:        url.to_string(),
            updated_at: None,
            created_at: None,
            closed_at:  None,
        }
    }

    async fn state_with(tracker: UrlScriptedTracker) -> Arc<AppState> {
        let state = TestAppStateBuilder::new()
            .work_tracker(Arc::new(tracker) as Arc<dyn TrackerProvider>)
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        store
            .upsert_project(&WorkProject {
                project_id:               "fabro".to_string(),
                display_name:             "Fabro".to_string(),
                tracker_provider:         "github".to_string(),
                tracker_repo:             "denkhaus/work".to_string(),
                target_repo:              None,
                project_label:            "project:fabro".to_string(),
                lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                    .into_iter()
                    .collect(),
                claim_label_prefix:       "fabro:run:".to_string(),
                claim_comment_enabled:    false,
                backpressure_open_pr_cap: 0,
            })
            .await
            .expect("project upserts");
        state
    }

    #[tokio::test]
    async fn a_resolved_link_carries_its_state_and_tracker_url_together() {
        let state = state_with(UrlScriptedTracker {
            item:      Some(scripted_item(
                "open",
                "https://tracker.example/denkhaus/work/issues/12",
            )),
            get_fails: false,
        })
        .await;

        let resolved = super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await;

        let LinkedItemResolution::Resolved(resolved) = resolved else {
            panic!("the scripted item resolves: {resolved:?}");
        };
        assert!(
            matches!(resolved.state, LinkedItemState::Open),
            "the live state rides the resolution"
        );
        assert_eq!(
            resolved.url, "https://tracker.example/denkhaus/work/issues/12",
            "the tracker URL rides the SAME round-trip — clients never build forge URLs"
        );
    }

    #[tokio::test]
    async fn a_closed_item_resolves_closed_with_its_url() {
        let state = state_with(UrlScriptedTracker {
            item:      Some(scripted_item(
                "closed",
                "https://tracker.example/denkhaus/work/issues/12",
            )),
            get_fails: false,
        })
        .await;

        let resolved = super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await;

        let LinkedItemResolution::Resolved(resolved) = resolved else {
            panic!("the scripted item resolves: {resolved:?}");
        };
        assert!(matches!(resolved.state, LinkedItemState::Closed));
        assert_eq!(
            resolved.url,
            "https://tracker.example/denkhaus/work/issues/12"
        );
    }

    #[tokio::test]
    async fn an_absent_item_is_the_trackers_verdict_not_a_stale_read() {
        let state = state_with(UrlScriptedTracker {
            item:      None,
            get_fails: false,
        })
        .await;

        assert!(
            matches!(
                super::resolve_linked_item(&state, "fabro", "denkhaus/work#13").await,
                LinkedItemResolution::NotFound
            ),
            "the tracker answered — honest nulls, no staleness"
        );
    }

    #[tokio::test]
    async fn a_failed_tracker_read_is_stale_not_silently_null() {
        let state = state_with(UrlScriptedTracker {
            item:      None,
            get_fails: true,
        })
        .await;

        assert!(
            matches!(
                super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await,
                LinkedItemResolution::Stale
            ),
            "the failed read is the explicit degradation denkhaus/work#1364 names"
        );
    }

    /// The derivation's truth table (work#1385): the linkless arms key
    /// on the decision row, the linked arms rank in_progress over
    /// filed over closed, and an unresolved item keeps the record
    /// `filed` rather than guessing a terminal state.
    #[test]
    fn the_derivation_ranks_links_over_decisions_and_states_over_presence() {
        use super::derive_triage;

        assert_eq!(
            derive_triage(&[], None),
            fabro_store::platform_records::ObservationTriageState::Untriaged,
            "a linkless record without a decision reads untriaged"
        );
        assert_eq!(
            derive_triage(&[], Some(super::NO_FILING_DECISION)),
            fabro_store::platform_records::ObservationTriageState::Reviewed,
            "a linkless no_filing decision reads reviewed"
        );
        assert_eq!(
            derive_triage(
                &[Some(LinkedItemState::Open)],
                Some(super::NO_FILING_DECISION)
            ),
            fabro_store::platform_records::ObservationTriageState::Filed,
            "link rows beat the decision row — a filing overrides (ADR-0044)"
        );
        assert_eq!(
            derive_triage(
                &[
                    Some(LinkedItemState::Open),
                    Some(LinkedItemState::InProgress)
                ],
                None
            ),
            fabro_store::platform_records::ObservationTriageState::InProgress,
            "a live claim outranks the plain open state"
        );
        assert_eq!(
            derive_triage(&[Some(LinkedItemState::Open)], None),
            fabro_store::platform_records::ObservationTriageState::Filed
        );
        assert_eq!(
            derive_triage(
                &[Some(LinkedItemState::Closed), Some(LinkedItemState::Merged)],
                None
            ),
            fabro_store::platform_records::ObservationTriageState::Filed,
            "a merged item is not a plain close — the record keeps filed"
        );
        assert_eq!(
            derive_triage(&[Some(LinkedItemState::Closed)], None),
            fabro_store::platform_records::ObservationTriageState::Closed
        );
        assert_eq!(
            derive_triage(&[None], None),
            fabro_store::platform_records::ObservationTriageState::Filed,
            "an unresolved item counts as not-closed, never a guessed terminal state"
        );
    }
}
