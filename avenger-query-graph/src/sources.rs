//! Checks of sources whose time to live has passed.

use avenger_common::time::Instant;
use datafusion::{common::Result, datasource::listing::ListingTable, execution::SessionState};
use futures::TryStreamExt;

use crate::store::ResultStore;

/// What a check of a listing table records of one file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FileMeta {
    location: String,
    size: u64,
    modified: String,
    e_tag: Option<String>,
}

/// Check each of the sources `names` whose time to live has passed. A listing table whose files'
/// metadata changed gets a new version, and a source of another kind gets one at each check.
/// True when a source has a new version.
pub(crate) async fn check(store: &ResultStore, names: &[String]) -> Result<bool> {
    let now = Instant::now();
    let due: Vec<_> = {
        let state = store.state();
        (names.iter())
            .filter_map(|name| {
                let source = state.sources.get(name)?;
                let ttl = source.options.ttl?;
                let expired =
                    (source.checked).is_none_or(|checked| now.duration_since(checked) >= ttl);
                expired.then(|| (name.clone(), source.checker.clone()))
            })
            .collect()
    };
    let mut changed = false;
    for (name, checker) in due {
        let files = match &checker {
            Some((provider, session)) => match provider.downcast_ref::<ListingTable>() {
                Some(listing) => Some(list(listing, session).await?),
                None => None,
            },
            None => None,
        };
        let mut state = store.state();
        let source = state.sources.entry(name).or_default();
        source.checked = Some(now);
        match files {
            Some(files) => {
                if source
                    .files
                    .as_ref()
                    .is_some_and(|recorded| *recorded != files)
                {
                    source.version += 1;
                    changed = true;
                }
                source.files = Some(files);
            }
            None => {
                source.version += 1;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// The metadata of each file of `listing`, in order.
async fn list(listing: &ListingTable, session: &SessionState) -> Result<Vec<FileMeta>> {
    let extension = &listing.options().file_extension;
    let mut files = Vec::new();
    for url in listing.table_paths() {
        let store = session.runtime_env().object_store(url)?;
        let listed: Vec<_> = url
            .list_all_files(session, store.as_ref(), extension)
            .await?
            .try_collect()
            .await?;
        files.extend(listed.into_iter().map(|meta| FileMeta {
            location: meta.location.to_string(),
            size: meta.size,
            modified: meta.last_modified.to_rfc3339(),
            e_tag: meta.e_tag,
        }));
    }
    files.sort();
    Ok(files)
}
