//! The file source: the object store of a project's files and the URL of its root.

use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use bytes::Bytes;
use datafusion::{
    object_store::{memory::InMemory, path::Path, ObjectStore, ObjectStoreExt, PutPayload},
    prelude::SessionContext,
};
use url::Url;

use crate::path::relative;

/// The files of a project: an object store and the URL of the project's root in it, such as
/// `file:///home/me/sales/` for a directory or `memory://sales/` for embedded files.
///
/// A path of a file is relative to the root, with `/` between its names, as
/// `data/files/sales.csv`, and reads without its `.` names and its empty names. The key of the
/// file in the store is the path of the root's URL and the file's path, as DataFusion reads it
/// from the file's URL, so a session in which
/// [`register`](FileSource::register) puts the store reads the file at the root's URL joined with
/// its path. The names of the keys are the names of the files, as `Path::parse` keeps them, and
/// not `Path::from`, which encodes characters such as `[`, `?` and `*`.
#[derive(Clone, Debug)]
pub struct FileSource {
    store: Arc<dyn ObjectStore>,
    base: Url,
    /// The key of the root in the store, the path of `base`.
    root: Path,
    /// The canonical path of the root on disk, for the files of a directory on disk, through
    /// which a listing finds the directories that it already reads.
    #[cfg(not(target_arch = "wasm32"))]
    disk: Option<std::path::PathBuf>,
}

/// The files under a directory of a file source, as [`FileSource::list`] gives them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The paths of the files, in their order.
    pub paths: Vec<String>,
    /// The directories under the listed directory that do not list, in the order of their
    /// paths. The listing leaves out the files under them.
    pub skipped: Vec<Skipped>,
}

/// A directory that a listing leaves out because the store fails to list it, such as a
/// directory that can't be read, or one that holds a name that the store's paths can't hold,
/// or, on disk, because it is a link to a directory that the listing already reads.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("cannot list '{path}': {error}")]
pub struct Skipped {
    /// The path of the directory, relative to the root.
    pub path: String,
    /// Why the directory does not list: the store's error.
    pub error: String,
}

/// Why a file source cannot be built, or a file of it cannot be read or listed.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// A path that is not relative to the root, which starts with `/`, holds `\`, or has a `..`
    /// name, or the root where the path of a file is due.
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
        Ok(FileSource {
            store,
            base,
            root,
            #[cfg(not(target_arch = "wasm32"))]
            disk: None,
        })
    }

    /// The files of the directory `root` on disk, at its `file:` URL, through a
    /// `LocalFileSystem` whose keys are absolute paths, so that a session reads the absolute
    /// paths and `file:` URLs of other files through the same store. The root is the canonical
    /// path of the directory, and a path that is not a directory is an error.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn directory(root: impl AsRef<std::path::Path>) -> Result<FileSource, SourceError> {
        let root = root.as_ref();
        let path = || root.display().to_string();
        let canonical = (std::fs::canonicalize(root).ok())
            .filter(|canonical| canonical.is_dir())
            .ok_or_else(|| SourceError::Base(path()))?;
        let base = Url::from_directory_path(&canonical).map_err(|()| SourceError::Base(path()))?;
        let store = datafusion::object_store::local::LocalFileSystem::new();
        let source = FileSource::new(Arc::new(store), base)?;
        Ok(FileSource {
            disk: Some(canonical),
            ..source
        })
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
        let path = entry(path)?;
        let mut url = self.base.clone();
        (url.path_segments_mut())
            .map_err(|()| SourceError::Base(self.base.to_string()))?
            .pop_if_empty()
            .extend(path.split('/'));
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

    /// The files under the directory `directory`, and under its directories in turn. The empty
    /// path is the root. A hidden file or directory, whose name starts with `.`, such as `.git`
    /// or `.DS_Store`, is not listed, and the listing does not descend into a hidden directory.
    /// A directory under `directory` that the store fails to list is skipped, with its error,
    /// and a failure to list `directory` itself is the error. In a directory on disk, a
    /// directory whose canonical path the listing already reads, such as a link to a directory
    /// above it or to one that the listing reaches before it, is skipped too, so the listing
    /// reads each directory once, and a link to a directory outside the listed one is followed.
    pub async fn list(&self, directory: &str) -> Result<Listing, SourceError> {
        let start = match relative(directory).as_deref() {
            None => return Err(SourceError::Path(directory.into())),
            Some("") => self.root.clone(),
            Some(directory) => self.key(directory)?,
        };
        let visible = |key: &Path| !key.filename().is_some_and(|name| name.starts_with('.'));
        let mut listing = Listing::default();
        // The canonical paths of the directories that the listing reads, on disk.
        let mut read: BTreeSet<std::path::PathBuf> = self.canonical(&start).into_iter().collect();
        // One level at a time, so that a hidden directory is never read, and the levels nearer
        // the listed directory first, so that a directory is read at its shortest path. Each
        // prefix is a directory and whether it is under the listed directory.
        let mut pending = VecDeque::from([(start, false)]);
        while let Some((prefix, under)) = pending.pop_front() {
            let key = (!prefix.as_ref().is_empty()).then_some(&prefix);
            let level = match self.store.list_with_delimiter(key).await {
                Ok(level) => level,
                Err(error) if under => {
                    listing.skipped.push(Skipped {
                        path: self.path_of(&prefix),
                        error: error.to_string(),
                    });
                    continue;
                }
                Err(error) => return Err(SourceError::Store(error)),
            };
            for directory in level.common_prefixes.into_iter().filter(visible) {
                let canonical = self.canonical(&directory);
                if canonical.is_none_or(|canonical| read.insert(canonical)) {
                    pending.push_back((directory, true));
                } else {
                    listing.skipped.push(Skipped {
                        path: self.path_of(&directory),
                        error: "links to a folder that the listing already reads".into(),
                    });
                }
            }
            let files = level.objects.into_iter().map(|meta| meta.location);
            listing
                .paths
                .extend(files.filter(visible).map(|key| self.path_of(&key)));
        }
        listing.paths.sort();
        listing.skipped.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(listing)
    }

    /// The canonical path on disk of the directory at the key `key`, in the files of a
    /// directory on disk, and `None` in other stores or when the path does not resolve.
    fn canonical(&self, key: &Path) -> Option<std::path::PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let disk = self.disk.as_ref()?;
            std::fs::canonicalize(disk.join(self.path_of(key))).ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = key;
            None
        }
    }

    /// The path, relative to the root, of the key `key` of the store under the root.
    /// The root itself is the empty path.
    fn path_of(&self, key: &Path) -> String {
        let key = key.as_ref();
        let path = match (self.root.as_ref(), key.strip_prefix(self.root.as_ref())) {
            ("", _) => key,
            (_, Some("")) => "",
            (_, Some(rest)) => rest.strip_prefix('/').unwrap_or(key),
            (_, None) => key,
        };
        path.to_string()
    }

    /// The key of the file or the directory at `path` in the store.
    fn key(&self, path: &str) -> Result<Path, SourceError> {
        let names = entry(path)?;
        let key = match self.root.as_ref() {
            "" => names,
            root => format!("{root}/{names}"),
        };
        Path::parse(key).map_err(|_| SourceError::Path(path.into()))
    }
}

/// The path `path` of a file or a directory, relative to the root, as [`relative`] reads it,
/// which is not the root itself.
fn entry(path: &str) -> Result<String, SourceError> {
    relative(path)
        .filter(|relative| !relative.is_empty())
        .ok_or_else(|| SourceError::Path(path.into()))
}
