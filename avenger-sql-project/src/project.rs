//! The files of a project by their roles.

use crate::{
    manifest::{Manifest, ManifestError, Role, RoleConflict, MANIFEST},
    source::{FileSource, SourceError},
};

/// The files of a project: its manifest, its file source, and the paths of its files by role,
/// each in the order of the paths.
#[derive(Clone, Debug)]
pub struct ProjectFiles {
    manifest: Manifest,
    source: FileSource,
    files: Vec<String>,
    data_scripts: Vec<String>,
    chart_scripts: Vec<String>,
    modules: Vec<ModuleFile>,
    stylesheets: Vec<String>,
}

/// A module of a project: a `.sqlm` file under the modules root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleFile {
    pub path: String,
    /// The module path that `IMPORT MODULE` names: `a/b.sqlm` under the root is `["a", "b"]`.
    pub names: Vec<String>,
}

/// Why a project does not open.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("the project has no {MANIFEST} at its root")]
    NoManifest,
    /// A path on disk with no manifest in its directory or the directories above it.
    #[cfg(not(target_arch = "wasm32"))]
    #[error("no {MANIFEST} in '{}' or the directories above it", .0.display())]
    NotInProject(std::path::PathBuf),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The files that the patterns of two roles match, in the order of their paths.
    #[error("{}", (.0.iter()).map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
    Roles(Vec<RoleConflict>),
}

impl ProjectFiles {
    /// The files `files` of the project of `manifest` and `source`, paths relative to its root,
    /// by the roles that the manifest gives them, or the files that two roles match.
    pub fn new(
        manifest: Manifest,
        source: FileSource,
        mut files: Vec<String>,
    ) -> Result<ProjectFiles, Vec<RoleConflict>> {
        files.sort();
        files.dedup();
        let mut project = ProjectFiles {
            manifest,
            source,
            files: Vec::new(),
            data_scripts: Vec::new(),
            chart_scripts: Vec::new(),
            modules: Vec::new(),
            stylesheets: Vec::new(),
        };
        let mut conflicts = Vec::new();
        for path in &files {
            match project.manifest.role(path) {
                Ok(Some(Role::DataScript)) => project.data_scripts.push(path.clone()),
                Ok(Some(Role::ChartScript)) => project.chart_scripts.push(path.clone()),
                Ok(Some(Role::Module(names))) => project.modules.push(ModuleFile {
                    path: path.clone(),
                    names,
                }),
                Ok(Some(Role::Stylesheet)) => project.stylesheets.push(path.clone()),
                Ok(None) => {}
                Err(conflict) => conflicts.push(conflict),
            }
        }
        project.files = files;
        match conflicts.is_empty() {
            true => Ok(project),
            false => Err(conflicts),
        }
    }

    /// Opens the project of `source`: reads its manifest at the root and lists its files.
    pub async fn open(source: FileSource) -> Result<ProjectFiles, ProjectError> {
        let text = match source.read_text(MANIFEST).await {
            Err(SourceError::NotFound(_)) => return Err(ProjectError::NoManifest),
            text => text?,
        };
        let manifest = Manifest::parse(&text)?;
        let files = source.list("").await?;
        ProjectFiles::new(manifest, source, files).map_err(ProjectError::Roles)
    }

    /// Opens the project on disk that holds `path`, a file or a directory: the project of the
    /// nearest directory, from `path` up, whose manifest is in it.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn find(path: impl AsRef<std::path::Path>) -> Result<ProjectFiles, ProjectError> {
        let path = path.as_ref();
        let root = find_root(path).ok_or_else(|| ProjectError::NotInProject(path.into()))?;
        ProjectFiles::open(FileSource::directory(root)?).await
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn source(&self) -> &FileSource {
        &self.source
    }

    /// Every file of the project, with a role or without one, such as the manifest and the
    /// data files.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn data_scripts(&self) -> &[String] {
        &self.data_scripts
    }

    pub fn chart_scripts(&self) -> &[String] {
        &self.chart_scripts
    }

    pub fn modules(&self) -> &[ModuleFile] {
        &self.modules
    }

    /// The `.css` files, and the entry stylesheet when the project has it.
    pub fn stylesheets(&self) -> &[String] {
        &self.stylesheets
    }
}

/// The root of the project on disk that holds `path`, a file or a directory: the nearest
/// directory, from `path` up, that holds the manifest.
#[cfg(not(target_arch = "wasm32"))]
pub fn find_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path = std::fs::canonicalize(path).ok()?;
    (path.ancestors())
        .find(|directory| directory.join(MANIFEST).is_file())
        .map(std::path::Path::to_path_buf)
}
