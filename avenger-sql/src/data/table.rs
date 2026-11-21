//! The table of a set of files: a DataFusion `ListingTable` over the files of the URIs of
//! [`Files`], read through the object stores of a session.

use std::{borrow::Cow, sync::Arc};

use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, SchemaRef},
    catalog::{ScanArgs, ScanResult, Session, TableProvider},
    common::{
        tree_node::{Transformed, TreeNode},
        Constraints, Result, ScalarValue, Statistics,
    },
    datasource::{
        file_format::{
            arrow::ArrowFormat, csv::CsvFormat, file_compression_type::FileCompressionType,
            json::JsonFormat, FileFormat,
        },
        listing::{
            helpers::parse_partitions_for_path, ListingOptions, ListingTable, ListingTableConfig,
            ListingTableUrl,
        },
    },
    execution::{cache::TableScopedPath, object_store::ObjectStoreUrl},
    logical_expr::{
        dml::InsertOp, expr::InList, BinaryExpr, Expr, Operator, TableProviderFilterPushDown,
        TableType,
    },
    object_store::{path::Path, ObjectMeta, ObjectStore, ObjectStoreExt},
    physical_plan::ExecutionPlan,
};
use futures::{StreamExt, TryStreamExt};
use glob::Pattern;
use url::Url;

use super::{uri_kind, Compression, Files, Format, Partitions, UriKind};
use crate::{catalog::FilesTable, types};

/// The table of the files `files`: a `ListingTable` of their format and options, whose URIs are
/// relative to `base`, the URL of the project's root directory, such as `file:///home/me/sales/`
/// or `memory://project/`, at whose object store the session `session` reads them. A URI that is
/// a `file:` URL, or, outside of wasm, an absolute path, names its file on its own.
///
/// A URI with a `*` reads the files of the directory before the `*`, up to the last `/` or `\`
/// before it, whose names match the rest as a glob pattern, in which `*` is the only wildcard:
/// `[` and `?` are the characters of a file's name. A URI that ends with `/` or `\` reads the
/// files of its directory with the format's extension, and that of the compression with one, as
/// `.csv.gz`. Neither reads the files of the directory's subdirectories. Any other URI is one
/// file, and a URI that names a directory without a `/` or `\` at its end is an error. Each URI
/// matches a file or more, or the table is an error that names it.
///
/// A table without a column list takes the schema of its files, which DataFusion infers from the
/// first rows of the first CSV or JSON file that is not empty, and from every file of the other
/// formats, and a JSON file whose first character is `[` is an array of objects. The partition
/// columns of a Hive layout follow the columns of the files. When `WITH PARTITION COLUMNS` has no
/// list, they are the keys of the paths of the files after `hive_partition_uri_prefix`, each of
/// the type that all its values have, as [`key_type`] detects it.
/// The error of a table is a message for the user, which a loader places at the URIs.
///
/// A table without partition columns reads the files that its URIs matched when it was built,
/// and a loader builds the table again after the files change. The scans of a Hive layout list
/// the files under its prefix through the session's cache of listings, which DataFusion keeps by
/// default, so they see the files as they were when the table was built, and a session without
/// the cache, whose `list_files_cache_limit` is 0, lists them at each query.
///
/// The paths of the store are the names of the files, as `Path::parse` keeps them, and not
/// `Path::from`, which encodes characters such as `[`, `?` and `*`.
pub async fn file_table(session: &dyn Session, files: &Files, base: &str) -> FilesTable {
    let base = Url::parse(base)
        .ok()
        .filter(|url| !url.cannot_be_a_base() && url.path().ends_with('/'))
        .ok_or_else(|| {
            format!("The base URL {base} of the project's files is not the URL of a directory")
        })?;
    let compression = compression_type(files.compression);
    let paths = (files.uris.iter())
        .map(|uri| table_url(&base, uri, files))
        .collect::<Result<Vec<_>, String>>()?;
    let store_url = paths[0].object_store();
    if let Some(other) = (paths.iter()).find(|path| path.object_store() != store_url) {
        return Err(format!(
            "The URIs of a table are in one object store, and {} is not in {}",
            other.as_str(),
            store_url.as_str()
        ));
    }
    let store = (session.runtime_env().object_store(&store_url)).map_err(|error| {
        format!(
            "The session reads no files at {}: {error}",
            store_url.as_str()
        )
    })?;
    // The files of each URI, and the table paths of the table: the URL of each file of a table
    // without partition columns, so that it reads those files and no other, and the prefix of
    // each URI of a Hive layout, which DataFusion lists.
    let mut listed: Vec<ObjectMeta> = Vec::new();
    let mut table_paths: Vec<ListingTableUrl> = Vec::new();
    for (path, uri) in paths.iter().zip(&files.uris) {
        let found = list(session, store.as_ref(), path, uri).await?;
        if found.is_empty() {
            return Err(format!("No file matches '{uri}'"));
        }
        match path.files {
            UrlFiles::Hive => table_paths.push(path.url.clone()),
            UrlFiles::File | UrlFiles::Names(_) => {
                for file in &found {
                    table_paths.push(path.file(&file.location)?);
                }
            }
        }
        listed.extend(found);
    }
    // Empty files hold no schema, and some formats cannot infer one from them, so the first file
    // that is not empty tells an array of JSON from lines, and names and types the columns.
    let filled: Vec<ObjectMeta> = (listed.iter())
        .filter(|file| file.size > 0)
        .cloned()
        .collect();
    let format: Arc<dyn FileFormat> = match files.format {
        Format::Csv => {
            let csv = (files.csv.as_ref())
                .ok_or_else(|| "The CSV files have no options of CSV files".to_string())?;
            Arc::new(
                CsvFormat::default()
                    .with_has_header(csv.header)
                    .with_delimiter(csv.delimiter)
                    .with_quote(csv.quote)
                    .with_newlines_in_values(csv.quoted_newlines)
                    .with_file_compression_type(compression),
            )
        }
        Format::Json => {
            let array = match filled.first() {
                Some(file) => json_array(store.as_ref(), file, compression)
                    .await
                    .map_err(|error| format!("{} can't be read: {error}", file.location))?,
                None => false,
            };
            Arc::new(
                JsonFormat::default()
                    .with_newline_delimited(!array)
                    .with_file_compression_type(compression),
            )
        }
        Format::Arrow => Arc::new(ArrowFormat),
        #[cfg(feature = "parquet")]
        Format::Parquet => {
            Arc::new(datafusion::datasource::file_format::parquet::ParquetFormat::default())
        }
        #[cfg(feature = "avro")]
        Format::Avro => Arc::new(datafusion::datasource::file_format::avro::AvroFormat),
        #[allow(unreachable_patterns)]
        format => {
            return Err(format!(
                "This build of avenger-sql reads no {} files: it lacks the cargo feature {}",
                format.name(),
                format.feature().unwrap_or_default()
            ))
        }
    };
    // The URIs select the files, so the table keeps every file that they list.
    let mut options = ListingOptions::new(format).with_file_extension("");
    let partitions: Vec<(String, DataType)> = match &files.partitions {
        None => Vec::new(),
        Some(Partitions::Columns(fields)) => (fields.iter())
            .map(|field| (field.name().clone(), field.data_type().clone()))
            .collect(),
        Some(Partitions::Inferred) => {
            let mut keys: Option<Vec<String>> = None;
            for (path, uri) in paths.iter().zip(&files.uris) {
                let found = (options.infer_partitions(session, &path.url).await)
                    .map_err(|error| format!("The partition keys of '{uri}': {error}"))?;
                match &keys {
                    Some(keys) if *keys != found => {
                        return Err(format!(
                            "The files of '{uri}' have the partition keys {}, and the files of '{}' the keys {}",
                            found.join(", "),
                            files.uris[0],
                            keys.join(", ")
                        ))
                    }
                    _ => keys = Some(found),
                }
            }
            let keys = keys.unwrap_or_default();
            if keys.is_empty() {
                return Err(format!(
                    "The paths of the files after '{}' have no partition keys, as in year=2025/",
                    files
                        .hive_partition_uri_prefix
                        .as_deref()
                        .unwrap_or_default()
                ));
            }
            // The values of the keys in the paths of the files, which share one prefix, and
            // which DataFusion reads in the same way at a scan.
            let names = || keys.iter().map(String::as_str);
            let values: Vec<Vec<String>> = (listed.iter())
                .filter_map(|file| {
                    parse_partitions_for_path(&paths[0].url, &file.location, names())
                })
                .map(|values| values.into_iter().map(Cow::into_owned).collect())
                .collect();
            (keys.iter().enumerate())
                .map(|(index, key)| {
                    let ty = key_type(values.iter().map(|values| values[index].as_str()));
                    (key.clone(), ty)
                })
                .collect()
        }
    };
    options = options.with_table_partition_cols(partitions);
    let schema: SchemaRef = match &files.columns {
        Some(columns) => Arc::new(Schema::new(columns.clone())),
        None => {
            // The first rows of the first CSV or JSON file name and type the columns, and the
            // files of the other formats each hold a schema, which DataFusion merges.
            let mut files_of_schema = filled.clone();
            if files_of_schema.is_empty() {
                return Err(
                    "The files are empty, so they have no columns: a column list gives them"
                        .to_string(),
                );
            }
            if matches!(files.format, Format::Csv | Format::Json) {
                files_of_schema.truncate(1);
            }
            (options.format.infer_schema(session, &store, &files_of_schema).await).map_err(
                |error| {
                    format!(
                        "The schema of the files can't be read: {error}. A column list gives the table its columns"
                    )
                },
            )?
        }
    };
    // A partition column follows the columns of the files, so a column of the same name in the
    // files or in the column list is an error of the table.
    let names = |field: &Field| field.name().to_ascii_lowercase();
    if let Some(field) = (schema.fields().iter()).find(|field| {
        (options.table_partition_cols.iter())
            .any(|(key, _)| key.to_ascii_lowercase() == names(field))
    }) {
        return Err(format!(
            "Column {} is both a column of the files and a partition key",
            field.name()
        ));
    }
    let config = ListingTableConfig::new_with_multi_paths(table_paths)
        .with_listing_options(options)
        .with_schema(schema);
    let table = ListingTable::try_new(config).map_err(|error| error.to_string())?;
    if table.options().table_partition_cols.is_empty() {
        Ok(Arc::new(table) as Arc<dyn TableProvider>)
    } else {
        Ok(Arc::new(HiveTable { listing: table }) as Arc<dyn TableProvider>)
    }
}

/// The table of a Hive layout: a `ListingTable` whose scans read every folder whose values match
/// the filters, however a folder spells them.
///
/// DataFusion 55 lists only the files under a prefix that it spells from the equalities of a
/// scan's filters on the leading partition keys, with the display of each literal, so it misses
/// the folders spelled otherwise, such as `month=01` for `month = 1`, a time such as
/// `t=2026-02-01 08:30:00+00`, which displays as `2026-02-01T08:30:00Z`, or a value whose writer
/// encoded a `:` as `%3A`. Each scan writes such an equality as `IN` of its one value, which has
/// the same rows, and which DataFusion turns into no prefix, so the scan lists the files under
/// the table's prefix and filters each by the values that it reads from its path.
#[derive(Debug)]
struct HiveTable {
    listing: ListingTable,
}

impl HiveTable {
    /// The filters `filters`, with each equality of a partition key and a literal as `IN`.
    fn filters(&self, filters: &[Expr]) -> Result<Vec<Expr>> {
        let keys = &self.listing.options().table_partition_cols;
        let key = |expr: &Expr| matches!(expr, Expr::Column(column) if keys.iter().any(|(key, _)| *key == column.name));
        (filters.iter().cloned())
            .map(|filter| {
                filter
                    .transform_up(|expr| match expr {
                        Expr::BinaryExpr(BinaryExpr {
                            left,
                            op: Operator::Eq,
                            right,
                        }) if (key(&left) && matches!(*right, Expr::Literal(..)))
                            || (key(&right) && matches!(*left, Expr::Literal(..))) =>
                        {
                            let (column, value) = if key(&left) {
                                (left, *right)
                            } else {
                                (right, *left)
                            };
                            Ok(Transformed::yes(Expr::InList(InList::new(
                                column,
                                vec![value],
                                false,
                            ))))
                        }
                        expr => Ok(Transformed::no(expr)),
                    })
                    .map(|expr| expr.data)
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl TableProvider for HiveTable {
    fn schema(&self) -> SchemaRef {
        self.listing.schema()
    }

    fn constraints(&self) -> Option<&Constraints> {
        self.listing.constraints()
    }

    fn table_type(&self) -> TableType {
        self.listing.table_type()
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.listing.get_column_default(column)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let filters = self.filters(filters)?;
        self.listing.scan(state, projection, &filters, limit).await
    }

    async fn scan_with_args<'a>(
        &self,
        state: &dyn Session,
        args: ScanArgs<'a>,
    ) -> Result<ScanResult> {
        let filters = args
            .filters()
            .map(|filters| self.filters(filters))
            .transpose()?;
        let args = args.with_filters(filters.as_deref());
        self.listing.scan_with_args(state, args).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> Result<Vec<TableProviderFilterPushDown>> {
        self.listing.supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        self.listing.statistics()
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.listing.insert_into(state, input, insert_op).await
    }
}

/// The type of a partition key whose values in the paths of the files are `values`, as BigQuery
/// detects the types of a Hive layout's keys: `INT64` when every value is an integer, else `DATE`
/// when every value is a date such as `2025-01-31`, else `TIMESTAMP` when every value is a date
/// and a time, such as `2025-01-31 16:00:00+00`, and else `STRING`. Each test is the cast that a
/// scan applies to the values, so every file of a table reads under the type of its keys.
fn key_type<'a>(values: impl Iterator<Item = &'a str> + Clone) -> DataType {
    let casts = |value: &str, ty: &DataType| {
        ScalarValue::try_from_string(value.to_string(), ty).is_ok_and(|value| !value.is_null())
    };
    // Arrow's cast to `Date32` also reads a date and a time, and drops the time, so a date is
    // also no longer than `YYYY-MM-DD`.
    let date = |value: &str| value.len() <= 10 && casts(value, &DataType::Date32);
    if values.clone().next().is_none() {
        DataType::Utf8
    } else if values.clone().all(|value| casts(value, &DataType::Int64)) {
        DataType::Int64
    } else if values.clone().all(date) {
        DataType::Date32
    } else if values
        .clone()
        .all(|value| casts(value, &types::timestamp()))
    {
        types::timestamp()
    } else {
        DataType::Utf8
    }
}

/// The compression of DataFusion of `compression`.
fn compression_type(compression: Option<Compression>) -> FileCompressionType {
    match compression {
        None => FileCompressionType::UNCOMPRESSED,
        Some(Compression::Gzip) => FileCompressionType::GZIP,
        Some(Compression::Bzip2) => FileCompressionType::BZIP2,
        Some(Compression::Xz) => FileCompressionType::XZ,
        Some(Compression::Zstd) => FileCompressionType::ZSTD,
    }
}

/// The extension of the files of `files` in a directory: the format's, and the compression's.
fn extension(files: &Files) -> String {
    let compression = match files.compression {
        None => "",
        Some(Compression::Gzip) => ".gz",
        Some(Compression::Bzip2) => ".bz2",
        Some(Compression::Xz) => ".xz",
        Some(Compression::Zstd) => ".zst",
    };
    format!("{}{compression}", files.format.extension())
}

/// The separators of the names of a path, as [`uri_kind`] reads them.
const SEPARATORS: [char; 2] = ['/', '\\'];

/// The URL of the URI `uri` of `files`, relative to `base`: the URL of a file, of a directory
/// with the glob pattern of the names of its files, or of the prefix of a Hive layout.
fn table_url(base: &Url, uri: &str, files: &Files) -> Result<FileUrl, String> {
    let directory_glob = || format!("*.{}", Pattern::escape(&extension(files)));
    let pattern = |glob: &str| {
        Pattern::new(glob)
            .map_err(|error| format!("The URI '{uri}' is not a pattern of file names: {error}"))
    };
    let listing = |url: Url, glob: Option<Pattern>| {
        ListingTableUrl::try_new(url, glob)
            .map_err(|error| format!("The URI '{uri}' is not a path: {error}"))
    };
    let (directory, glob) = match &files.hive_partition_uri_prefix {
        // The URL of a Hive layout is its prefix, under which DataFusion reads the keys of the
        // paths, and the rest of the URI selects the files by name.
        Some(prefix) if files.partitions.is_some() => {
            let directory = prefix.trim_end_matches(SEPARATORS);
            if directory.is_empty() {
                return Err(format!(
                    "The hive_partition_uri_prefix '{prefix}' is not the path of a directory"
                ));
            }
            let rest = uri.strip_prefix(prefix.as_str()).ok_or_else(|| {
                format!(
                    "The URI '{uri}' doesn't start with the hive_partition_uri_prefix '{prefix}'"
                )
            })?;
            let rest = rest.strip_prefix(SEPARATORS).unwrap_or(rest);
            if rest.contains(SEPARATORS) {
                return Err(format!(
                    "The URI '{uri}' of a Hive layout is its hive_partition_uri_prefix, or the prefix and a pattern of the names of its files, as '{directory}/*.{}'",
                    files.format.extension()
                ));
            }
            let glob = match rest {
                "" => directory_glob(),
                rest => file_glob(rest),
            };
            let url = listing(
                resolve(base, &format!("{directory}/"))?,
                Some(pattern(&glob)?),
            )?;
            return Ok(FileUrl {
                url,
                files: UrlFiles::Hive,
            });
        }
        _ => match uri.find('*') {
            Some(star) => {
                let split = uri[..star]
                    .rfind(SEPARATORS)
                    .map_or(0, |separator| separator + 1);
                (&uri[..split], file_glob(&uri[split..]))
            }
            None if uri.ends_with(SEPARATORS) => (uri, directory_glob()),
            None => {
                return Ok(FileUrl {
                    url: listing(resolve(base, uri)?, None)?,
                    files: UrlFiles::File,
                })
            }
        },
    };
    Ok(FileUrl {
        url: listing(resolve(base, directory)?, None)?,
        files: UrlFiles::Names(pattern(&glob)?),
    })
}

/// The glob pattern of the names of files that `text` writes, whose only wildcard is `*`.
fn file_glob(text: &str) -> String {
    (text.split('*'))
        .map(Pattern::escape)
        .collect::<Vec<_>>()
        .join("*")
}

/// A URI of a [`Files`], resolved against the URL of the project's root, as [`file_table`]
/// resolves it: the URL of a file, of a directory with a pattern of the names of its files, or of
/// the prefix of a Hive layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileUrl {
    /// The URL of the file, the directory, or the prefix, with the pattern of the names of the
    /// files of a Hive layout, which DataFusion's listing of the prefix matches.
    url: ListingTableUrl,
    files: UrlFiles,
}

/// The files that a [`FileUrl`] reads at its URL.
#[derive(Clone, Debug, PartialEq, Eq)]
enum UrlFiles {
    /// The file at the URL.
    File,
    /// The files in the directory of the URL, and not in its subdirectories, whose names match
    /// the pattern.
    Names(Pattern),
    /// The files under the prefix of a Hive layout that DataFusion's listing keeps.
    Hive,
}

impl FileUrl {
    /// The object store of the URL, which holds the files that it reads.
    pub fn object_store(&self) -> ObjectStoreUrl {
        self.url.object_store()
    }

    /// The URL, as text.
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    /// Whether the URI reads the file at `path` of its object store. The match reads the URI and
    /// not the store, so it holds for a file that no longer exists, or that no listing has seen:
    /// the file of a URI of one file, a file in the directory of a pattern or of a URI of a
    /// directory, and not in a subdirectory, whose name matches, and a file under the prefix of
    /// a Hive layout that DataFusion's listing keeps, by `ListingTableUrl::contains`, which
    /// `ignore_subdirectory`, the session's `listing_table_ignore_subdirectory`, configures.
    pub fn reads(&self, path: &Path, ignore_subdirectory: bool) -> bool {
        match &self.files {
            UrlFiles::File => path == self.url.prefix(),
            UrlFiles::Names(names) => match self.url.strip_prefix(path) {
                Some(mut segments) => match (segments.next(), segments.next()) {
                    (Some(name), None) => names.matches(name),
                    _ => false,
                },
                None => false,
            },
            UrlFiles::Hive => self.url.contains(path, ignore_subdirectory),
        }
    }

    /// The table path of the file at `path`, which the URI reads: the URL of the file, so a
    /// table reads it and no other.
    fn file(&self, path: &Path) -> Result<ListingTableUrl, String> {
        let mut url = self.url.get_url().clone();
        if let UrlFiles::Names(_) = self.files {
            let name = path.filename().unwrap_or_default();
            (url.path_segments_mut())
                .map_err(|()| format!("The URL {} of a directory has no path", self.as_str()))?
                .pop_if_empty()
                .push(name);
        }
        ListingTableUrl::try_new(url, None).map_err(|error| error.to_string())
    }
}

/// The URL of the path `path`, by its [`uri_kind`]: a `file:` URL as it is, an absolute path of
/// this machine outside of wasm, or a path relative to `base`, whose segments are names and never
/// a query or a fragment, which `/` or `\` separate, as [`uri_kind`] reads them, and in which `..`
/// names the parent directory. A `file:` URL follows the syntax of URLs, so a `?` or a `#` in it
/// starts its query or its fragment.
fn resolve(base: &Url, path: &str) -> Result<Url, String> {
    match uri_kind(path) {
        UriKind::FileUrl => {
            return Url::parse(path).map_err(|error| format!("'{path}' is not a URL: {error}"))
        }
        UriKind::Url => {
            return Err(format!(
            "'{path}' is a URL of another scheme than file:, and a data script reads local files"
        ))
        }
        UriKind::Absolute => return absolute(path),
        UriKind::Relative | UriKind::AboveRoot => {}
    }
    // The depth of the path below the root of the store, which `..` doesn't go above.
    let mut depth = (base.path_segments().into_iter().flatten())
        .filter(|segment| !segment.is_empty())
        .count();
    let mut url = base.clone();
    {
        let mut segments =
            (url.path_segments_mut()).map_err(|()| format!("The base URL {base} has no path"))?;
        segments.pop_if_empty();
        for segment in path.split(SEPARATORS) {
            match segment {
                "." => {}
                ".." if depth == 0 => {
                    return Err(format!(
                        "'{path}' goes above the root of the store of the project's files, {base}"
                    ))
                }
                ".." => {
                    depth -= 1;
                    segments.pop();
                }
                segment => {
                    depth += 1;
                    segments.push(segment);
                }
            }
        }
    }
    Ok(url)
}

/// The URL of the absolute path `path`, of a directory when it ends with a separator. `\`
/// separates names as `/` does, on any machine.
#[cfg(not(target_arch = "wasm32"))]
fn absolute(path: &str) -> Result<Url, String> {
    let separated = match std::path::MAIN_SEPARATOR {
        '/' => path.replace('\\', "/"),
        _ => path.to_string(),
    };
    let url = match separated.ends_with(SEPARATORS) {
        true => Url::from_directory_path(&separated),
        false => Url::from_file_path(&separated),
    };
    url.map_err(|()| format!("'{path}' is not an absolute path of this machine"))
}

/// A web build reads no absolute paths.
#[cfg(target_arch = "wasm32")]
fn absolute(path: &str) -> Result<Url, String> {
    Err(format!(
        "'{path}' is an absolute path, which a web build doesn't read: a URI is a path relative to the project root"
    ))
}

/// The files of the store `store` that the URI `uri`, at `url`, reads, in the order of their
/// paths, as the store holds them now, or the error of listing them. A URI of one file names no
/// directory. The listing of a Hive layout replaces the one that the session's cache of listings
/// holds for its prefix, so a table that a loader builds again after its files change lists them
/// as they are.
async fn list(
    session: &dyn Session,
    store: &dyn ObjectStore,
    url: &FileUrl,
    uri: &str,
) -> Result<Vec<ObjectMeta>, String> {
    let failed =
        |error: &dyn std::fmt::Display| format!("The files of '{uri}' can't be listed: {error}");
    let prefix = url.url.prefix();
    let mut files: Vec<ObjectMeta> = match &url.files {
        UrlFiles::File => match store.head(prefix).await {
            Ok(file) => vec![file],
            Err(datafusion::object_store::Error::NotFound { .. }) => {
                // A path without a file is a directory when the store holds files under it.
                let listing = (store.list_with_delimiter(Some(prefix)).await)
                    .map_err(|error| failed(&error))?;
                if !listing.objects.is_empty() || !listing.common_prefixes.is_empty() {
                    return Err(format!(
                        "'{uri}' names a directory, and the URI of a directory ends with /, as '{}/'",
                        uri.trim_end_matches(SEPARATORS)
                    ));
                }
                Vec::new()
            }
            Err(error) => return Err(failed(&error)),
        },
        UrlFiles::Names(_) => (store.list_with_delimiter(Some(prefix)).await)
            .map_err(|error| failed(&error))?
            .objects
            .into_iter()
            .filter(|file| url.reads(&file.location, true))
            .collect(),
        UrlFiles::Hive => {
            if let Some(cache) = session.runtime_env().cache_manager.get_list_files_cache() {
                cache.remove(&TableScopedPath {
                    table: None,
                    path: prefix.clone(),
                });
            }
            let listing = async {
                url.url
                    .list_all_files(session, store, "")
                    .await?
                    .try_collect::<Vec<_>>()
                    .await
            };
            listing.await.map_err(|error| failed(&error))?
        }
    };
    files.sort_by(|a, b| a.location.cmp(&b.location));
    Ok(files)
}

/// Whether the JSON file `file` is an array of objects, which its first character `[` tells,
/// after a byte order mark and white space. It reads the file until that character.
async fn json_array(
    store: &dyn ObjectStore,
    file: &ObjectMeta,
    compression: FileCompressionType,
) -> datafusion::common::Result<bool> {
    let stream = store.get(&file.location).await?.into_stream();
    let mut stream = compression.convert_stream(stream.map_err(Into::into).boxed())?;
    let mut start = true;
    while let Some(bytes) = stream.next().await.transpose()? {
        let mut bytes: &[u8] = &bytes;
        if start && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bytes = &bytes[3..];
        }
        start = false;
        if let Some(byte) = bytes.iter().find(|byte| !byte.is_ascii_whitespace()) {
            return Ok(*byte == b'[');
        }
    }
    Ok(false)
}
