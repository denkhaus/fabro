//! The hook-files transport declaration at admission (fabro-091b),
//! pinned fork-only.
//!
//! A `[[run.hooks]]` entry may declare the files it needs
//! (`files = [...]`): the Fabro bundler carries them in the workflow
//! closure and the Fabro server stages them host-side for
//! `sandbox = false` hooks, which run with no workspace. Petri's
//! frontend Entry mirror must ACCEPT the field so admission does not
//! reject workflows that declare it (petri fork b03b0c3) — the engine
//! parses and ignores it; Fabro owns the transport.
//!
//! The lanes' sync smoke pins the DECLARATIONS in the four workflow
//! files, not this seam: a dependency update that drops the frontend
//! acceptance reds here first, instead of every lane failing at its
//! next fire. The bundling and staging halves are pinned in
//! `fabro-manifest` and `fabro-server`'s `fork_hook_assets` tests.

use std::collections::BTreeMap;

use fabro_petri::check::{self, Bundle, CheckRequest, Launch};
use fabro_petri::runtime::RuntimeSpec;

/// The workflow.toml of a host-side hook that declares a transport file:
/// the `files` key is the declaration under test.
const SETTINGS: &str = "_version = 1\n\
                       [workflow]\n\
                       graph = \"workflow.fabro\"\n\
                       \n\
                       [[run.hooks]]\n\
                       name = \"shadow\"\n\
                       event = \"stage_complete\"\n\
                       sandbox = false\n\
                       files = [\"scripts/shadow.nu\"]\n\
                       script = \"true\"\n";

/// A bundle whose `[[run.hooks]]` entry declares a transport file —
/// present in the bundle the way the bundler carries it — must admit:
/// the frontend's Entry accepts the `files` field, so a workflow that
/// uses the host-hook transport is not refused at check.
#[test]
fn a_hook_files_declaration_admits() {
    let request = CheckRequest {
        bundle:             Bundle {
            files:        BTreeMap::from([
                (
                    "workflow.fabro".to_string(),
                    "digraph HookClosure {\n  start [shape=Mdiamond]\n  exit \
                     [shape=Msquare]\n  start -> exit\n}\n"
                        .to_string(),
                ),
                ("workflow.toml".to_string(), SETTINGS.to_string()),
                (
                    "scripts/shadow.nu".to_string(),
                    "def main [] { 0 }\n".to_string(),
                ),
            ]),
            entrypoint:   "workflow.fabro".to_string(),
            project_toml: None,
        },
        inputs:             BTreeMap::new(),
        vars:               BTreeMap::new(),
        launch:             Launch::default(),
        runtime:            RuntimeSpec::default(),
        unbound_is_warning: false,
    };
    let admitted = check::check(&request).expect("a hook-files declaration must admit");
    let _ = (admitted.graph, admitted.children);
}
