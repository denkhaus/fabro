# Petri-Integration: Analyse & Migrationsplan (2026-09-20)

Status: beschlossen (Option B). Epic: **fabro-9930**, Kinder W0–W5.
Basis: `denkhaus-petri` = `upstream/main` @ `40419cbd2` (167 Commits über
Merge-Base `4b1f440b6`). `denkhaus` bleibt deploybar, Linie läuft weiter
(Merge-Leg seit 2026-09-13 deaktiviert). Run-History wird NICHT archiviert
(`run_events` wird bei Cutover gedroppt, `runs`-Summaries überleben).

## Warum kein Merge auf denkhaus

| Kennzahl | Wert |
|---|---|
| Upstream-Commits | 167 |
| Fork-Commits über Merge-Base | 1259 |
| upstream-gelöscht UND fork-modifiziert | 84 (Delete/Modify) |
| Shared-Files beide modifiziert | 180 |
| Fork-only unter lib/ | 380 (inkl. ganzer fabro-core) |

Upstream löschte: fabro-core, fabro-sandbox, fabro-hooks, fabro-validate,
fabro-acp; fabro-workflow 131 → 17 src-Dateien. Jeder Run ist ein Petri-Run
(kein Flag). Die Engine unserer Features existiert nicht mehr — ein Merge
wäre eine Re-Architektur in der Konfliktbehandlung.

## Petri-Kernkonzepte (Schnellreferenz)

- Sans-IO-Kern: `apply(state, event) -> (state, commands)`; deterministisch,
  replay-geprüft. Frontends: Attractor (.fabro-DOT) + GitHub Actions → IR.
- Records statt Event-Log: `petri_records` + `platform_records` (SQLite),
  Projection-Fold → View-Tabellen + Run-Stream (`stream_seq`).
  Migration 2026091803 droppt `run_events` („greenfield").
- Sandboxes: Petri besitzt sie (sandbox-driver, Lease-Ledger, Retention,
  prune); Server attached nur (fabro-pebble-sandbox = Driver-Handle als
  Pebble-Environment).
- Checkpoints nativ: fork/rewind/retry/timeline (CLI + API + Worker-Recovery).
- Routing AND-of-XOR; Failure-Tiers, RetryPolicy/Backoff, Firing-Budgets,
  Goal-Gates, Wait-Steps, Manager-Step (reattach), Run-Context (kv.*),
  Host-Tool-Capability für Fabro-Run-Tools.

## Feature-Migrationsmatrix

Score = Wiederherstellbarkeit: 🟢 8–10 leicht (Seam überlebt) ·
🟡 4–7 substanzieller Port · 🔴 1–3 Redesign auf Petri-Konzepten ·
⚫ O superseded. Seed = Kind unter fabro-9930.

| Feature | Score | Kind-Seed | Kern des Ports |
|---|---|---|---|
| Dev-Loop-Assets (justfile/scripts/mise/gate) | 🟢 10 | fabro-fef5 (W0) | Übernahme + Build-Grün + Inventur |
| Stall-Budget (0e11, stall_timeout) | 🟢 9 | fabro-96c6 (W3-5) | Attribut überlebt; Pin auf fabro-dot |
| PR-create-retry (67e5) | 🟢 8 | fabro-b5a9 (W2-3) | fabro-github unverändert |
| Fork-Catalog-Overlay (cd27) | 🟢 8 | fabro-6945 (W2-4) | Seam an codecs-neue catalog.rs |
| spa_refresh (332e) | 🟢 8 | fabro-d0dd (W4-5) | fork-owned fabro-dev |
| PR-model-plumbing (890b) | 🟢 7 | fabro-b5a9 (W2-3) | → operations/create.rs |
| Duplicate-Child-Guard (8ee1) | 🟢 7 | fabro-a875 (W2-1) | create.rs-Seam überlebt |
| Automations-CLI (fabro auto) | 🟢 7 | fabro-6c16 (W1-1) | Client-Regen + Drift |
| ask-Duplikat (bd6c) / attach-Replay (204e) | 🟢 7 | fabro-d0dd (W4-5) | erst verifizieren (Stream-Rewrite) |
| Run-Tools-Parität (06e0 u.a.) | 🟢 7 | fabro-96c6 (W3-5) | upstream-nativ; inspects-Scoping prüfen |
| fs_hide/fs_write (ba96) | 🟢 7 | fabro-d0dd (W4-5) | Pebble-ToolContext + Exec-Policy |
| Wait-Endpoint (571e) | 🟡 ~6 | fabro-8795 (W4-4) | Parität prüfen, sonst Stream-Cursor |
| Publish-blocked + Boundary (67e5/08b4) | 🟡 6 | fabro-6655 (W1-3) | Projection-Fold + Slack + Web |
| Diff-based Publish-Schutz (4ebd) | 🟡 6 | fabro-2889 (W2-2) | supervisor + platform_records |
| Provider-Gate + Breaker (986b Serverhälfte) | 🟡 6 | fabro-a52f (W1-2) | scheduler-Loop überlebt |
| Availability-Probe (8d30a) | 🟡 5 | fabro-afab (W4-2) | Scope-Records statt Inventory |
| Approval-TTL (54f0) | 🟡 5 | fabro-fdd8 (W4-3) | Interview-Records |
| Lifecycle-Guards (Inspection/TurnScoped) | 🟡 5 | fabro-afab (W4-2) | Driver-Attach-Pfade |
| Web-Re-Ports (resumeRun, Popover, Phases) | 🟡 5 | fabro-71a8 (W4-1) | Petri-Views (subsumiert 3a5e, ea68) |
| capability_gate (ADR-0019) / environment_compat (94f6) / Staleness-Supervisor | 🟡 5 | fabro-fdd8 (W4-3) | neue Agent-/Env-Surfaces |
| Preamble-Budget (a85b) | 🔴 4 | fabro-788b (W3-3) | Attractor-Compaction oder Frontend-Attr |
| seed_cycles (45d0) | 🔴 4 | fabro-fa0a (W3-4) | Run-Context/kv.* |
| Stage-Envelope (ADR-0009: stage_policy, context_read e804) | 🔴 4 | fabro-aa5f (W3-6) | Host-Tool-Capability |
| Exit-Kinds deadlock/soft (b907, ADR-0010) | 🔴 3 | fabro-288d (W3-2) | Tiers + Goal-Gates (ADR-0010 rev) |
| Quota-Park Engine-Hälfte (986b, ADR-0021) | 🔴 3 | fabro-2e7b (W3-1) | Tier-Routing + Wait-Node (ADR-0021 rev 2) |
| Resume-from-Failure (7627, closed) | ⚫ O | fabro-d420 (W4-6) | rewind/retry nativ; Web-Mapping in W4-1 |
| Terminal-Run-Provisioning (8d30b, closed) | ⚫ O | fabro-d420 (W4-6) | Petri-Retention; Fenster beweisen |
| Sandbox-GC (44d8, closed) | ⚫ O | fabro-d420 (W4-6) | lease ledger + prune; Knobs offeren |
| Legacy-Catalog-Fix (b7c4, closed) | ⚫ O | — | Offer-Branch separat; post-cutover irrelevant |
| Presence-Pin-System (Meta) | 🔴 — | fabro-fcb2 (W1-4) | Pins je Port neu (Zwei-Pin-Regel) |

## Attribut-/Asset-Matrix (develop/conductor/merge-upstream)

Überleben im Attractor-Frontend: `stall_timeout`, `max_node_visits`,
`inspects`, `retry_policy`, `output_schema`, `output_retries`,
`reasoning_effort`, `skills="discover"`, `fs_write`, `fabro_tools`,
`preamble_stages_ignore` (verifizieren), workflow.toml-Settings-Layer,
hooks/MCP, Manager-Loops, nested workflows, goal gates.

Sterben (W3-5 fabro-96c6 ersetzt): `exit_kind`-Kanten (→ Tier-Routing),
`cycle_counter_reset_key` + seed_cycles-Reads (→ Run-Context, W3-4),
`preamble_budget_kb` (→ Compaction/Attr, W3-3), Quota-Park-Kanten
(→ Wait-Node, W3-1), `FailureReason::Deadlock` (→ Tiers, W3-2).

## Prod-/Daten-Hinweise (Cutover = fabro-d659)

- Era-Check gegen Prod-Snapshot Pflicht (fabro-eec6, 3. Vorfall):
  `PRAGMA wal_checkpoint(TRUNCATE)` VOR cp; isolierter Container
  (`--network none`, dummy SESSION_SECRET, prod settings.toml); Startup
  muss die petri_records/platform_records-Migrationen sauber durchlaufen.
- Kein Run-History-Archiv (Entscheidung 2026-09-20): Detail-Ansichten
  alter Runs entfallen; `runs`-Summaries überleben die Tabellen-Rebuild-
  Migration.
- Deploy-Fenster: kein Conductor-Pass aktiv; 404 bei `fabro ps` heißt
  „kein Container registriert" — Host via SSH prüfen.
- Toolchain-Env-PUT + host `docker pull` bleiben EIN Schritt (0e9c).
- Bootstrap-Lücke: die Linie kann Port-Seeds erst nach Petri-Deploy
  verarbeiten; W0–W2 agent-seitig, Staging-Instanz früh erwägen (W3-5
  Conductor-Szenario braucht sie).

## Wellen → Seeds

- **W0** fabro-fef5 — Baseline, Assets, Inventur-Reconciliation
- **W1** fabro-6c16 (auto-CLI) · fabro-a52f (Gate/Breaker) ·
  fabro-6655 (Taxonomie-Fold) · fabro-fcb2 (Pin-System v2)
- **W2** fabro-a875 (Dup-Guard) · fabro-2889 (Publish-Schutz) ·
  fabro-b5a9 (PR retry+model) · fabro-6945 (Catalog-Overlay)
- **W3** fabro-2e7b (Quota-Park) · fabro-288d (Exit-Kinds) ·
  fabro-788b (Preamble) · fabro-fa0a (seed_cycles) ·
  fabro-96c6 (Asset-Umbau, nach W3-1..4) · fabro-aa5f (Stage-Envelope)
- **W4** fabro-71a8 (Web) · fabro-afab (Probe+Guards) ·
  fabro-fdd8 (Server-Betrieb) · fabro-8795 (Wait-Endpoint) ·
  fabro-d0dd (kleine CLI) · fabro-d420 (Superseded-Beweise)
- **W5** fabro-d659 — Cutover-Runbook + denkhaus-Archiv

## W0-Inventur-Reconciliation (2026-09-21, abro-fef5)

Mechanisch: 338 fork-modifizierte + 385 fork-only lib-Pfade gegen die
Wellen-Kinder gemappt. Ergebnis:

**3 Lücken → neue Kinder**: fabro-1392 (Validierungs-Regel-Familie →
Petri check), fabro-9b1b (Hooks-Familie → Petri-Hook-Service),
fabro-a044 (Workflow-Transforms → Frontend-Lowering, upstream-Abdeckung
zuerst verifizieren). W3-5 (fabro-96c6) hängt jetzt zusätzlich hinter
fabro-1392 + fabro-9b1b.

**Bewusste Drops (kein Port)**: `fabro parse` (upstream gelöscht),
fabro-acp-Änderungen (Attractor-Steps nativ), Legacy-Run-Event-Reader
(run_event/bound.rs, fork_legacy_read.rs), alte SQLite-Aktivierungs-
Migrationen (Era endet), Root-openapitools.json (upstream: paket-lokal
in fabro-api-client).

**Rebase-Vermerke**: fork-DB-Migrationen 2026090201_automation_overlap_policy
+ 2026090601_automation_schedule_breaker → auf Petri-Migrationskette
(W1-1/W1-2 beachten, W5 Deploy-Reihenfolge).

**Baseline-Ergebnis**: build grün (4m27s); Tests 840/845, die 5 Fehler
waren fehlende dind-Runner-Images (pre-Pull-Flake, upstream-CI-Pattern);
isoliert danach grün. dogfood-gate.yml um Plugin-Install + Image-Pull
erweitert, Branch-Filter auf denkhaus-petri.

**W0-Verifikation (getragener Baum)**: 4187/4193 grün; 6 Fehler allesamt
umgebungsbildet: 3× FABRO_SERVER-Env (unset! siehe unten), 2× fehlendes
CATALOG_IMAGE (ghcr.io/lithoscomputer/ubuntu-22.04:slim, pre-pull), 1×
fabro-dot-Snapshot für eingecheckte Workflows — akzeptiert: Petris
DOT-Parser liest alle 5 Fork-Graphen (develop 12n/27e, conductor 5n/13e,
architect 6n/10e, merge-upstream 4n/7e, revisor 5n/10e inkl. file-refs).
Wichtig für lokale Läufe: IMMER `env -u FABRO_SERVER` (Agent-Shell trägt
FABRO_SERVER=http://127.0.0.1:32276 — macht parse-Tests rot); Docker-
Tests brauchen slim+dind-Runner + CATALOG_IMAGE lokal gepullt.

## W2-4 + W3-hooks Verifikation (2026-09-21)

- Catalog-Overlay (fabro-6945): Seam in beiden Builder-Pfaden der codecs-
  Ära, Pin grün — W2 KOMPLETT (a875, 2889, b5a9, 6945).
- Hooks-Familie (fabro-9b1b): Petris HookVokabular ist ein Superset
  (StageComplete, script/command/url/prompt/agent, blocking, timeout,
  sandbox); die HookEntry-Layer nimmt unsere TOMLs unverändert — die
  [run.hooks]-Sektionen senken sauber (KEIN Hook-Fehler bei validate).
  Ausführungs-Verifikation (stage-journal feuert) läuft mit dem
  W3-5-Conductor-Szenario (Staging), die Engine-Maschine decken die
  fabro-petri-Hook-Tests.
- KORREKTUR (gleiche Session, nach Zweitmessung): die GRAPHE validieren
  NICHT grün (rc=1) — die erste Lesung war falsch. Alle 5 Workflows
  fallen auf `attractor.unknown_attribute`: die Fork-Stage-Envelope-
  Attribute (fs_write/fs_hide, context_allow/consume_keys, preamble_*,
  skills, tools, fabro_tools) sind KEINE Attractor-Attribute — sie waren
  Fork-Engine-Features (ADR-0009). Petris designed escape: der `x.*`-
  Namespace (mitgeführt ohne Lesen). W3-5-Umzug: Attribute → `x.*`
  präfixen, bis W3-6 sie als Host-Tool-Policies aus der Admission liest.
  Betroffene Graph-Attribute: cycle_counter_reset_key, inspects,
  preamble_budget_kb, stall_timeout überlebt (kein Fehler), max_node_visits
  überlebt.
- Drei validate-Warnungen verfeinern die W3-Karte (settings zogen zur
  Plattform): `[run.meta_branch]` (standalone runner macht kein eigenes
  Git), `[run.notifications]` (Plattform-Facility), `fabro_tools = true`
  (Host-Tool-Capability statt Engine-Setting — W3-6-Anker).

## W3: Preamble-Budget-Disposition (fabro-788b, 2026-09-21)

Petri hat die Preamble-Maschinerie GEPORTET (attractor/steps/fidelity.rs
folgt wörtlich dem fork-era fabro-workflow/handler/llm/preamble.rs) — aber
ohne die Fork-Erweiterungen. Zwei Fork-Teile, zwei Wege:

1. **Per-Node-Scoping** (x.preamble_stages_ignore/allow_keys): nativer
   Hebel ist das `fidelity`-Attribut (truncate/compact/summary:low/medium/
   high/full, per Node wählbar). Keine 1:1-Semantik (Ignore-Listen vs.
   Modus-Leiter), aber der äquivalente Kontrollraum. Die x.*-Werte bleiben
   inert dokumentiert; die Linie tuned post-Cutover per fidelity aus
   Run-Evidence (revisor-Arbeit, nicht Migrationsarbeit).
2. **Aggregate-Budget + Demote-Large-Values** (preamble_budget_kb 48KB,
   Blob-Offload-Schwelle): kein Fabro-Seam — die Preamble baut Petri
   pinned. Strategischer Pfad: UPSTREAM-OFFER an petri (Port der Fork-
   Budget-Arbeit in fidelity.rs, INLINE_VALUE_MAX ist der Anknüpfungspunkt:
   8KB-Hardcap vs. unser 48KB-Aggregat + Blob-Detour). Pattern wie der
   legacy-catalog-Offer.

BONUS aus derselben Recherche: die native Attractor-Attributliste nennt
on_failure, on_retries_exhausted, allow_partial, goal_gate, retry_target,
fallback_retry_target — das ist das W3-1/W3-2-Tier-Routing-Vokabular. Die
Grilling-Vorlagen können direkt darauf aufbauen.

## W3: seed_cycles-Disposition (fabro-fa0a, 2026-09-21)

Stärker als erwartet: Petris attractor steps tragen `context_updates`
NATIV (outcome.rs — Agent-Outcomes mergen Key/Value in den Run-Context;
Conditions routen gegen den prospektiven Kontext). Die Fork-Zähler
(seed_cycles/cycle_counter_reset_key) werden:

1. **Workflow-eigene Zähler**: Implementer/Reviewer schreiben
   context_updates (visit_count, current_seed_id); Reset-Semantik im
   Node-Outcome statt engine-Injektion.
2. **Deterministische Guards**: die 3x-Review=>Blocked-Zyklen werden
   Edge-CONDITIONS auf den Kontextwerten — keine Prompt-Zählung mehr,
   besser als der Fork-Stand.
Graph-Rework gehört zu W3-5-Rest (mit den Tier-Entscheidungen).

## W3: Stage-Envelope-Seam-Analyse (fabro-aa5f, 2026-09-21)

Zwei Hälften, zwei Lagen:

1. **fs-Scoping (x.fs_write/x.fs_hide)**: Enforcement lebt in UNSERER
   Crate (fabro-pebble-sandbox: exec-Policy, Pfad-Auflösung). Offene
   Design-Frage: wie fließen die x.*-Werte vom admittierten Graph zur
   per-Node-Pebble-Session? Die Admission (fabro-petri/src/admission.rs)
   ist fabro-seitig lesbar; die Step-Konstruktion läuft petri-seitig.
   Kandidat: fabro-seitiger Hook/Layer, der beim Workspace/Session-Bau
   die x.*-Werte des feuernden Knotens liest und die Pebble-Exec-Policy
   parametrisiert. Braucht eine Design-Runde (ADR-0009 rev).
2. **context_read (e804)**: nativ gibt es KEIN Kontext-Tool — Kontext
   kommt über die Fidelity-Preamble. Solange kein Budget-Enforcement
   existiert (788b-Offer an petri ausstehend), ist ein aktiver Read-Tool
   nachrangig: die Werte sind ohnehin im Preamble. Nachziehen, sobald
   das Offer angenommen ist.

FAZIT W3: alle vier offenen Kinder (2e7b, 288d, aa5f, 96c6-Rest)
konvergieren auf DIE Design-Runde — Tier-Routing (Quota-Park +
Exit-Kinds) und ADR-0009-rev (Policy-Flow) zusammen entscheiden, dann
ist der Rest mechanisch.

## W4-3 Dispositionen (fabro-fdd8, 2026-09-21)

1. **Staleness-Supervisor**: bereits W2-2 geportet (fork_staleness_
   supervisor, 8/8 Wire-Tests).
2. **Approval-TTL (54f0)**: SUPERSEDED — Petris Interview-Gates tragen
   natives Expiry (TimeoutPolicy::HandlerManaged: Frage-Deadlines,
   Gate meldet Expiry, Dispatcher cancelt als `question_expired`).
   Zombie-Approvals können strukturell nicht entstehen; die Engine-Ära-
   Supervisor-Datei wurde nicht übernommen.
3. **environment_compat (94f6)**: ENTFALLEN (strukturell) —
   `unsupported_resource_fields` existiert auf Petri nicht; Env-Rows
   validieren zur Schreibzeit, Provider-Kompetenz lebt im sandbox-driver.
   Bei realen Cutover-Vorfällen: Lint gegen das Driver-Kompetenzmodell.
4. **capability_gate (ADR-0019)**: ENTFALLEN (strukturell) — Petri
   resolved GitHub-Credentials ausschließlich aus Server-Settings;
   Workflow-Registrierung trägt Inhalt, keine Credential-Requests; Env-
   PUTs sind principal-authentifiziert. Der Fork-Gate-Punkt (per-Run-
   Permission-Interpolation) existiert nicht mehr — die Invariante
   („agenten-autore Konfiguration kann sich keine Credentials minten")
   hält strukturell.


## W3-Korrektur 2 (2026-09-21): x.* wird beim Lowering GEDROPPED

Petris x.-Namespace ist Check-akzeptiert aber Lowering-gefallen (attrs.rs:
`starts_with(EXTENSION_PREFIX) -> continue`) — die Werte reaching NICHT
die Admission. Korrektur des W3-5-Mechanismus: der `graph_source`
(originaler DOT-Text) reitet in jeder Run-Spec und trägt die x.*-Werte
verbatim. Exit-Kinds (fabro-288d) lesen ihn direkt im Fold
(fork_exit_kinds.rs, 4 Pins grün); fs-Policy (aa5f) ebenso über den
graph_source beim Session-Bau statt über die Admission.
