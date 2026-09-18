//! Petri compiles: the create handler hands a workflow version's files, the
//! run's inputs and the launch to `Runtime::check`, and gets back either the
//! admitted graphs or Petri's diagnostics.
//!
//! `Runtime::check` reads the workflow and its settings files from disk, so
//! the bundle is materialized into a temporary directory first, laid out the
//! way the Fabro frontend expects: the workflow file with `workflow.toml`
//! beside it under a bundle root that holds a `.fabro` directory (with
//! `.fabro/project.toml` when the caller has one). The directory is removed
//! when the check returns. An in-memory `FileSource` entry point on
//! `Runtime` would remove the round trip; that is a Petri follow-up.
//!
//! The launch binds the compile variables the Fabro frontend reads:
//! `petri.launch_model` and `petri.launch_provider` as the model default
//! below every file layer, and `petri.repository` as the repository the root
//! `start` stage checks out. A caller with no local repository binds `null`,
//! and the run starts from an empty workspace.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use petri_runtime::LoadError;
use petri_runtime::frontend::{
    self, CompileInputs, LAUNCH_MODEL_VAR, LAUNCH_PROVIDER_VAR, REPOSITORY_VAR, Severity,
};
use petri_runtime::ir::Graph;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::RuntimeSpec;

/// The directory under the temporary bundle root the version's files land
/// in. Its parent holds `.fabro`, so the Fabro frontend takes the parent as
/// the bundle root.
const BUNDLE_DIR: &str = "bundle";

/// The project settings file the Fabro frontend reads at the bundle root.
const PROJECT_FILE: &str = ".fabro/project.toml";

/// One workflow bundle to check: its files by bundle-relative path.
#[derive(Clone, Debug, Default)]
pub struct Bundle {
    /// Every file of the version closure, keyed by its path relative to
    /// the bundle (`workflow.fabro`, `workflow.toml`, `prompts/goal.md`,
    /// `children/check.fabro`), with `/` separators.
    pub files:        BTreeMap<String, String>,
    /// The workflow file to check, one of `files`.
    pub entrypoint:   String,
    /// `.fabro/project.toml` at the bundle root, when the caller has one.
    pub project_toml: Option<String>,
}

/// What the launch binds below the file layers.
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub model:      Option<String>,
    pub provider:   Option<String>,
    /// The local repository the root `start` stage checks out into the
    /// workspace; `None` starts the run from an empty workspace.
    pub repository: Option<PathBuf>,
}

/// One check: the bundle, the run's inputs, the launch and the runtime.
#[derive(Clone, Default)]
pub struct CheckRequest {
    pub bundle:  Bundle,
    /// The intent's inputs, under which `[run.inputs]` defaults fill in.
    pub inputs:  BTreeMap<String, Value>,
    pub launch:  Launch,
    pub runtime: RuntimeSpec,
}

/// Petri's diagnostic, in the shape Fabro's create handler maps onto its
/// own: the stable code, the text, the hint, and the position in the
/// bundle when known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    /// Petri's stable code: `attractor.model.unknown`, `fabro.hooks.toml`,
    /// `unsupported.workflow_toml.key`.
    pub code:     String,
    pub message:  String,
    pub hint:     Option<String>,
    /// The bundle-relative file the diagnostic names.
    pub file:     String,
    /// 1-based; `None` for a whole-file problem.
    pub line:     Option<u32>,
    pub column:   Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

/// What Petri admitted: the lowered root graph, the pre-lowered child
/// graphs, and the warnings the lowering raised.
pub struct Admitted {
    pub graph:    Graph,
    pub children: Vec<Graph>,
    pub warnings: Vec<Diagnostic>,
}

/// Why a check produced no graph.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// Petri refused the workflow. Every diagnostic is here, warnings
    /// included; at least one is an error.
    #[error("Petri refused the workflow with {} diagnostic(s)", .0.len())]
    Rejected(Vec<Diagnostic>),
    /// The bundle could not be materialized for the check.
    #[error("could not materialize the workflow bundle at `{path}`")]
    Materialize {
        path:   PathBuf,
        #[source]
        source: io::Error,
    },
    /// The bundle's entrypoint is not one of its files, or no frontend
    /// claims it.
    #[error("the workflow could not be loaded")]
    Load(#[source] LoadError),
}

/// Materialize the bundle, run `Runtime::check`, and hand back the admitted
/// graphs or the diagnostics. Blocking: it reads and writes files and
/// lowers the graph, so a server calls it from its blocking pool.
pub fn check(request: &CheckRequest) -> Result<Admitted, CheckError> {
    let root = tempfile::tempdir().map_err(|source| CheckError::Materialize {
        path: std::env::temp_dir(),
        source,
    })?;
    let workflow = materialize(root.path(), &request.bundle)?;
    let runtime = request.runtime.runtime(false);
    let inputs = compile_inputs(&request.inputs, &request.launch);
    let lowered = runtime
        .check(&workflow, None, None, &inputs)
        .map_err(CheckError::Load)?;
    let diagnostics: Vec<Diagnostic> = lowered
        .diagnostics
        .iter()
        .map(|diagnostic| convert(diagnostic, root.path()))
        .collect();
    match lowered.graph {
        Some(graph) => Ok(Admitted {
            graph,
            children: lowered.children,
            warnings: diagnostics,
        }),
        None => Err(CheckError::Rejected(diagnostics)),
    }
}

/// Write the bundle under `root/bundle/`, with `root/.fabro` beside it so
/// the frontend takes `root` as the bundle root. Returns the entrypoint's
/// path.
#[expect(
    clippy::disallowed_methods,
    reason = "the check is a blocking function; its caller runs it on the blocking pool"
)]
fn materialize(root: &Path, bundle: &Bundle) -> Result<PathBuf, CheckError> {
    let write = |relative: &str, text: &str| -> Result<(), CheckError> {
        let path = root.join(relative);
        let materialize = |source| CheckError::Materialize {
            path: path.clone(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(materialize)?;
        }
        std::fs::write(&path, text).map_err(materialize)
    };
    let fabro_dir = root.join(".fabro");
    std::fs::create_dir_all(&fabro_dir).map_err(|source| CheckError::Materialize {
        path: fabro_dir,
        source,
    })?;
    if let Some(project) = &bundle.project_toml {
        write(PROJECT_FILE, project)?;
    }
    for (relative, text) in &bundle.files {
        write(&format!("{BUNDLE_DIR}/{relative}"), text)?;
    }
    Ok(root.join(BUNDLE_DIR).join(&bundle.entrypoint))
}

/// The compile inputs: the intent's inputs, and the launch variables.
fn compile_inputs(inputs: &BTreeMap<String, Value>, launch: &Launch) -> CompileInputs {
    let mut compile = CompileInputs::new();
    for (name, value) in inputs {
        compile.inputs.insert(name.as_str().into(), value.clone());
    }
    let text = |value: &Option<String>| match value {
        Some(text) if !text.trim().is_empty() => Value::String(text.clone()),
        _ => Value::Null,
    };
    compile
        .vars
        .insert(LAUNCH_MODEL_VAR.into(), text(&launch.model));
    compile
        .vars
        .insert(LAUNCH_PROVIDER_VAR.into(), text(&launch.provider));
    // Bound even when absent: `Runtime::lower` would otherwise bind the
    // temporary bundle root, which is gone by the time the run starts.
    let repository = launch.repository.as_ref().map_or(Value::Null, |path| {
        Value::String(path.to_string_lossy().into_owned())
    });
    compile.vars.insert(REPOSITORY_VAR.into(), repository);
    compile
}

/// Petri's diagnostic in Fabro's shape, with the file made relative to the
/// bundle.
fn convert(diagnostic: &frontend::Diagnostic, root: &Path) -> Diagnostic {
    let file = diagnostic.span.file.as_str();
    let prefix = format!("{BUNDLE_DIR}/");
    let file = Path::new(file)
        .strip_prefix(root)
        .map_or(file, |relative| relative.to_str().unwrap_or(file))
        .to_string();
    let file = file
        .strip_prefix(&prefix)
        .map_or(file.as_str(), |relative| relative)
        .to_string();
    Diagnostic {
        severity: match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::Error,
            Severity::Warning => DiagnosticSeverity::Warning,
        },
        code: diagnostic.code.to_string(),
        message: diagnostic.message.clone(),
        hint: diagnostic.hint.clone(),
        file,
        line: (diagnostic.span.line > 0).then_some(diagnostic.span.line),
        column: (diagnostic.span.column > 0).then_some(diagnostic.span.column),
    }
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == DiagnosticSeverity::Error
    }
}
