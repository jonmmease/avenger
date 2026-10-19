//! A disk cache of fetched images that honors HTTP caching headers.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::fetcher::{FetchResponse, Validators};

/// How long a response without expiry headers stays fresh: 7 days, the least that
/// OpenStreetMap's tile usage policy asks clients to cache tiles.
const DEFAULT_FRESHNESS: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How old a temporary file or a file without metadata must be before the cache removes it,
/// so a write in progress in another process survives.
const ABANDONED: Duration = Duration::from_secs(60 * 60);
const META_SUFFIX: &str = "~meta.json";
const TEMP_MARKER: &str = "~tmp";

/// Where an [`crate::ImageCache`] keeps the responses it fetches, and how many bytes it keeps.
#[derive(Debug, Clone)]
pub struct DiskCacheOptions {
    /// The directory of the cache's files.
    pub dir: PathBuf,
    /// The bytes of files that the cache keeps, 50 MB by default. The least recently used
    /// responses go first.
    pub budget: u64,
}

impl DiskCacheOptions {
    /// A disk cache in `dir` with the default budget.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            budget: 50_000_000,
        }
    }
}

/// A stored response's validators, freshness, and use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Meta {
    pub(crate) url: String,
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
    pub(crate) cache_control: Option<String>,
    /// The `Expires` header, which sets when the response goes stale without a `max-age`.
    #[serde(default)]
    pub(crate) expires: Option<String>,
    /// When the response goes stale, in milliseconds since the Unix epoch.
    pub(crate) fresh_until: i64,
    /// When the response was last read or written, in milliseconds since the Unix epoch.
    pub(crate) last_access: i64,
}

impl Meta {
    pub(crate) fn is_fresh(&self) -> bool {
        now_ms() < self.fresh_until
    }

    /// Whether a stale response must wait for its revalidation before it draws.
    pub(crate) fn must_revalidate(&self) -> bool {
        let control = CacheControl::parse(self.cache_control.as_deref());
        control.must_revalidate || control.no_cache
    }

    pub(crate) fn validators(&self) -> Validators {
        Validators {
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
        }
    }
}

/// A stored response's body and metadata.
pub(crate) struct Stored {
    pub(crate) body: Vec<u8>,
    pub(crate) meta: Meta,
}

/// Responses stored as files named from their URLs, under a byte budget. Each response's body is
/// a file, and a JSON file alongside it holds its metadata. Every file is written under a
/// temporary name and renamed, so a partial write is never read.
pub(crate) struct DiskCache {
    dir: PathBuf,
    budget: u64,
    /// The stored responses' sizes and last uses, read from the directory when a response is
    /// first read or written.
    index: Mutex<Option<Index>>,
}

#[derive(Default)]
struct Index {
    /// Each stored body's path, with the bytes of its two files and its last access.
    entries: HashMap<PathBuf, (u64, i64)>,
    bytes: u64,
}

impl DiskCache {
    pub(crate) fn new(options: &DiskCacheOptions) -> Self {
        Self {
            dir: options.dir.clone(),
            budget: options.budget,
            index: Mutex::new(None),
        }
    }

    /// The stored response for `url`, which counts as a use.
    pub(crate) fn load(&self, url: &str) -> Option<Stored> {
        let path = self.path_for(url)?;
        let mut meta: Meta = serde_json::from_slice(&fs::read(meta_path(&path)).ok()?).ok()?;
        if meta.url != url {
            return None;
        }
        let body = fs::read(&path).ok()?;
        meta.last_access = now_ms();
        self.write_meta(&path, &meta, body.len() as u64);
        Some(Stored { body, meta })
    }

    /// Store a successful response, unless its headers forbid it.
    pub(crate) fn store(&self, url: &str, response: &FetchResponse) {
        let control = CacheControl::parse(response.cache_control.as_deref());
        if control.no_store {
            return;
        }
        let Some(path) = self.path_for(url) else {
            return;
        };
        let now = now_ms();
        let meta = Meta {
            url: url.to_string(),
            etag: response.validators.etag.clone(),
            last_modified: response.validators.last_modified.clone(),
            cache_control: response.cache_control.clone(),
            expires: response.expires.clone(),
            fresh_until: fresh_until(&control, response, now),
            last_access: now,
        };
        if path
            .parent()
            .is_some_and(|dir| fs::create_dir_all(dir).is_ok())
            && write_atomic(&path, &response.body).is_ok()
        {
            self.write_meta(&path, &meta, response.body.len() as u64);
        }
    }

    /// Refresh a stored response's metadata from a 304 response to its revalidation. Each
    /// header that the 304 response carries replaces the stored one, and the others stay, as
    /// RFC 9111 §3.2 updates stored headers. A 304 response that forbids storing removes the
    /// stored response.
    pub(crate) fn refresh(&self, stored: &Meta, response: &FetchResponse) {
        let Some(path) = self.path_for(&stored.url) else {
            return;
        };
        let mut meta = stored.clone();
        let validators = &response.validators;
        meta.etag = validators.etag.clone().or(meta.etag);
        meta.last_modified = validators.last_modified.clone().or(meta.last_modified);
        meta.cache_control = response.cache_control.clone().or(meta.cache_control);
        meta.expires = response.expires.clone().or(meta.expires);
        let control = CacheControl::parse(meta.cache_control.as_deref());
        if control.no_store {
            self.remove_at(&path);
            return;
        }
        let Ok(size) = fs::metadata(&path).map(|metadata| metadata.len()) else {
            return;
        };
        let headers = FetchResponse {
            cache_control: meta.cache_control.clone(),
            expires: meta.expires.clone(),
            age: response.age.clone(),
            date: response.date.clone(),
            ..Default::default()
        };
        let now = now_ms();
        meta.fresh_until = fresh_until(&control, &headers, now);
        meta.last_access = now;
        self.write_meta(&path, &meta, size);
    }

    /// Remove the stored response for `url`, when there is one.
    pub(crate) fn remove(&self, url: &str) {
        if let Some(path) = self.path_for(url) {
            self.remove_at(&path);
        }
    }

    /// Remove the response at `path`, its metadata, and its index entry.
    fn remove_at(&self, path: &Path) {
        let _ = fs::remove_file(meta_path(path));
        let _ = fs::remove_file(path);
        let mut index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = index.as_mut() {
            if let Some((size, _)) = index.entries.remove(path) {
                index.bytes -= size;
            }
        }
    }

    /// Write `meta` beside the body at `path`, record the pair in the index, and evict the least
    /// recently used responses beyond the budget.
    fn write_meta(&self, path: &Path, meta: &Meta, body_size: u64) {
        let Ok(json) = serde_json::to_vec(meta) else {
            return;
        };
        if write_atomic(&meta_path(path), &json).is_err() {
            return;
        }
        let mut index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        let index = index.get_or_insert_with(|| scan(&self.dir));
        let size = body_size + json.len() as u64;
        if let Some((old, _)) = index
            .entries
            .insert(path.to_path_buf(), (size, meta.last_access))
        {
            index.bytes -= old;
        }
        index.bytes += size;
        if index.bytes > self.budget {
            let mut entries: Vec<_> = index
                .entries
                .iter()
                .map(|(path, &(size, access))| (access, size, path.clone()))
                .collect();
            entries.sort_unstable_by_key(|(access, ..)| *access);
            for (_, size, path) in entries {
                if index.bytes <= self.budget {
                    break;
                }
                let _ = fs::remove_file(meta_path(&path));
                let _ = fs::remove_file(&path);
                index.entries.remove(&path);
                index.bytes -= size;
            }
        }
    }

    /// The path of `url`'s body: the directory, then the URL's scheme, host, and path. Characters
    /// outside letters, digits, `.`, `-`, and `_` become `_`. When that changes the URL, or the
    /// URL has a query, a hash of the whole URL follows `~` before the extension, so different
    /// URLs get different files.
    fn path_for(&self, url: &str) -> Option<PathBuf> {
        let (scheme, rest) = url.split_once("://")?;
        let (location, query) = match rest.find(['?', '#']) {
            Some(index) => (&rest[..index], true),
            None => (rest, false),
        };
        let mut changed = query;
        let mut segments: Vec<String> = Vec::new();
        for segment in std::iter::once(scheme).chain(location.split('/')) {
            let clean: String = segment
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let clean = if clean.is_empty() || clean == "." || clean == ".." {
                "_".to_string()
            } else {
                clean
            };
            changed |= clean != segment;
            segments.push(clean);
        }
        if changed {
            let last = segments.last_mut()?;
            let hash = format!("~{:016x}", fnv1a(url.as_bytes()));
            match last.rfind('.') {
                Some(dot) if dot > 0 => last.insert_str(dot, &hash),
                _ => last.push_str(&hash),
            }
        }
        let mut path = self.dir.clone();
        path.extend(segments);
        Some(path)
    }
}

/// The directives of a `Cache-Control` header that the cache follows.
#[derive(Debug, Default)]
struct CacheControl {
    max_age: Option<u64>,
    no_store: bool,
    no_cache: bool,
    must_revalidate: bool,
}

impl CacheControl {
    fn parse(header: Option<&str>) -> Self {
        let mut control = Self::default();
        for directive in header.unwrap_or_default().split(',') {
            let (name, value) = match directive.split_once('=') {
                Some((name, value)) => (name, Some(value.trim().trim_matches('"'))),
                None => (directive, None),
            };
            match name.trim().to_ascii_lowercase().as_str() {
                // The first max-age counts, and one that isn't a number makes the response
                // stale, as RFC 9111 §4.2.1 advises.
                "max-age" if control.max_age.is_none() => {
                    control.max_age = Some(value.and_then(delta_seconds).unwrap_or(0));
                }
                "no-store" => control.no_store = true,
                "no-cache" => control.no_cache = true,
                "must-revalidate" => control.must_revalidate = true,
                _ => {}
            }
        }
        control
    }
}

/// When a response that arrives at `now` goes stale, as RFC 9111 §4.2 computes it: never fresh
/// under `no-cache`, else once its age passes its freshness lifetime. The lifetime is its
/// `max-age`, else its `Expires` less its `Date`, else 7 days. Its age on arrival is the larger
/// of its `Age` and the time since its `Date`.
fn fresh_until(control: &CacheControl, response: &FetchResponse, now: i64) -> i64 {
    if control.no_cache {
        return now;
    }
    let date = response.date.as_deref().and_then(http_date);
    let lifetime = match (control.max_age, &response.expires) {
        (Some(max_age), _) => seconds_to_ms(max_age),
        // An Expires header that doesn't parse, such as "0", means already expired.
        (None, Some(expires)) => {
            http_date(expires).map_or(0, |expires| expires - date.unwrap_or(now))
        }
        (None, None) => DEFAULT_FRESHNESS.as_millis() as i64,
    };
    let age = response
        .age
        .as_deref()
        .and_then(delta_seconds)
        .map_or(0, seconds_to_ms);
    let apparent_age = date.map_or(0, |date| (now - date).max(0));
    now + lifetime - age.max(apparent_age)
}

/// How long a failed response's `Retry-After` header asks to wait, from `now`: a number of
/// seconds, or the time until a date. None for a header that's missing, invalid, or past.
pub(crate) fn retry_after(header: Option<&str>, now: i64) -> Option<Duration> {
    let header = header?;
    let ms = match delta_seconds(header) {
        Some(seconds) => seconds_to_ms(seconds),
        None => http_date(header)? - now,
    };
    (ms > 0).then(|| Duration::from_millis(ms as u64))
}

/// The seconds of an HTTP delta-seconds value, at most 2^31, as RFC 9111 §1.2.2 caps them.
fn delta_seconds(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(
        value
            .parse()
            .map_or(1 << 31, |seconds: u64| seconds.min(1 << 31)),
    )
}

fn seconds_to_ms(seconds: u64) -> i64 {
    seconds.min(1 << 31) as i64 * 1000
}

/// The milliseconds since the Unix epoch of an HTTP date.
fn http_date(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|date| date.timestamp_millis())
}

/// Read the stored responses' sizes and last uses from `dir`, and remove abandoned temporary
/// files and bodies without metadata.
fn scan(dir: &Path) -> Index {
    let mut index = Index::default();
    let mut bodies = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if metadata.is_dir() {
                stack.push(path);
            } else if name.contains(TEMP_MARKER) {
                if is_abandoned(&metadata) {
                    let _ = fs::remove_file(&path);
                }
            } else if let Some(body) = name.strip_suffix(META_SUFFIX) {
                let body = path.with_file_name(body);
                let meta: Option<Meta> = fs::read(&path)
                    .ok()
                    .and_then(|json| serde_json::from_slice(&json).ok());
                match (meta, fs::metadata(&body)) {
                    (Some(meta), Ok(body_metadata)) => {
                        let size = body_metadata.len() + metadata.len();
                        index.bytes += size;
                        index.entries.insert(body, (size, meta.last_access));
                    }
                    _ => {
                        let _ = fs::remove_file(&path);
                    }
                }
            } else {
                bodies.push((path, metadata));
            }
        }
    }
    for (path, metadata) in bodies {
        if !index.entries.contains_key(&path) && is_abandoned(&metadata) {
            let _ = fs::remove_file(&path);
        }
    }
    index
}

fn is_abandoned(metadata: &fs::Metadata) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > ABANDONED)
}

fn meta_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(META_SUFFIX);
    path.with_file_name(name)
}

/// Write `bytes` to a temporary file beside `path`, then rename it to `path`.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        "{TEMP_MARKER}{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = path.with_file_name(name);
    let written = fs::File::create(&temp).and_then(|mut file| file.write_all(bytes));
    let renamed = written.and_then(|()| fs::rename(&temp, path));
    if renamed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    renamed
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

/// The 64-bit FNV-1a hash, which stays the same across runs and Rust versions.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_name_readable_files_that_differ() {
        let cache = DiskCache::new(&DiskCacheOptions::new("/cache"));
        let path = |url| cache.path_for(url).unwrap();
        assert_eq!(
            path("https://tile.openstreetmap.org/3/4/2.png"),
            Path::new("/cache/https/tile.openstreetmap.org/3/4/2.png")
        );
        let queried = path("https://example.com/a/b.png?key=1");
        assert!(queried.starts_with("/cache/https/example.com/a"));
        assert_ne!(queried, path("https://example.com/a/b.png?key=2"));
        assert_ne!(
            path("https://example.com/a b.png"),
            path("https://example.com/a_b.png")
        );
        assert!(!path("https://example.com/../../etc/passwd")
            .components()
            .any(|component| component == std::path::Component::ParentDir));
        assert!(cache.path_for("data:image/png;base64,AAAA").is_none());
    }

    #[test]
    fn headers_set_freshness() {
        // When a response with `headers` that arrives 1,000 seconds after the epoch goes stale.
        let until = |headers: &[(&str, &str)]| {
            let header = |name: &str| {
                headers
                    .iter()
                    .find(|(header, _)| *header == name)
                    .map(|(_, value)| value.to_string())
            };
            let response = FetchResponse {
                status: 200,
                cache_control: header("cache-control"),
                expires: header("expires"),
                age: header("age"),
                date: header("date"),
                ..Default::default()
            };
            let control = CacheControl::parse(response.cache_control.as_deref());
            fresh_until(&control, &response, 1_000_000)
        };
        assert_eq!(until(&[("cache-control", "max-age=60")]), 1_060_000);
        let shared = [("cache-control", "public, max-age=60"), ("age", "20")];
        assert_eq!(until(&shared), 1_040_000);
        assert_eq!(until(&[("cache-control", "no-cache")]), 1_000_000);
        let expires = [("expires", "Thu, 01 Jan 1970 00:20:00 GMT")];
        assert_eq!(until(&expires), 1_200_000);
        assert_eq!(until(&[("expires", "0")]), 1_000_000);
        assert_eq!(until(&[]), 1_000_000 + 7 * 24 * 60 * 60 * 1000);
        // Expires counts from the response's Date, so a client clock 200 seconds behind the
        // server's leaves the response its 600 seconds.
        let behind = [
            ("date", "Thu, 01 Jan 1970 00:20:00 GMT"),
            ("expires", "Thu, 01 Jan 1970 00:30:00 GMT"),
        ];
        assert_eq!(until(&behind), 1_600_000);
        // A response sent 30 seconds ago arrives 30 seconds old, though no Age says so.
        let sent = [
            ("date", "Thu, 01 Jan 1970 00:16:10 GMT"),
            ("cache-control", "max-age=60"),
        ];
        assert_eq!(until(&sent), 1_030_000);
        // The first max-age counts, one that isn't a number makes the response stale, and a
        // longer one than 2^31 seconds is cut to it.
        assert_eq!(
            until(&[("cache-control", "max-age=60, max-age=10")]),
            1_060_000
        );
        assert_eq!(until(&[("cache-control", "max-age=soon")]), 1_000_000);
        let long = [("cache-control", "max-age=99999999999999999999")];
        assert_eq!(until(&long), 1_000_000 + (1i64 << 31) * 1000);
        assert!(CacheControl::parse(Some("No-Store")).no_store);
    }

    #[test]
    fn retry_after_waits_seconds_or_until_a_date() {
        let now = 1_000_000;
        assert_eq!(
            retry_after(Some("120"), now),
            Some(Duration::from_secs(120))
        );
        let date = Some("Thu, 01 Jan 1970 00:17:40 GMT");
        assert_eq!(retry_after(date, now), Some(Duration::from_secs(60)));
        for none in [
            None,
            Some("0"),
            Some("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some("soon"),
        ] {
            assert_eq!(retry_after(none, now), None, "{none:?}");
        }
    }

    /// A 304 response replaces only the caching headers that it carries, and one that forbids
    /// storing removes the stored response.
    #[test]
    fn a_304_refreshes_the_headers_it_carries() {
        let dir = std::env::temp_dir().join(format!("avenger-image-304-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&DiskCacheOptions::new(&dir));
        let url = "https://tiles.test/1/0/0.png";
        let response = |status: u16, [cache_control, expires]: [Option<&str>; 2]| FetchResponse {
            status,
            cache_control: cache_control.map(str::to_string),
            expires: expires.map(str::to_string),
            body: vec![1],
            ..Default::default()
        };
        // Store a response with `stored` headers, and refresh it with a 304 response that
        // carries `carried`. The stored response's metadata after the refresh.
        let refreshed = |stored, carried| {
            cache.store(url, &response(200, stored));
            let meta = cache.load(url).unwrap().meta;
            cache.refresh(&meta, &response(304, carried));
            cache.load(url).map(|stored| stored.meta)
        };
        let future = "Thu, 01 Jan 2099 00:00:00 GMT";

        // A bare 304 keeps the stored `Expires`, instead of making the response fresh for the
        // default 7 days.
        let meta = refreshed([None, Some("Thu, 01 Jan 1970 00:00:00 GMT")], [None, None]);
        assert!(!meta.unwrap().is_fresh());
        // A stored `no-cache` stays when the 304 brings only `Expires`, so the response still
        // revalidates.
        let meta = refreshed([Some("no-cache"), None], [None, Some(future)]).unwrap();
        assert_eq!(meta.expires.as_deref(), Some(future));
        assert!(!meta.is_fresh() && meta.must_revalidate());
        assert!(refreshed([Some("no-cache"), None], [Some("no-store"), None]).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
