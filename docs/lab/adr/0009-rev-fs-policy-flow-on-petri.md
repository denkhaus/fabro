# ADR-0009 rev: FS-Policy-Flow auf Petri — ENTSCHIEDEN 2026-09-21

## BESCHLUSS (User-Abnahme „alles einverstanden")
Fablo-seitiger Hook beim Workspace-/Session-Bau: die x.fs_write/x.fs_hide-
Werte des feuernden Knotens werden aus der Admission gelesen und
parametrisieren die Pebble-Exec-Policy über unsere fabro-pebble-sandbox-
Crate. Implementierung: fabro-aa5f. context_read (e804) bleibt zurück,
bis das Preamble-Budget-Offer an Petri angenommen ist.
