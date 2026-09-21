# ADR-0010 rev: Exit-Kinds auf Petri — ENTSCIEDEN 2026-09-21 (Option A)

## Ausgangslage
- Fork: exit_kind-Attribut (boundary/deadlock/soft) klassifiziert
  Terminal-Events (FailureReason::Deadlock/SoftStop, SuccessReason::
  Boundary — Typen petri-seitig schon gelandet via W1-1).
- Petri: x.kind-Kanten (inert); natives Vokabular: on_failure,
  on_retries_exhausted, allow_partial, goal_gate + Firing-Budgets
  (max_visits) + partial_success (is_success_like).

## Optionen
A. **Tier-Mapping im Fold**: der fork_taxonomy-Seam liest die
   x.kind-Werte aus der Admission und klassifiziert das Terminal
   (Boundary → Succeeded{Boundary}, deadlock → Failed{Deadlock},
   soft → Failed{SoftStop}). Pro: Graph-Assets bleiben wie sie sind
   (x.kind schon migriert), Klassifikation zentral, W1-3-dockt an.
   Contra: die Kanten-ROUTING-Semantik (Zyklus-Guard-Erkennung) braucht
   max_visits/goal_gate im Graph — Guards werden Conditions (fa0a-Muster).
B. **Native Failure-Policies**: on_failure/on_retries_exhausted + goal
   gates übernehmen Guard-Erkennung UND Klassifikation; x.kind fällt weg.
   Pro: keine Fork-Schicht. Contra: Graph-Umbau tiefer, PartialSuccess
   ≠ Boundary-Semantik (Ziel-nicht-erreicht vs. some-nodes-failed).

## Empfehlung zur Diskussion
A — das Terminal-Klassifikations-Problem (was SOLL der Run reporten)
ist orthogonal zum Guard-Problem (wann bricht der Zyklus ab): A löst
Klassifikation mit vorhandener Fork-Schicht, Guards nativ per
max_visits + Conditions (fa0a-Muster bereits entschieden).


## BESCHLUSS (2026-09-21, User-Abnahme „alles einverstanden")
Option A: x.kind-Werte aus der Admission im Fold klassifizieren
(Boundary → Succeeded{Boundary}, deadlock → Failed{Deadlock},
soft → Failed{SoftStop}); Guards nativ per max_visits + Context-
Conditions (fa0a-Muster). Implementierung: fabro-288d.