The one permitted `fabro_ask` call errored ("run sandbox is unavailable for Ask Fabro: Failed to reconnect Docker sandbox"), so I route failure without retry, write no review file, and leave run `01M0N5AVYXR7K8PPMGCQRZF90X` unrevised for a later pass once the sandbox recovers.

{
  "outcome": "failed",
  "failure_reason": "fabro_ask errored for run 01M0N5AVYXR7K8PPMGCQRZF90X: 'run sandbox is unavailable for Ask Fabro: Failed to reconnect Docker sandbox'. Per hard rules (one ask per pass, no retry), routing failure; no review file written.",
  "suggested_next_ids": [],
  "context_updates": {
    "revision_findings": [],
    "journal": {
      "painpoints": [
        {"text": "fabro_ask failed for run 01M0N5AVYXR7K8PPMGCQRZF90X — 'run sandbox is unavailable for Ask Fabro: Failed to reconnect Docker sandbox'. The single allowed ask was consumed by infra failure, not analysis. Fix idea: restore the Docker run sandbox (or add a fallback that serves the Ask-Fabro analyst from persisted stage transcripts) so a pass is never burned on an unavailable sandbox."}
      ],
      "observations": ["No review file exists under `.fabro/reviews/develop/` for this pass; run 01M0N5AVYXR7K8PPMGCQRZF90X remains unrevised and should be re-picked once the run sandbox is back."]
    }
  }
}