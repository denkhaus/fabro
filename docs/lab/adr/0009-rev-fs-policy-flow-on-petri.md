# ADR-0009 rev: FS-Policy-Flow auf Petri — ENTSCHIEDEN 2026-09-21

## BESCHLUSS (User-Abnahme „alles einverstanden")
Fablo-seitiger Hook beim Workspace-/Session-Bau: die x.fs_write/x.fs_hide-
Werte des feuernden Knotens werden aus der Admission gelesen und
parametrisieren die Pebble-Exec-Policy über unsere fabro-pebble-sandbox-
Crate. Implementierung: fabro-aa5f. context_read (e804) bleibt zurück,
bis das Preamble-Budget-Offer an Petri angenommen ist.

## UMSETZUNG 2026-09-21 (fabro-aa5f)

Am gepinnten Petri-Rev (9d51715) existiert KEIN host-seitiger Seam an der
Session-Bau-Stelle: Petri baut die Run-Sessions selbst (NativeSession::open
ueber den Scope-ExecEnv), HostTools liefert nur Tools, und die
HookServiceHandle-Capability duldet keine Interposition (Duplikat = Panic;
vorab installieren verdraengt Petris lokalen [[run.hooks]]-Dienst).
Upstream-Angebote entfallen per Nutzerdirektive 2026-09-21 (Upstream
ignoriert unsere Contributions); das Preamble-Budget-Offer oben ist damit
gegenstandslos.

Gelandete Decke deshalb:
- Werte-Quelle: graph_source (x.* ist lowering-gefallen), nie die Admission.
- Lints beim Check: Glob-Gueltigkeit (Fehler, refused), fs_write-unter-
  fs_hide- und preamble-Budget-Konsistenz (Warnungen) — der fabro-1392-Anteil.
- Write-Enforcement am Checkpoint: FabroHooks verweigert den Commit einer
  Stufe, deren staged Dateien ausserhalb x.fs_write liegen (Klasse
  `fs_policy_violation`, Run endet, nichts landet im Run-Branch). FsScope
  getragen in fabro-pebble-sandbox (fs_scope.rs, aus fabro-ba96).
- fs_hide (Read-Seite) und Tool-level-Deny sind am Rev nicht erzwingbar;
  context_read (e804) bleibt zurueck. Aktivierung der Guard-Semantik mit
  dem Petri-Deploy (W5); das Staging-Szenario (fabro-96c6-Rest) muss
  beweisen, dass die Envelope-nodes der Linie (read-only authored) ohne
  Verletzung laufen.
