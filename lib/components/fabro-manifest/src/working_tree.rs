use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use fabro_config::project::WorkflowLocation;
use fabro_config::{EnvironmentDockerfileLayer, EnvironmentImageLayer, SettingsLayer};
use fabro_graphviz::graph::AttrValue;
use fabro_graphviz::parser;
use fabro_template::{
    BundleTemplateStore, FilesystemTemplateStore, RecordingTemplateStore, TemplateContext,
    TemplateDependencyClosure, TemplateRenderMode, TemplateSource,
    discover_static_dependency_closure, render_source,
};
use fabro_types::ManifestPath;
use fabro_workflow::static_reference::{
    AttributeScope, ReferenceKind, reference_kind_for_attribute,
};

pub(super) struct CollectWorkingTreeInput<'a> {
    pub(super) cwd:            &'a Path,
    pub(super) root_location:  WorkflowLocation,
    pub(super) inputs:         &'a HashMap<String, toml::Value>,
    pub(super) project_config: Option<CollectedSourceInput>,
    pub(super) user_config:    Option<CollectedSourceInput>,
}

pub(super) struct CollectedSourceInput {
    pub(super) access_path: PathBuf,
    pub(super) source:      String,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedWorkingTree {
    pub(super) entrypoint:     CollectedPath,
    pub(super) workflows:      BTreeMap<CollectedPath, CollectedWorkflow>,
    pub(super) project_config: Option<CollectedDocument>,
    pub(super) user_config:    Option<CollectedDocument>,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedWorkflow {
    pub(super) graph:  CollectedDocument,
    pub(super) config: Option<CollectedDocument>,
    pub(super) files:  BTreeMap<CollectedPath, CollectedFile>,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedDocument {
    pub(super) access_path: PathBuf,
    pub(super) path:        CollectedPath,
    pub(super) source:      String,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedFile {
    pub(super) document:  CollectedDocument,
    pub(super) reference: CollectedFileReference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CollectedFileReferenceType {
    FileInline,
    Import,
    Dockerfile,
}

#[derive(Clone, Debug)]
pub(super) struct CollectedFileReference {
    pub(super) type_:            CollectedFileReferenceType,
    pub(super) original:         String,
    pub(super) from_access_path: Option<PathBuf>,
}

/// A canonical virtual coordinate inside one collected working-tree closure.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct CollectedPath(String);

impl CollectedPath {
    fn try_new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let value = path
            .to_str()
            .ok_or_else(|| anyhow!("collected path is not valid UTF-8"))?;

        if value.is_empty() {
            bail!("collected path must not be empty");
        }
        if path.is_absolute() {
            bail!("collected path must be relative: {value}");
        }
        if value.contains('\\') {
            bail!("collected path must use forward slashes: {value}");
        }
        if value.chars().any(char::is_control) {
            bail!("collected path contains a control character");
        }
        let bytes = value.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            bail!("collected path must not use a Windows drive prefix: {value}");
        }
        if value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            bail!("collected path contains an empty, dot, or parent component: {value}");
        }

        Ok(Self(value.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl fmt::Display for CollectedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ComponentRole {
    Workflow,
    ProjectConfig,
    UserConfig,
}

impl ComponentRole {
    const fn label(self) -> &'static str {
        match self {
            Self::Workflow => "workflow",
            Self::ProjectConfig => "project_config",
            Self::UserConfig => "user_config",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct DocumentId(usize);

#[derive(Clone, Debug)]
struct DraftDocument {
    access_path:      PathBuf,
    provisional_path: PathBuf,
    component:        ComponentRole,
    source:           String,
}

#[derive(Clone, Debug)]
struct DraftFileReference {
    type_:         CollectedFileReferenceType,
    original:      String,
    from_document: Option<DocumentId>,
}

#[derive(Clone, Debug)]
struct DraftFile {
    document:  DocumentId,
    reference: DraftFileReference,
}

#[derive(Clone, Debug)]
struct DraftWorkflow {
    graph:  DocumentId,
    config: Option<DocumentId>,
    files:  BTreeMap<String, DraftFile>,
}

struct CollectionDraft<'a> {
    cwd:               PathBuf,
    inputs:            &'a HashMap<String, toml::Value>,
    documents:         Vec<DraftDocument>,
    document_ids:      HashMap<(PathBuf, ComponentRole, PathBuf), DocumentId>,
    workflows:         BTreeMap<String, DraftWorkflow>,
    visited_workflows: HashMap<String, DocumentId>,
}

impl<'a> CollectionDraft<'a> {
    fn new(cwd: &Path, inputs: &'a HashMap<String, toml::Value>) -> Result<Self> {
        Ok(Self {
            cwd: normalized_absolute_access_path(cwd)?,
            inputs,
            documents: Vec::new(),
            document_ids: HashMap::new(),
            workflows: BTreeMap::new(),
            visited_workflows: HashMap::new(),
        })
    }

    fn insert_document(
        &mut self,
        access_path: &Path,
        provisional_path: PathBuf,
        component: ComponentRole,
        source: String,
    ) -> DocumentId {
        let access_path = lexically_normalize_access_path(access_path);
        let key = (access_path.clone(), component, provisional_path.clone());
        if let Some(document) = self.document_ids.get(&key) {
            return *document;
        }

        let document = DocumentId(self.documents.len());
        self.documents.push(DraftDocument {
            access_path,
            provisional_path,
            component,
            source,
        });
        self.document_ids.insert(key, document);
        document
    }

    fn document(&self, document: DocumentId) -> &DraftDocument {
        &self.documents[document.0]
    }

    fn collect_workflow_location(
        &mut self,
        location: &WorkflowLocation,
        provisional_graph_path: PathBuf,
    ) -> Result<DocumentId> {
        let graph_access_path = normalized_absolute_access_path(&location.graph)?;
        let graph_manifest_path = manifest_path_from_absolute(&graph_access_path, &self.cwd)?;
        let graph_key = graph_manifest_path.to_string();
        if let Some(document) = self.visited_workflows.get(&graph_key) {
            return Ok(*document);
        }

        let graph_source = std::fs::read_to_string(&graph_access_path)
            .with_context(|| format!("Failed to read {}", graph_access_path.display()))?;
        let graph = self.insert_document(
            &graph_access_path,
            provisional_graph_path,
            ComponentRole::Workflow,
            graph_source,
        );
        self.visited_workflows.insert(graph_key.clone(), graph);

        let config = location
            .toml
            .as_ref()
            .map(|config_path| {
                let access_path = normalized_absolute_access_path(config_path)?;
                let source = std::fs::read_to_string(&access_path)
                    .with_context(|| format!("Failed to read {}", access_path.display()))?;
                let file_name = access_path.file_name().ok_or_else(|| {
                    anyhow!(
                        "workflow config has no file name: {}",
                        access_path.display()
                    )
                })?;
                let provisional_path = virtual_sibling_path(
                    &self.document(graph).provisional_path,
                    Path::new(file_name),
                )?;
                Ok::<_, anyhow::Error>(self.insert_document(
                    &access_path,
                    provisional_path,
                    ComponentRole::Workflow,
                    source,
                ))
            })
            .transpose()?;

        let mut workflow = DraftWorkflow {
            graph,
            config,
            files: BTreeMap::new(),
        };
        if let Some(config) = config {
            self.collect_config_dockerfile(config, &mut workflow.files)?;
        }
        self.collect_workflow_files(graph, &mut workflow.files, &mut HashSet::new())?;
        self.workflows.insert(graph_key, workflow);

        Ok(graph)
    }

    fn collect_workflow_entry(
        &mut self,
        workflow: &Path,
        resolve_from: &Path,
        provisional_graph_path: PathBuf,
    ) -> Result<DocumentId> {
        let normalized_workflow = if workflow.extension().is_some() && workflow.is_relative() {
            normalize_absolute_path(resolve_from, &workflow.to_string_lossy()).ok_or_else(|| {
                anyhow!(
                    "unsupported manifest workflow reference: {}",
                    workflow.display()
                )
            })?
        } else {
            workflow.to_path_buf()
        };
        let location = WorkflowLocation::resolve(&normalized_workflow, resolve_from)?;
        self.collect_workflow_location(&location, provisional_graph_path)
    }

    fn collect_workflow_files(
        &mut self,
        graph_document_id: DocumentId,
        files: &mut BTreeMap<String, DraftFile>,
        visited_imports: &mut HashSet<String>,
    ) -> Result<()> {
        let graph = parser::parse(&self.document(graph_document_id).source).with_context(|| {
            format!(
                "Failed to parse {}",
                self.document(graph_document_id).access_path.display()
            )
        })?;
        let graph_access_path = self.document(graph_document_id).access_path.clone();
        let workflow_base_dir = graph_access_path.parent().unwrap_or_else(|| Path::new("."));
        let graph_manifest_path = manifest_path_from_absolute(&graph_access_path, &self.cwd)?;
        let workflow_template_root = manifest_parent_or_dot(&graph_manifest_path)?;

        if let Some(goal_reference) = graph.attrs.get("goal").and_then(AttrValue::as_str) {
            if let Some(reference) = goal_reference.strip_prefix('@') {
                let bundled = self.collect_bundled_file(
                    files,
                    workflow_base_dir,
                    reference,
                    CollectedFileReferenceType::FileInline,
                    manifest_attr_reference_kind(AttributeScope::Graph, "goal", goal_reference)?,
                    graph_document_id,
                )?;
                self.collect_bundled_template_includes(
                    files,
                    bundled,
                    &workflow_template_root,
                    graph_document_id,
                )?;
            } else {
                self.collect_template_include_files(
                    files,
                    TemplateSource::new(
                        graph_manifest_path.clone(),
                        workflow_template_root.clone(),
                        goal_reference.to_owned(),
                    ),
                    graph_document_id,
                    graph_document_id,
                )?;
            }
        }

        let mut nodes = graph.nodes.values().collect::<Vec<_>>();
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        for node in nodes {
            if let Some(prompt_reference) = node.attrs.get("prompt").and_then(AttrValue::as_str) {
                if !prompt_reference.starts_with('@') {
                    self.collect_template_include_files(
                        files,
                        TemplateSource::new(
                            graph_manifest_path.clone(),
                            workflow_template_root.clone(),
                            prompt_reference.to_owned(),
                        ),
                        graph_document_id,
                        graph_document_id,
                    )?;
                }
            }

            let mut attributes = node.attrs.iter().collect::<Vec<_>>();
            attributes.sort_by_key(|(name, _)| *name);
            for (name, value) in attributes {
                let Some(value) = value.as_str() else {
                    continue;
                };
                let Some(ReferenceKind::FileInline) =
                    reference_kind_for_attribute(AttributeScope::Node, name, value)
                else {
                    continue;
                };
                let reference = value.strip_prefix('@').ok_or_else(|| {
                    anyhow!("file inline reference must start with '@': {name}={value}")
                })?;
                let bundled = self.collect_bundled_file(
                    files,
                    workflow_base_dir,
                    reference,
                    CollectedFileReferenceType::FileInline,
                    ReferenceKind::FileInline,
                    graph_document_id,
                )?;

                if name == "prompt" {
                    self.collect_bundled_template_includes(
                        files,
                        bundled,
                        &workflow_template_root,
                        graph_document_id,
                    )?;
                }
            }

            if let Some(import_reference) = node.attrs.get("import").and_then(AttrValue::as_str) {
                let imported = self.collect_bundled_file(
                    files,
                    workflow_base_dir,
                    import_reference,
                    CollectedFileReferenceType::Import,
                    manifest_attr_reference_kind(AttributeScope::Node, "import", import_reference)?,
                    graph_document_id,
                )?;
                let import_key =
                    manifest_path_from_absolute(&self.document(imported).access_path, &self.cwd)?
                        .to_string();
                if visited_imports.insert(import_key) {
                    self.collect_workflow_files(imported, files, visited_imports)?;
                }
            }

            if let Some(child_reference) = node
                .attrs
                .get("stack.child_workflow")
                .and_then(AttrValue::as_str)
            {
                manifest_attr_reference_kind(
                    AttributeScope::Node,
                    "stack.child_workflow",
                    child_reference,
                )?
                .validate(child_reference)
                .map_err(anyhow::Error::new)?;
                let child_provisional_path = virtual_reference_path(
                    self.document(graph_document_id)
                        .provisional_path
                        .parent()
                        .unwrap_or_else(|| Path::new(".")),
                    child_reference,
                )?;
                self.collect_workflow_entry(
                    Path::new(child_reference),
                    workflow_base_dir,
                    child_provisional_path,
                )?;
            }
        }

        Ok(())
    }

    /// Collects the template dependency closure of an already-bundled
    /// `@`-referenced file (a goal or prompt document).
    fn collect_bundled_template_includes(
        &mut self,
        files: &mut BTreeMap<String, DraftFile>,
        bundled: DocumentId,
        workflow_template_root: &ManifestPath,
        from_document: DocumentId,
    ) -> Result<()> {
        let document = self.document(bundled);
        let source = document.source.clone();
        let bundled_manifest_path = manifest_path_from_absolute(&document.access_path, &self.cwd)?;
        let template_root =
            template_root_for_bundled_file(&bundled_manifest_path, workflow_template_root)?;
        self.collect_template_include_files(
            files,
            TemplateSource::new(bundled_manifest_path, template_root, source),
            bundled,
            from_document,
        )
    }

    fn collect_template_include_files(
        &mut self,
        files: &mut BTreeMap<String, DraftFile>,
        source: TemplateSource,
        source_document: DocumentId,
        from_document: DocumentId,
    ) -> Result<()> {
        let source_path = source.path.clone();
        let stable_root = stable_template_root(self.document(source_document), &source)?;
        let store = FilesystemTemplateStore::new(self.cwd.clone());
        let closure = discover_static_dependency_closure([source], &store)
            .context("failed to discover template dependencies")?;
        self.verify_recorded_template_dependencies(&source_path, &closure, files, from_document)?;

        let mut sources = closure.sources.into_iter().collect::<Vec<_>>();
        sources.sort_by_key(|(path, _)| path.to_string());
        for (path, source) in sources {
            if path == source_path {
                continue;
            }
            let relative = path
                .as_path()
                .strip_prefix(source.root.as_path())
                .map_err(|_| {
                    anyhow!(
                        "template path {path} is outside its logical root {}",
                        source.root
                    )
                })?;
            let provisional_path = normalize_relative_path(&stable_root.join(relative))?;
            let key = path.to_string();
            if let Some(existing) = files.get(&key) {
                let existing_path = &self.document(existing.document).provisional_path;
                if existing_path != &provisional_path {
                    bail!(
                        "collected file has conflicting logical coordinates `{}` and `{}`",
                        existing_path.display(),
                        provisional_path.display()
                    );
                }
                continue;
            }

            let access_path = lexically_normalize_access_path(&self.cwd.join(path.as_path()));
            let document = self.insert_document(
                &access_path,
                provisional_path,
                ComponentRole::Workflow,
                source.content,
            );
            files.insert(key.clone(), DraftFile {
                document,
                reference: DraftFileReference {
                    type_:         CollectedFileReferenceType::FileInline,
                    original:      key,
                    from_document: Some(from_document),
                },
            });
        }
        Ok(())
    }

    fn verify_recorded_template_dependencies(
        &self,
        source_path: &ManifestPath,
        closure: &TemplateDependencyClosure,
        files: &BTreeMap<String, DraftFile>,
        from_document: DocumentId,
    ) -> Result<()> {
        let Some(source) = closure.sources.get(source_path) else {
            return Ok(());
        };
        let mut bundled_files = closure
            .sources
            .iter()
            .map(|(path, source)| (path.clone(), source.content.clone()))
            .collect::<HashMap<_, _>>();
        for (key, file) in files {
            let path = ManifestPath::from_wire(key)
                .ok_or_else(|| anyhow!("invalid collected file key: {key}"))?;
            bundled_files.insert(path, self.document(file.document).source.clone());
        }
        let allowed = bundled_files.keys().cloned().collect();
        let store =
            RecordingTemplateStore::with_allowed(BundleTemplateStore::new(bundled_files), allowed);
        let context = TemplateContext::for_input_scan(self.inputs.clone());
        render_source(
            source,
            &context,
            Arc::new(store),
            TemplateRenderMode::Lenient,
        )
        .with_context(|| {
            let from =
                manifest_path_from_absolute(&self.document(from_document).access_path, &self.cwd)
                    .map_or_else(|_| source_path.to_string(), |path| path.to_string());
            format!("failed to verify template dependencies for {from}")
        })?;
        Ok(())
    }

    fn collect_config_dockerfile(
        &mut self,
        config: DocumentId,
        files: &mut BTreeMap<String, DraftFile>,
    ) -> Result<()> {
        let layer = self
            .document(config)
            .source
            .parse::<SettingsLayer>()
            .context("Failed to parse run config TOML")?;
        let config_access_path = self.document(config).access_path.clone();
        let base_dir = config_access_path
            .parent()
            .unwrap_or_else(|| Path::new("."));

        let mut environments = layer.environments.iter().collect::<Vec<_>>();
        environments.sort_by_key(|(name, _)| *name);
        for (_, environment) in environments {
            self.collect_environment_dockerfile(
                files,
                base_dir,
                config,
                environment.image.as_ref(),
            )?;
        }
        if let Some(run_environment) = layer.run.as_ref().and_then(|run| run.environment.as_ref()) {
            self.collect_environment_dockerfile(
                files,
                base_dir,
                config,
                run_environment.image.as_ref(),
            )?;
        }
        Ok(())
    }

    fn collect_environment_dockerfile(
        &mut self,
        files: &mut BTreeMap<String, DraftFile>,
        base_dir: &Path,
        config: DocumentId,
        image: Option<&EnvironmentImageLayer>,
    ) -> Result<()> {
        let dockerfile = image.and_then(|image| image.dockerfile.as_ref());
        let Some(EnvironmentDockerfileLayer::Path { path }) = dockerfile else {
            return Ok(());
        };
        self.collect_bundled_file(
            files,
            base_dir,
            path,
            CollectedFileReferenceType::Dockerfile,
            ReferenceKind::Dockerfile,
            config,
        )?;
        Ok(())
    }

    fn collect_bundled_file(
        &mut self,
        files: &mut BTreeMap<String, DraftFile>,
        base_dir: &Path,
        reference: &str,
        reference_type: CollectedFileReferenceType,
        reference_kind: ReferenceKind,
        from_document: DocumentId,
    ) -> Result<DocumentId> {
        reference_kind
            .validate(reference)
            .map_err(anyhow::Error::new)?;

        let access_path = normalize_absolute_path(base_dir, reference)
            .ok_or_else(|| anyhow!("unsupported manifest reference: {reference}"))?;
        let manifest_path = manifest_path_from_absolute(&access_path, &self.cwd)?;
        let key = manifest_path.to_string();
        let provisional_path = virtual_reference_path(
            self.document(from_document)
                .provisional_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
            reference,
        )?;

        if let Some(existing) = files.get(&key) {
            let existing_path = &self.document(existing.document).provisional_path;
            if existing_path != &provisional_path {
                bail!(
                    "collected file has conflicting logical coordinates `{}` and `{}`",
                    existing_path.display(),
                    provisional_path.display()
                );
            }
            return Ok(existing.document);
        }

        let source = std::fs::read_to_string(&access_path)
            .with_context(|| format!("Failed to read {}", access_path.display()))?;
        let document = self.insert_document(
            &access_path,
            provisional_path,
            self.document(from_document).component,
            source,
        );
        files.insert(key, DraftFile {
            document,
            reference: DraftFileReference {
                type_:         reference_type,
                original:      reference.to_owned(),
                from_document: Some(from_document),
            },
        });
        Ok(document)
    }

    fn finish(
        self,
        entrypoint: DocumentId,
        project_config: Option<DocumentId>,
        user_config: Option<DocumentId>,
    ) -> Result<CollectedWorkingTree> {
        let documents = finalize_documents(self.documents)?;
        let mut workflows = BTreeMap::new();
        for workflow in self.workflows.into_values() {
            let graph = documents[workflow.graph.0].clone();
            let config = workflow
                .config
                .map(|document| documents[document.0].clone());
            let mut files = BTreeMap::new();
            for file in workflow.files.into_values() {
                let document = documents[file.document.0].clone();
                let from_access_path = file
                    .reference
                    .from_document
                    .map(|from| documents[from.0].access_path.clone());
                files.insert(document.path.clone(), CollectedFile {
                    document,
                    reference: CollectedFileReference {
                        type_: file.reference.type_,
                        original: file.reference.original,
                        from_access_path,
                    },
                });
            }
            workflows.insert(graph.path.clone(), CollectedWorkflow {
                graph,
                config,
                files,
            });
        }

        Ok(CollectedWorkingTree {
            entrypoint: documents[entrypoint.0].path.clone(),
            workflows,
            project_config: project_config.map(|document| documents[document.0].clone()),
            user_config: user_config.map(|document| documents[document.0].clone()),
        })
    }
}

/// Finalizes draft documents into collected documents, rejecting conflicting
/// physical aliases and virtual-coordinate collisions.
fn finalize_documents(drafts: Vec<DraftDocument>) -> Result<Vec<CollectedDocument>> {
    let mut deficits = BTreeMap::<ComponentRole, usize>::new();
    for draft in &drafts {
        let deficit = leading_parent_count(&draft.provisional_path);
        deficits
            .entry(draft.component)
            .and_modify(|current| *current = (*current).max(deficit))
            .or_insert(deficit);
    }

    let paths = drafts
        .iter()
        .map(|draft| {
            finalize_component_path(
                &draft.provisional_path,
                draft.component,
                deficits.get(&draft.component).copied().unwrap_or_default(),
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let mut order = (0..drafts.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        paths[*left]
            .cmp(&paths[*right])
            .then_with(|| drafts[*left].access_path.cmp(&drafts[*right].access_path))
    });

    let mut physical_to_virtual = BTreeMap::<PathBuf, CollectedPath>::new();
    let mut virtual_to_physical = BTreeMap::<CollectedPath, PathBuf>::new();
    for index in order {
        let physical = std::fs::canonicalize(&drafts[index].access_path).with_context(|| {
            format!(
                "failed to identify collected file {}",
                drafts[index].access_path.display()
            )
        })?;
        let path = &paths[index];
        if let Some(existing) = physical_to_virtual.get(&physical) {
            if existing != path {
                bail!(
                    "one physical file has conflicting collected coordinates `{existing}` and `{path}`"
                );
            }
        }
        if let Some(existing) = virtual_to_physical.get(path) {
            if existing != &physical {
                bail!("collected coordinate `{path}` maps to multiple physical files");
            }
        }
        physical_to_virtual.insert(physical.clone(), path.clone());
        virtual_to_physical.insert(path.clone(), physical);
    }

    Ok(drafts
        .into_iter()
        .zip(paths)
        .map(|(draft, path)| CollectedDocument {
            access_path: draft.access_path,
            path,
            source: draft.source,
        })
        .collect())
}

pub(super) fn collect_working_tree(
    input: CollectWorkingTreeInput<'_>,
) -> Result<CollectedWorkingTree> {
    let mut draft = CollectionDraft::new(input.cwd, input.inputs)?;
    let root_graph_access_path = normalized_absolute_access_path(&input.root_location.graph)?;
    let root_graph_path =
        seed_component_path(&root_graph_access_path, &draft.cwd, ComponentRole::Workflow)?;

    let project_config = input
        .project_config
        .map(|config| seed_config_document(&mut draft, config, ComponentRole::ProjectConfig))
        .transpose()?;
    let user_config = input
        .user_config
        .map(|config| seed_config_document(&mut draft, config, ComponentRole::UserConfig))
        .transpose()?;

    let entrypoint = draft.collect_workflow_location(&input.root_location, root_graph_path)?;
    if let Some(project_config) = project_config {
        let root_key =
            manifest_path_from_absolute(&draft.document(entrypoint).access_path, &draft.cwd)?
                .to_string();
        let mut root = draft
            .workflows
            .remove(&root_key)
            .ok_or_else(|| anyhow!("root workflow missing from collected working tree"))?;
        draft.collect_config_dockerfile(project_config, &mut root.files)?;
        draft.workflows.insert(root_key, root);
    }

    draft.finish(entrypoint, project_config, user_config)
}

fn seed_config_document(
    draft: &mut CollectionDraft<'_>,
    config: CollectedSourceInput,
    role: ComponentRole,
) -> Result<DocumentId> {
    let access_path = normalized_absolute_access_path(&config.access_path)?;
    let provisional_path = seed_component_path(&access_path, &draft.cwd, role)?;
    Ok(draft.insert_document(&access_path, provisional_path, role, config.source))
}

fn seed_component_path(
    access_path: &Path,
    cwd: &Path,
    component: ComponentRole,
) -> Result<PathBuf> {
    if !matches!(component, ComponentRole::UserConfig) {
        if let Ok(relative) = access_path.strip_prefix(cwd) {
            let relative = normalize_relative_path(relative)?;
            if leading_parent_count(&relative) == 0 && !relative.as_os_str().is_empty() {
                return Ok(relative);
            }
        }
    }

    let file_name = access_path
        .file_name()
        .ok_or_else(|| anyhow!("collected root has no file name: {}", access_path.display()))?;
    let root = match component {
        ComponentRole::Workflow => PathBuf::from("_fabro_external/entrypoint"),
        ComponentRole::ProjectConfig => PathBuf::from("_fabro_external/project_config"),
        ComponentRole::UserConfig => PathBuf::from("_fabro_external/user_config"),
    };
    normalize_relative_path(&root.join(file_name))
}

fn stable_template_root(document: &DraftDocument, source: &TemplateSource) -> Result<PathBuf> {
    let relative = source
        .path
        .as_path()
        .strip_prefix(source.root.as_path())
        .map_err(|_| {
            anyhow!(
                "template source {} is outside its logical root {}",
                source.path,
                source.root
            )
        })?;
    let mut root = document.provisional_path.clone();
    for component in relative.components() {
        if matches!(component, Component::Normal(_)) && !root.pop() {
            bail!(
                "template source {} cannot be placed under its collected root",
                source.path
            );
        }
    }
    Ok(root)
}

fn finalize_component_path(
    provisional: &Path,
    component: ComponentRole,
    deficit: usize,
) -> Result<CollectedPath> {
    let path = if deficit == 0 {
        normalize_relative_path(provisional)?
    } else {
        let mut prefix = PathBuf::from("_fabro_rebased");
        prefix.push(component.label());
        for _ in 0..deficit {
            prefix.push("anchor");
        }
        normalize_relative_path(&prefix.join(provisional))?
    };
    CollectedPath::try_new(virtual_path_to_wire(&path)?)
}

fn virtual_path_to_wire(path: &Path) -> Result<String> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            bail!("collected path is not finalized: {}", path.display());
        };
        segments.push(
            segment
                .to_str()
                .ok_or_else(|| anyhow!("collected path is not valid UTF-8"))?,
        );
    }
    Ok(segments.join("/"))
}

fn virtual_sibling_path(path: &Path, sibling: &Path) -> Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    normalize_relative_path(&parent.join(sibling))
}

fn virtual_reference_path(base: &Path, reference: &str) -> Result<PathBuf> {
    let reference_path = Path::new(reference);
    if reference_path.is_absolute() || reference.starts_with('~') {
        bail!("unsupported collected reference: {reference}");
    }
    normalize_relative_path(&base.join(reference_path))
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                } else {
                    normalized.push("..");
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("collected path must be relative: {}", path.display());
            }
        }
    }
    Ok(normalized)
}

fn leading_parent_count(path: &Path) -> usize {
    path.components()
        .take_while(|component| matches!(component, Component::ParentDir))
        .count()
}

fn lexically_normalize_access_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir => normalized.push(Path::new("/")),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    normalized
}

fn normalized_absolute_access_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("failed to make collected path absolute: {}", path.display()))?;
    Ok(lexically_normalize_access_path(&absolute))
}

pub(super) fn normalize_absolute_path(base_dir: &Path, reference: &str) -> Option<PathBuf> {
    let path = Path::new(reference);
    if path.is_absolute() || reference.starts_with('~') {
        return None;
    }
    Some(lexically_normalize_access_path(&base_dir.join(path)))
}

pub(super) fn manifest_path_from_absolute(path: &Path, cwd: &Path) -> Result<ManifestPath> {
    ManifestPath::from_absolute(path, cwd)
        .ok_or_else(|| anyhow!("Failed to compute manifest path for {}", path.display()))
}

fn manifest_parent_or_dot(path: &ManifestPath) -> Result<ManifestPath> {
    let parent = path.parent_or_dot().to_string_lossy();
    ManifestPath::from_wire(&parent)
        .ok_or_else(|| anyhow!("invalid manifest parent path for {path}: {parent}"))
}

fn template_root_for_bundled_file(
    path: &ManifestPath,
    workflow_template_root: &ManifestPath,
) -> Result<ManifestPath> {
    if manifest_path_is_within_root(path, workflow_template_root) {
        Ok(workflow_template_root.clone())
    } else {
        manifest_parent_or_dot(path)
    }
}

fn manifest_path_is_within_root(path: &ManifestPath, root: &ManifestPath) -> bool {
    if root.as_path().as_os_str().is_empty() {
        return !matches!(
            path.as_path().components().next(),
            Some(Component::ParentDir)
        );
    }
    path.starts_with(root)
}

fn manifest_attr_reference_kind(
    scope: AttributeScope,
    key: &str,
    value: &str,
) -> Result<ReferenceKind> {
    reference_kind_for_attribute(scope, key, value)
        .ok_or_else(|| anyhow!("unsupported manifest reference attribute: {key}={value}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, source: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("fixture directory should be created");
        }
        std::fs::write(path, source).expect("fixture file should be written");
    }

    fn collect_graph(cwd: &Path, graph: &Path) -> Result<CollectedWorkingTree> {
        let inputs = HashMap::new();
        let root_location = WorkflowLocation::resolve(graph, cwd)?;
        collect_working_tree(CollectWorkingTreeInput {
            cwd,
            root_location,
            inputs: &inputs,
            project_config: None,
            user_config: None,
        })
    }

    fn logical_contents(tree: &CollectedWorkingTree) -> BTreeMap<String, String> {
        let mut contents = BTreeMap::new();
        if let Some(config) = &tree.project_config {
            contents.insert(config.path.to_string(), config.source.clone());
        }
        if let Some(config) = &tree.user_config {
            contents.insert(config.path.to_string(), config.source.clone());
        }
        for workflow in tree.workflows.values() {
            contents.insert(
                workflow.graph.path.to_string(),
                workflow.graph.source.clone(),
            );
            if let Some(config) = &workflow.config {
                contents.insert(config.path.to_string(), config.source.clone());
            }
            for file in workflow.files.values() {
                contents.insert(file.document.path.to_string(), file.document.source.clone());
            }
        }
        contents
    }

    fn logical_provenance(
        tree: &CollectedWorkingTree,
    ) -> BTreeMap<String, (CollectedFileReferenceType, String, Option<String>)> {
        let mut paths_by_access = HashMap::new();
        if let Some(config) = &tree.project_config {
            paths_by_access.insert(config.access_path.clone(), config.path.to_string());
        }
        if let Some(config) = &tree.user_config {
            paths_by_access.insert(config.access_path.clone(), config.path.to_string());
        }
        for workflow in tree.workflows.values() {
            paths_by_access.insert(
                workflow.graph.access_path.clone(),
                workflow.graph.path.to_string(),
            );
            if let Some(config) = &workflow.config {
                paths_by_access.insert(config.access_path.clone(), config.path.to_string());
            }
            for file in workflow.files.values() {
                paths_by_access.insert(
                    file.document.access_path.clone(),
                    file.document.path.to_string(),
                );
            }
        }

        let mut provenance = BTreeMap::new();
        for workflow in tree.workflows.values() {
            for file in workflow.files.values() {
                let from = file.reference.from_access_path.as_ref().map(|access_path| {
                    paths_by_access
                        .get(access_path)
                        .expect("reference source should be collected")
                        .clone()
                });
                provenance.insert(
                    file.document.path.to_string(),
                    (file.reference.type_, file.reference.original.clone(), from),
                );
            }
        }
        provenance
    }

    #[test]
    fn collected_path_rejects_parent_components() {
        assert!(CollectedPath::try_new("../prompt.md").is_err());
    }

    #[test]
    fn collected_path_rejects_non_canonical_forms() {
        for value in ["", ".", "a/./b", "a//b", "a/../b", "C:/a", "a\\b", "a\nb"] {
            assert!(CollectedPath::try_new(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn component_rebase_is_uniform_and_preserves_relative_relationships() {
        let root = Path::new("_fabro_external/entrypoint/workflow.fabro");
        let sibling = virtual_reference_path(
            root.parent().expect("entrypoint should have a parent"),
            "../../../sibling/workflow.fabro",
        )
        .expect("reference should normalize");
        let deficit = leading_parent_count(&sibling);

        let rebased_root = finalize_component_path(root, ComponentRole::Workflow, deficit)
            .expect("root should finalize");
        let rebased_sibling = finalize_component_path(&sibling, ComponentRole::Workflow, deficit)
            .expect("sibling should finalize");
        let resolved = virtual_reference_path(
            rebased_root
                .as_path()
                .parent()
                .expect("rebased root should have a parent"),
            "../../../sibling/workflow.fabro",
        )
        .expect("rebased reference should normalize");

        assert_eq!(resolved, rebased_sibling.as_path());
        assert!(!rebased_root.as_str().contains(".."));
        assert!(!rebased_sibling.as_str().contains(".."));
    }

    #[test]
    fn collector_captures_complete_workflow_and_config_closure() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let project = temp.path().join("project");
        let root = project.join(".fabro/workflows/root");
        let child = project.join(".fabro/workflows/child");
        let project_config_path = project.join(".fabro/project.toml");
        let user_config_path = temp.path().join("home/.fabro/config.toml");
        let project_config = r#"_version = 1

[environments.project]
provider = "docker"

[environments.project.image]
dockerfile = { path = "Project.Dockerfile" }
"#;
        let workflow_config = r#"_version = 1

[workflow]
graph = "workflow.fabro"

[environments.workflow]
provider = "docker"

[environments.workflow.image]
dockerfile = { path = "Dockerfile" }
"#;
        let user_config = "_version = 1\n";
        write_file(&project_config_path, project_config);
        write_file(&project.join(".fabro/Project.Dockerfile"), "FROM project\n");
        write_file(&user_config_path, user_config);
        write_file(&root.join("workflow.toml"), workflow_config);
        write_file(&root.join("Dockerfile"), "FROM workflow\n");
        write_file(
            &root.join("workflow.fabro"),
            r#"digraph Root {
                start [shape=Mdiamond]
                prompt [prompt="@prompts/plan.md"]
                imported [import="imports/shared.fabro"]
                child [shape=house, stack.child_workflow="../child/workflow.fabro"]
                exit [shape=Msquare]
                start -> prompt -> imported -> child -> exit
            }"#,
        );
        write_file(
            &root.join("prompts/plan.md"),
            r#"{% include "partial.md" %}"#,
        );
        write_file(&root.join("prompts/partial.md"), "partial\n");
        write_file(
            &root.join("imports/shared.fabro"),
            r#"digraph Shared {
                start [shape=Mdiamond]
                shared [prompt="@../prompts/shared.md"]
                exit [shape=Msquare]
                start -> shared -> exit
            }"#,
        );
        write_file(&root.join("prompts/shared.md"), "shared\n");
        write_file(&child.join("workflow.toml"), workflow_config);
        write_file(&child.join("Dockerfile"), "FROM child\n");
        write_file(
            &child.join("workflow.fabro"),
            "digraph Child { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }",
        );

        let inputs = HashMap::new();
        let tree = collect_working_tree(CollectWorkingTreeInput {
            cwd:            &project,
            root_location:  WorkflowLocation::resolve(&root.join("workflow.toml"), &project)
                .expect("root workflow should resolve"),
            inputs:         &inputs,
            project_config: Some(CollectedSourceInput {
                access_path: project_config_path,
                source:      project_config.to_owned(),
            }),
            user_config:    Some(CollectedSourceInput {
                access_path: user_config_path,
                source:      user_config.to_owned(),
            }),
        })
        .expect("working tree should collect");

        assert_eq!(
            tree.entrypoint.as_str(),
            ".fabro/workflows/root/workflow.fabro"
        );
        assert_eq!(tree.workflows.len(), 2);
        assert_eq!(
            logical_contents(&tree)
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                ".fabro/Project.Dockerfile",
                ".fabro/project.toml",
                ".fabro/workflows/child/Dockerfile",
                ".fabro/workflows/child/workflow.fabro",
                ".fabro/workflows/child/workflow.toml",
                ".fabro/workflows/root/Dockerfile",
                ".fabro/workflows/root/imports/shared.fabro",
                ".fabro/workflows/root/prompts/partial.md",
                ".fabro/workflows/root/prompts/plan.md",
                ".fabro/workflows/root/prompts/shared.md",
                ".fabro/workflows/root/workflow.fabro",
                ".fabro/workflows/root/workflow.toml",
                "_fabro_external/user_config/config.toml",
            ]
        );
    }

    #[test]
    fn external_collection_is_stable_when_the_checkout_moves() {
        fn fixture(parent: &Path) -> (PathBuf, PathBuf) {
            let cwd = parent.join("checkout");
            let workflow = parent.join("catalog/root/workflow.fabro");
            std::fs::create_dir_all(&cwd).expect("checkout should be created");
            write_file(
                &workflow,
                r#"digraph Root {
                    start [shape=Mdiamond]
                    work [prompt="@prompts/plan.md"]
                    exit [shape=Msquare]
                    start -> work -> exit
                }"#,
            );
            write_file(&parent.join("catalog/root/prompts/plan.md"), "plan\n");
            (cwd, workflow)
        }

        let first = tempfile::tempdir().expect("first temp directory should be created");
        let second = tempfile::tempdir().expect("second temp directory should be created");
        let (first_cwd, first_workflow) = fixture(first.path());
        let (second_cwd, second_workflow) = fixture(second.path());

        let first_tree =
            collect_graph(&first_cwd, &first_workflow).expect("first working tree should collect");
        let second_tree = collect_graph(&second_cwd, &second_workflow)
            .expect("second working tree should collect");

        assert_eq!(first_tree.entrypoint, second_tree.entrypoint);
        assert_eq!(
            logical_contents(&first_tree),
            logical_contents(&second_tree)
        );
        assert_eq!(
            logical_provenance(&first_tree),
            logical_provenance(&second_tree)
        );
        assert_eq!(
            first_tree.entrypoint.as_str(),
            "_fabro_external/entrypoint/workflow.fabro"
        );
    }

    #[test]
    fn external_sibling_workflow_reference_resolves_in_stable_namespace() {
        fn fixture(parent: &Path) -> (PathBuf, PathBuf) {
            let cwd = parent.join("checkout");
            let root_dir = parent.join("user/workflows/root");
            let child_dir = parent.join("user/workflows/child");
            let root = root_dir.join("workflow.fabro");
            std::fs::create_dir_all(&cwd).expect("checkout should be created");
            write_file(
                &root,
                r#"digraph Root {
                    start [shape=Mdiamond]
                    prompt [prompt="@prompts/root.md"]
                    imported [import="imports/root.fabro"]
                    child [shape=house, stack.child_workflow="../child/workflow.fabro"]
                    exit [shape=Msquare]
                    start -> prompt -> imported -> child -> exit
                }"#,
            );
            write_file(
                &root_dir.join("prompts/root.md"),
                r#"{% include "root-partial.md" %}"#,
            );
            write_file(&root_dir.join("prompts/root-partial.md"), "root partial\n");
            write_file(
                &root_dir.join("imports/root.fabro"),
                r#"digraph Import {
                    start [shape=Mdiamond]
                    work [prompt="@../prompts/root-import.md"]
                    exit [shape=Msquare]
                    start -> work -> exit
                }"#,
            );
            write_file(&root_dir.join("prompts/root-import.md"), "root import\n");
            write_file(
                &child_dir.join("workflow.fabro"),
                r#"digraph Child {
                    start [shape=Mdiamond]
                    prompt [prompt="@prompts/child.md"]
                    imported [import="imports/child.fabro"]
                    exit [shape=Msquare]
                    start -> prompt -> imported -> exit
                }"#,
            );
            write_file(
                &child_dir.join("prompts/child.md"),
                r#"{% include "child-partial.md" %}"#,
            );
            write_file(
                &child_dir.join("prompts/child-partial.md"),
                "child partial\n",
            );
            write_file(
                &child_dir.join("imports/child.fabro"),
                r#"digraph Import {
                    start [shape=Mdiamond]
                    work [prompt="@../prompts/child-import.md"]
                    exit [shape=Msquare]
                    start -> work -> exit
                }"#,
            );
            write_file(&child_dir.join("prompts/child-import.md"), "child import\n");
            (cwd, root)
        }

        let first = tempfile::tempdir().expect("first temp directory should be created");
        let second = tempfile::tempdir().expect("second temp directory should be created");
        let (first_cwd, first_root) = fixture(first.path());
        let (second_cwd, second_root) = fixture(second.path());
        let tree =
            collect_graph(&first_cwd, &first_root).expect("first working tree should collect");
        let moved =
            collect_graph(&second_cwd, &second_root).expect("moved working tree should collect");
        let entrypoint = &tree.entrypoint;
        let resolved = virtual_reference_path(
            entrypoint
                .as_path()
                .parent()
                .expect("entrypoint should have a parent"),
            "../child/workflow.fabro",
        )
        .expect("child reference should resolve");

        assert_eq!(resolved, Path::new("_fabro_external/child/workflow.fabro"));
        assert!(tree.workflows.contains_key(
            &CollectedPath::try_new(resolved).expect("child path should be canonical")
        ));
        assert_eq!(logical_contents(&tree), logical_contents(&moved));
        assert_eq!(logical_provenance(&tree), logical_provenance(&moved));
        assert_eq!(tree.entrypoint, moved.entrypoint);
        for path in logical_contents(&tree).into_keys() {
            assert!(!path.contains(first.path().file_name().unwrap().to_string_lossy().as_ref()));
            CollectedPath::try_new(path).expect("every collected coordinate should be canonical");
        }
    }

    #[test]
    fn repeated_references_collect_one_file() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let cwd = temp.path();
        let graph = cwd.join("workflow.fabro");
        write_file(
            &graph,
            r#"digraph Root {
                start [shape=Mdiamond]
                first [prompt="@prompt.md"]
                second [prompt="@prompt.md"]
                exit [shape=Msquare]
                start -> first -> second -> exit
            }"#,
        );
        write_file(&cwd.join("prompt.md"), "prompt\n");

        let tree = collect_graph(cwd, &graph).expect("working tree should collect");
        let root = tree
            .workflows
            .get(&tree.entrypoint)
            .expect("root workflow should be present");

        assert_eq!(root.files.len(), 1);
        let assembled =
            crate::assemble_current_manifest(tree, cwd).expect("legacy manifest should assemble");
        assert_eq!(assembled.workflows["workflow.fabro"].files.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn collector_rejects_one_physical_file_with_two_coordinates() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let cwd = temp.path();
        let graph = cwd.join("workflow.fabro");
        write_file(
            &graph,
            r#"digraph Root {
                start [shape=Mdiamond]
                first [prompt="@first.md"]
                second [prompt="@second.md"]
                exit [shape=Msquare]
                start -> first -> second -> exit
            }"#,
        );
        write_file(&cwd.join("actual.md"), "prompt\n");
        std::os::unix::fs::symlink("actual.md", cwd.join("first.md"))
            .expect("first symlink should be created");
        std::os::unix::fs::symlink("actual.md", cwd.join("second.md"))
            .expect("second symlink should be created");

        let error = collect_graph(cwd, &graph).expect_err("alias should be rejected");

        assert!(
            error
                .to_string()
                .contains("one physical file has conflicting collected coordinates"),
            "unexpected error: {error:#}"
        );
        assert!(error.to_string().contains("first.md"));
        assert!(error.to_string().contains("second.md"));
    }

    #[test]
    fn namespace_rejects_two_physical_files_at_one_coordinate() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let first = temp.path().join("first.md");
        let second = temp.path().join("second.md");
        write_file(&first, "first\n");
        write_file(&second, "second\n");
        let draft = |access_path| DraftDocument {
            access_path,
            provisional_path: PathBuf::from("shared.md"),
            component: ComponentRole::Workflow,
            source: String::new(),
        };

        let error = finalize_documents(vec![draft(first), draft(second)])
            .expect_err("virtual collision should be rejected");

        assert!(
            error
                .to_string()
                .contains("collected coordinate `shared.md` maps to multiple physical files"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn namespace_identity_errors_keep_the_io_error_in_the_source_chain() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let draft = DraftDocument {
            access_path:      temp.path().join("missing.md"),
            provisional_path: PathBuf::from("missing.md"),
            component:        ComponentRole::Workflow,
            source:           String::new(),
        };

        let error =
            finalize_documents(vec![draft]).expect_err("missing physical identity should fail");

        assert!(
            error
                .chain()
                .any(|cause| cause.downcast_ref::<std::io::Error>().is_some()),
            "unexpected error chain: {error:#}"
        );
    }

    #[test]
    fn read_errors_keep_the_io_error_in_the_source_chain() {
        let temp = tempfile::tempdir().expect("temp directory should be created");
        let cwd = temp.path();
        let graph = cwd.join("workflow.fabro");
        write_file(
            &graph,
            r#"digraph Root {
                start [shape=Mdiamond]
                work [prompt="@missing.md"]
                exit [shape=Msquare]
                start -> work -> exit
            }"#,
        );

        let error = collect_graph(cwd, &graph).expect_err("missing file should fail");

        assert!(
            error
                .chain()
                .any(|cause| cause.downcast_ref::<std::io::Error>().is_some()),
            "unexpected error chain: {error:#}"
        );
    }

    #[test]
    fn collector_does_not_push_an_ahead_branch() {
        fn commit_all(repository: &git2::Repository, message: &str) -> git2::Oid {
            let mut index = repository.index().expect("index should open");
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .expect("fixture files should be staged");
            index.write().expect("index should be written");
            let tree_id = index.write_tree().expect("tree should be written");
            let tree = repository.find_tree(tree_id).expect("tree should exist");
            let signature = git2::Signature::now("Fabro Test", "fabro@example.com")
                .expect("signature should be valid");
            let parents = repository
                .head()
                .ok()
                .and_then(|head| head.target())
                .map(|oid| {
                    repository
                        .find_commit(oid)
                        .expect("parent commit should exist")
                });
            let parent_refs = parents.iter().collect::<Vec<_>>();
            repository
                .commit(
                    Some("refs/heads/main"),
                    &signature,
                    &signature,
                    message,
                    &tree,
                    &parent_refs,
                )
                .expect("commit should be created")
        }

        let temp = tempfile::tempdir().expect("temp directory should be created");
        let origin_path = temp.path().join("origin.git");
        let checkout = temp.path().join("checkout");
        let origin =
            git2::Repository::init_bare(&origin_path).expect("bare origin should be initialized");
        let repository = git2::Repository::init(&checkout).expect("checkout should be initialized");
        repository
            .set_head("refs/heads/main")
            .expect("main should be selected");
        write_file(
            &checkout.join("workflow.fabro"),
            "digraph Root { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }",
        );
        let first_commit = commit_all(&repository, "initial");
        let mut remote = repository
            .remote(
                "origin",
                origin_path.to_str().expect("origin path should be UTF-8"),
            )
            .expect("origin should be configured");
        remote
            .push(&["refs/heads/main:refs/heads/main"], None)
            .expect("initial commit should be pushed");
        drop(remote);
        write_file(&checkout.join("README.md"), "ahead\n");
        let ahead_commit = commit_all(&repository, "ahead");
        assert_ne!(first_commit, ahead_commit);

        collect_graph(&checkout, &checkout.join("workflow.fabro"))
            .expect("working tree should collect");

        let origin_commit = origin
            .find_reference("refs/heads/main")
            .expect("origin main should exist")
            .target()
            .expect("origin main should point to a commit");
        assert_eq!(origin_commit, first_commit);
    }
}
