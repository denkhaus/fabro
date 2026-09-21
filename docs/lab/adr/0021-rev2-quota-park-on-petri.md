# ADR-0021 rev 2: Quota-Park auf Petri — ENTSCIEDEN 2026-09-21 (Option C)

## Ausgangslage
- Fork-Stand (ADR-0021 + rev): Engine-seitige Park-Klassifikation
  (SoftStop + TransientInfra + rate_limit-Signatur → Blocked{QuotaRateLimit}),
  Server-Gate vor jedem Cron-Fire (10-min-Recheck), Breaker-Exemption.
- Petri-Lage: kein Quota-Begriff; BlockedReason nur HumanInputRequired +
  unsere additive QuotaRateLimit. NATIVES Tier-Vokabular gefunden:
  on_failure, on_retries_exhausted, allow_partial, retry_target,
  fallback_retry_target + Wait-Steps + RetryPolicy/Backoff.
- Server-Gate + Breaker (W1-2) sind bereits petri-geported und bleiben
  erster Verteidiger.

## Optionen
A. **Graph-Level-Tier**: Agent-Stage schlägt mit rate_limit-Klasse fehl →
   on_failure-Route auf Park-Node (wait bis Window-Reset + Re-Enter über
   retry_target). Pro: engine-idiomatisch, deterministisch, resume-fähig.
   Contra: Park-Dauer muss der Graph kennen (5h-Window) — dynamisch?
   Wait-Step mit fester Dauer vs. condition-gesteuertem Re-Enter.
B. **Fork-Fold-Klassifikation**: die Projection-Fold (unser fork_taxonomy)
   klassifiziert Terminal-Runs mit rate_limit-Signatur als Parks —
   W1-3-Muster (conclusion_failure ist schon da). Pro: wenig Graph-Änderung,
   Gate übernimmt Recovery. Contra: der RUN ist dann terminal — kein
   In-Graph-Wait; Resume = neuer Run (fork_run/rewind nativ!). Prüfen:
   reicht "terminal parken + Gate re-fires" als Semantik?
C. **Hybrid**: B für die Klassifikation + nativer rewind beim Re-Fire
   (Gate feuert → Platform erzeugt Resume-Run vom Park-Checkpoint).

## Empfehlung zur Diskussion
C — nutzt beide Welten: Klassifikation im Fold (bewährt, schon
verankert), Recovery über natives rewind statt Engine-Wait. Kein
Wait-Dauer-Design nötig.


## BESCHLUSS (2026-09-21, User-Abnahme „alles einverstanden")
Option C: Terminal-Klassifikation im Projection-Fold (fabro-e566-Parks:
rate_limit-Signatur → Blocked{QuotaRateLimit}); Recovery über natives
rewind beim Gate-Re-Fire statt Engine-Wait. Implementierung: fabro-2e7b.