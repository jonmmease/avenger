//! Checks of sources whose time to live has passed.

use avenger_common::time::Instant;
use datafusion::{common::Result, datasource::listing::ListingTable, execution::SessionState};
use futures::TryStreamExt;

use crate::store::{ttl_of, ResultStore};

/// What a check of a listing table records of one file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FileMeta {
    location: String,
    size: u64,
    modified: String,
    e_tag: Option<String>,
}

/// Check each of the sources `sources`, by the addresses of their providers, whose time to live
/// has passed: the shortest that the options of the source's names set. A listing table gets a new version when its files' metadata changed, or at its
/// first check, which has no metadata to compare and can follow results that a run kept before
/// the source had a time to live. A source of another kind gets a new version at each check. True
/// when a source has a new version.
pub(crate) async fn check(store: &ResultStore, sources: &[usize]) -> Result<bool> {
    let now = Instant::now();
    let due: Vec<_> = {
        let state = store.state();
        (sources.iter())
            .filter_map(|address| {
                let source = state.sources.get(address)?;
                let ttl = ttl_of(&state.options, source)?;
                let expired =
                    (source.checked).is_none_or(|checked| now.duration_since(checked) >= ttl);
                let session = source.names.first()?.1.clone();
                expired.then(|| (*address, source.provider.clone(), session))
            })
            .collect()
    };
    let mut changed = false;
    for (address, provider, session) in due {
        let files = match provider.downcast_ref::<ListingTable>() {
            Some(listing) => Some(list(listing, &session).await?),
            None => None,
        };
        let mut state = store.state();
        let Some(source) = state.sources.get_mut(&address) else {
            continue;
        };
        source.checked = Some(now);
        let new = match files {
            Some(files) => source
                .files
                .replace(files.clone())
                .is_none_or(|recorded| recorded != files),
            None => true,
        };
        if new {
            source.version += 1;
            changed = true;
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
