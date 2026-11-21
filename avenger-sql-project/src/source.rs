//! The file source: the object store of a project's files and the URL of its root.

use std::sync::Arc;

use bytes::Bytes;
use datafusion::{
    object_store::{memory::InMemory, path::Path, ObjectStore, ObjectStoreExt, PutPayload},
    prelude::SessionContext,
};
use url::Url;

/// The files of a project: an object store and the URL of the project's root in it, such as
/// `file:///home/me/sales/` for a directory or `memory://sales/` for embedded files.
///
/// A path of a file is relative to the root, with `/` between its names, as
/// `data/files/sales.csv`. The key of the file in the store is the path of the root's URL and
/// the file's path, as DataFusion reads it from the file's URL, so a session in which
/// [`register`](FileSource::register) puts the store reads the file at the root's URL joined with
/// its path. The names of the keys are the names of the files, as `Path::parse` keeps them, and
/// not `Path::from`, which encodes characters such as `[`, `?` and `*`.
#[derive(Clone, Debug)]
pub struct FileSource {
    store: Arc<dyn ObjectStore>,
    base: Url,
    /// The key of the root in the store, the path of `base`.
    root: Path,
}

/// Why a file source cannot be built, or a file of it cannot be read or listed.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// A path that is not relative to the root, or that has an empty name, `.` or `..`.
    #[error("'{0}' is not a path relative to the project root")]
    Path(String),
    /// A base URL or a path on disk that is not a directory.
    #[error("'{0}' is not a directory or the URL of one")]
    Base(String),
    #[error("no file '{0}' in the project")]
    NotFound(String),
    #[error("the file '{0}' is not UTF-8 text")]
    NotText(String),
    #[error(transparent)]
    Store(datafusion::object_store::Error),
}

impl FileSource {
    /// The files of `store` under `base`, the URL of the project's root, whose path ends with
    /// `/`. The store holds each file at the path of `base` and the file's path.
    pub fn new(store: Arc<dyn ObjectStore>, base: Url) -> Result<FileSource, SourceError> {
        if base.cannot_be_a_base() || !base.path().ends_with('/') {
            return Err(SourceError::Base(base.to_string()));
        }
        let root =
            Path::from_url_path(base.path()).map_err(|_| SourceError::Base(base.to_string()))?;
        Ok(FileSource { store, base, root })
    }

    /// The files of the directory `root` on disk, at its `file:` URL, through a
    /// `LocalFileSystem` whose keys are absolute paths, so that a session reads the absolute
    /// paths and `file:` URLs of other files through the same store. The root is the canonical
    /// path of the directory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn directory(root: impl AsRef<std::path::Path>) -> Result<FileSource, SourceError> {
        let root = root.as_ref();
        let path = || root.display().to_string();
        let canonical = std::fs::canonicalize(root).map_err(|_| SourceError::Base(path()))?;
        let base = Url::from_directory_path(&canonical).map_err(|()| SourceError::Base(path()))?;
        let store = datafusion::object_store::local::LocalFileSystem::new();
        FileSource::new(Arc::new(store), base)
    }

    /// The files `files`, each a path relative to the root and its bytes, in an `InMemory`
    /// store at `memory://{name}/`, where `name` is a host name that the caller picks, such as
    /// `sales`.
    pub async fn embedded<P: AsRef<str>, B: Into<PutPayload>>(
        name: &str,
        files: impl IntoIterator<Item = (P, B)>,
    ) -> Result<FileSource, SourceError> {
        let base = format!("memory://{name}/");
        let url = Url::parse(&base)
            .ok()
            .filter(|url| url.host_str() == Some(name))
            .ok_or(SourceError::Base(base))?;
        let source = FileSource::new(Arc::new(InMemory::new()), url)?;
        for (path, bytes) in files {
            let key = source.key(path.as_ref())?;
            (source.store.put(&key, bytes.into()).await).map_err(SourceError::Store)?;
        }
        Ok(source)
    }

    pub fn store(&self) -> &Arc<dyn ObjectStore> {
        &self.store
    }

    /// The URL of the project's root, which ends with `/`, against which
    /// `avenger_sql::data::file_table` resolves the URIs of data scripts.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// The URL of the file at `path`.
    pub fn url(&self, path: &str) -> Result<Url, SourceError> {
        let mut url = self.base.clone();
        (url.path_segments_mut())
            .map_err(|()| SourceError::Base(self.base.to_string()))?
            .pop_if_empty()
            .extend(segments(path)?);
        Ok(url)
    }

    /// Puts the store in the session `ctx` at the scheme and the host of the root's URL, so that
    /// the session reads the project's files at their URLs.
    pub fn register(&self, ctx: &SessionContext) {
        ctx.register_object_store(&self.base, self.store.clone());
    }

    /// The bytes of the file at `path`.
    pub async fn read(&self, path: &str) -> Result<Bytes, SourceError> {
        let key = self.key(path)?;
        let not_found = |error| match error {
            datafusion::object_store::Error::NotFound { .. } => SourceError::NotFound(path.into()),
            error => SourceError::Store(error),
        };
        let result = self.store.get(&key).await.map_err(not_found)?;
        result.bytes().await.map_err(not_found)
    }

    /// The text of the file at `path`.
    pub async fn read_text(&self, path: &str) -> Result<String, SourceError> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes.into()).map_err(|_| SourceError::NotText(path.into()))
    }

    /// The paths of the files under the directory `directory`, and under its directories in
    /// turn, in the order of their paths. The empty path is the root. A hidden file or
    /// directory, whose name starts with `.`, such as `.git` or `.DS_Store`, is not listed, and
    /// the listing does not descend into a hidden directory.
    pub async fn list(&self, directory: &str) -> Result<Vec<String>, SourceError> {
        let directory = directory.trim_end_matches('/');
        let mut pending = vec![match directory {
            "" => self.root.clone(),
            directory => self.key(directory)?,
        }];
        let visible = |key: &Path| !key.filename().is_some_and(|name| name.starts_with('.'));
        let mut paths = Vec::new();
        // One level at a time, so that a hidden directory is never read.
        while let Some(prefix) = pending.pop() {
            let prefix = (!prefix.as_ref().is_empty()).then_some(prefix);
            let level = (self.store.list_with_delimiter(prefix.as_ref()).await)
                .map_err(SourceError::Store)?;
            pending.extend(level.common_prefixes.into_iter().filter(visible));
            for meta in level
                .objects
                .into_iter()
                .filter(|meta| visible(&meta.location))
            {
                let key = meta.location.as_ref();
                let path = match self.root.as_ref() {
                    "" => key,
                    root => (key.strip_prefix(root))
                        .and_then(|key| key.strip_prefix('/'))
                        .unwrap_or(key),
                };
                paths.push(path.to_string());
            }
        }
        paths.sort();
        Ok(paths)
    }

    /// The key of the file at `path` in the store.
    fn key(&self, path: &str) -> Result<Path, SourceError> {
        let names = segments(path)?.collect::<Vec<_>>().join("/");
        let key = match self.root.as_ref() {
            "" => names,
            root => format!("{root}/{names}"),
        };
        Path::parse(key).map_err(|_| SourceError::Path(path.into()))
    }
}

/// The names of the relative path `path`, which has no empty name, `.` or `..`.
fn segments(path: &str) -> Result<impl Iterator<Item = &str>, SourceError> {
    let valid = !path.is_empty()
        && (path.split('/')).all(|name| !matches!(name, "" | "." | ".."))
        && !path.contains('\\');
    match valid {
        true => Ok(path.split('/')),
        false => Err(SourceError::Path(path.into())),
    }
}
