//! The files of a project by their roles.

use avenger_sql::catalog::same_path;

use crate::{
    manifest::{Manifest, ManifestError, Role, RoleConflict, MANIFEST},
    source::{FileSource, Listing, Skipped, SourceError},
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
    skipped: Vec<Skipped>,
}

/// A module of a project: a `.av.sqlm` file under the modules root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleFile {
    pub path: String,
    /// The module path that `IMPORT MODULE` names: `a/b.av.sqlm` under the root is `["a", "b"]`.
    pub names: Vec<String>,
}

/// Two modules of one module path: a module path ignores case, as `IMPORT MODULE` does, so
/// `Geo.av.sqlm` and `geo.av.sqlm` are the same module.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the modules '{path}' and '{other}' have the same module path, which ignores case")]
pub struct ModuleConflict {
    pub path: String,
    /// The module whose path comes first.
    pub other: String,
}

/// Why a project does not open or load.
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
    /// The modules whose module paths are those of earlier modules, in the order of their paths.
    #[error("{}", (.0.iter()).map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
    Modules(Vec<ModuleConflict>),
    /// A session that the project can't load into, such as one whose query planner avenger-sql
    /// can't replace.
    #[error(transparent)]
    Session(datafusion::common::DataFusionError),
    /// A path to reload that is neither a data script nor a module of the project.
    #[error("'{0}' is neither a data script nor a module of the project")]
    NotScript(String),
}

impl ProjectFiles {
    /// The files of `listing` of the project of `manifest` and `source`, paths relative to its
    /// root, by the roles that the manifest gives them. The error is [`ProjectError::Roles`],
    /// the files that two roles match, or [`ProjectError::Modules`], the modules of one module
    /// path.
    pub fn new(
        manifest: Manifest,
        source: FileSource,
        listing: Listing,
    ) -> Result<ProjectFiles, ProjectError> {
        let Listing {
            paths: mut files,
            skipped,
        } = listing;
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
            skipped,
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
        if !conflicts.is_empty() {
            return Err(ProjectError::Roles(conflicts));
        }
        let mut conflicts = Vec::new();
        for (index, module) in project.modules.iter().enumerate() {
            let earlier = &project.modules[..index];
            if let Some(other) = earlier.iter().find(|m| same_path(&m.names, &module.names)) {
                conflicts.push(ModuleConflict {
                    path: module.path.clone(),
                    other: other.path.clone(),
                });
            }
        }
        match conflicts.is_empty() {
            true => Ok(project),
            false => Err(ProjectError::Modules(conflicts)),
        }
    }

    /// Opens the project of `source`: reads its manifest at the root and lists its files. A
    /// directory of the project that does not list is in [`skipped`](ProjectFiles::skipped).
    pub async fn open(source: FileSource) -> Result<ProjectFiles, ProjectError> {
        let text = match source.read_text(MANIFEST).await {
            Err(SourceError::NotFound(_)) => return Err(ProjectError::NoManifest),
            text => text?,
        };
        let manifest = Manifest::parse(&text)?;
        let files = source.list("").await?;
        ProjectFiles::new(manifest, source, files)
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

    /// The module at the module path `names`, which ignores case, as `IMPORT MODULE` does.
    pub fn module(&self, names: &[String]) -> Option<&ModuleFile> {
        (self.modules.iter()).find(|module| same_path(&module.names, names))
    }

    /// The `.css` files that are not data, and the entry stylesheet when the project has it.
    pub fn stylesheets(&self) -> &[String] {
        &self.stylesheets
    }

    /// The directories of the project that the listing of its files skipped, because the file
    /// source fails to list them, in the order of their paths. The files under them are not in
    /// [`files`](ProjectFiles::files).
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }
}

/// The root of the project on disk that holds `path`, a file or a directory: the nearest
/// directory, from `path` up, that holds the manifest. A path that does not exist, such as that
/// of a file to make, starts from the nearest directory above it that exists.
#[cfg(not(target_arch = "wasm32"))]
pub fn find_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path = (path.ancestors()).find_map(|path| match path.as_os_str().is_empty() {
        true => std::fs::canonicalize(".").ok(),
        false => std::fs::canonicalize(path).ok(),
    })?;
    (path.ancestors())
        .find(|directory| directory.join(MANIFEST).is_file())
        .map(std::path::Path::to_path_buf)
}
