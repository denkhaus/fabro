use std::collections::{HashMap, HashSet, VecDeque};

use fabro_types::ManifestPath;
use minijinja::machinery::ast::{Expr, Stmt};
use minijinja::machinery::{self, WhitespaceConfig};
use minijinja::syntax::SyntaxConfig;
use minijinja::value::Value;
use thiserror::Error;

use crate::TemplateError;
use crate::store::{TemplateLoadError, TemplateSource, TemplateStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateDependencyKind {
    Include,
    Extends,
    Import,
    FromImport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateDependency {
    pub kind:      TemplateDependencyKind,
    pub reference: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedTemplateDependencies {
    pub static_references:  Vec<TemplateDependency>,
    pub dynamic_references: Vec<TemplateDependencyKind>,
}

#[derive(Debug, Error)]
pub enum TemplateDiscoveryError {
    #[error("invalid template `{parent}`")]
    Parse {
        parent: ManifestPath,
        #[source]
        source: Box<TemplateError>,
    },
    #[error("failed to load a template dependency of `{parent}`")]
    Load {
        parent: ManifestPath,
        #[source]
        source: TemplateLoadError,
    },
    #[error("missing template dependency `{reference}` from `{parent}`")]
    Missing {
        parent:    ManifestPath,
        reference: String,
    },
    #[error("dynamic template dependency in `{parent}` must be declared explicitly")]
    Dynamic { parent: ManifestPath },
}

impl TemplateDiscoveryError {
    /// Path of the template source this error is attributed to.
    #[must_use]
    pub fn source_path(&self) -> &ManifestPath {
        match self {
            Self::Parse { parent, .. }
            | Self::Load { parent, .. }
            | Self::Missing { parent, .. }
            | Self::Dynamic { parent } => parent,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TemplateDependencyClosure {
    pub sources: HashMap<ManifestPath, TemplateSource>,
}

impl TemplateDependencyClosure {
    #[must_use]
    pub fn paths(&self) -> HashSet<ManifestPath> {
        self.sources.keys().cloned().collect()
    }
}

pub fn extract_template_dependencies(
    source_name: &str,
    source: &str,
) -> Result<ExtractedTemplateDependencies, TemplateError> {
    // MiniJinja's AST is behind `unstable_machinery`; keep direct usage in
    // this module so future MiniJinja upgrades have a single adapter surface.
    let parsed = machinery::parse(
        source,
        source_name,
        SyntaxConfig,
        WhitespaceConfig::default(),
    )
    .map_err(TemplateError::from)?;
    let mut dependencies = ExtractedTemplateDependencies::default();
    collect_stmt_dependencies(&parsed, &mut dependencies);
    Ok(dependencies)
}

pub fn discover_static_dependency_closure(
    roots: impl IntoIterator<Item = TemplateSource>,
    store: &dyn TemplateStore,
) -> Result<TemplateDependencyClosure, TemplateDiscoveryError> {
    let mut sources = HashMap::new();
    // A root and a loaded file can collide on `path` while carrying different
    // content (an inline prompt is anchored at its graph file's path), so
    // traversal dedup keys on the full occurrence rather than the path: a
    // path-keyed check would leave the collided occurrence unparsed. The
    // result map stays path-keyed, with the last distinct occurrence winning.
    let mut parsed = HashSet::new();
    let mut queue = VecDeque::new();

    let mut enqueue = |source: TemplateSource,
                       sources: &mut HashMap<ManifestPath, TemplateSource>,
                       queue: &mut VecDeque<TemplateSource>| {
        let occurrence = (
            source.path.clone(),
            source.root.clone(),
            source.content.clone(),
        );
        if parsed.insert(occurrence) {
            sources.insert(source.path.clone(), source.clone());
            queue.push_back(source);
        }
    };

    for source in roots {
        enqueue(source, &mut sources, &mut queue);
    }

    while let Some(source) = queue.pop_front() {
        let dependencies = extract_template_dependencies(&source.path.to_string(), &source.content)
            .map_err(|error| TemplateDiscoveryError::Parse {
                parent: source.path.clone(),
                source: Box::new(error),
            })?;
        if !dependencies.dynamic_references.is_empty() {
            return Err(TemplateDiscoveryError::Dynamic {
                parent: source.path,
            });
        }
        for dependency in dependencies.static_references {
            let loaded = store
                .load(&source, &dependency.reference)
                .map_err(|error| TemplateDiscoveryError::Load {
                    parent: source.path.clone(),
                    source: error,
                })?
                .ok_or_else(|| TemplateDiscoveryError::Missing {
                    parent:    source.path.clone(),
                    reference: dependency.reference.clone(),
                })?;
            enqueue(loaded, &mut sources, &mut queue);
        }
    }

    Ok(TemplateDependencyClosure { sources })
}

pub(crate) fn has_loader_dependent_tags(
    source_name: &str,
    source: &str,
) -> Result<Option<TemplateDependencyKind>, TemplateError> {
    let dependencies = extract_template_dependencies(source_name, source)?;
    Ok(dependencies
        .static_references
        .first()
        .map(|dependency| dependency.kind)
        .or_else(|| dependencies.dynamic_references.first().copied()))
}

fn collect_stmt_dependencies(stmt: &Stmt<'_>, dependencies: &mut ExtractedTemplateDependencies) {
    match stmt {
        Stmt::Template(template) => collect_stmt_list(&template.children, dependencies),
        Stmt::ForLoop(for_loop) => {
            collect_stmt_list(&for_loop.body, dependencies);
            collect_stmt_list(&for_loop.else_body, dependencies);
        }
        Stmt::IfCond(if_cond) => {
            collect_stmt_list(&if_cond.true_body, dependencies);
            collect_stmt_list(&if_cond.false_body, dependencies);
        }
        Stmt::WithBlock(with_block) => collect_stmt_list(&with_block.body, dependencies),
        Stmt::SetBlock(set_block) => collect_stmt_list(&set_block.body, dependencies),
        Stmt::AutoEscape(auto_escape) => collect_stmt_list(&auto_escape.body, dependencies),
        Stmt::FilterBlock(filter_block) => collect_stmt_list(&filter_block.body, dependencies),
        Stmt::Block(block) => collect_stmt_list(&block.body, dependencies),
        Stmt::Include(include) => {
            collect_loader_expr(TemplateDependencyKind::Include, &include.name, dependencies);
        }
        Stmt::Extends(extends) => {
            collect_loader_expr(TemplateDependencyKind::Extends, &extends.name, dependencies);
        }
        Stmt::Import(import) => {
            collect_loader_expr(TemplateDependencyKind::Import, &import.expr, dependencies);
        }
        Stmt::FromImport(from_import) => collect_loader_expr(
            TemplateDependencyKind::FromImport,
            &from_import.expr,
            dependencies,
        ),
        Stmt::Macro(macro_) => collect_stmt_list(&macro_.body, dependencies),
        Stmt::CallBlock(call_block) => collect_stmt_list(&call_block.macro_decl.body, dependencies),
        Stmt::EmitExpr(_) | Stmt::EmitRaw(_) | Stmt::Set(_) | Stmt::Do(_) => {}
    }
}

fn collect_stmt_list(stmts: &[Stmt<'_>], dependencies: &mut ExtractedTemplateDependencies) {
    for stmt in stmts {
        collect_stmt_dependencies(stmt, dependencies);
    }
}

fn collect_loader_expr(
    kind: TemplateDependencyKind,
    expr: &Expr<'_>,
    dependencies: &mut ExtractedTemplateDependencies,
) {
    if let Some(reference) = const_string(expr) {
        dependencies
            .static_references
            .push(TemplateDependency { kind, reference });
    } else {
        dependencies.dynamic_references.push(kind);
    }
}

fn const_string(expr: &Expr<'_>) -> Option<String> {
    let value: Value = expr.as_const()?;
    value.as_str().map(ToOwned::to_owned)
}
