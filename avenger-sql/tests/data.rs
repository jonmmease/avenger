//! The options of the files of data scripts: the table of the options, the formats, the URIs,
//! and the files that the statements of a script read, without I/O.

use avenger_sql::{
    data::{
        file_option, file_reads, uri_kind, FileRead, Format, OptionPlace, Partitions, UriKind,
        FILE_OPTIONS, UNSUPPORTED_OPTIONS,
    },
    types::TypeOptions,
};
use avenger_sql_syntax::parse_data;
use datafusion::arrow::datatypes::{DataType, Field};

fn reads(text: &str) -> Vec<FileRead> {
    file_reads(&parse_data(text).syntax(), TypeOptions::default())
}

#[test]
fn the_table_of_options_holds_the_options_of_files_and_of_tables() {
    let names: Vec<&str> = FILE_OPTIONS.iter().map(|option| option.name).collect();
    assert_eq!(
        names,
        [
            "format",
            "uris",
            "skip_leading_rows",
            "field_delimiter",
            "quote",
            "null_marker",
            "allow_quoted_newlines",
            "compression",
            "hive_partition_uri_prefix",
            "description",
        ]
    );
    let format = file_option("FORMAT").unwrap();
    assert!(format.required);
    assert_eq!(format.ty.to_string(), "STRING");
    assert!(format.values.contains(&"JSON"));
    assert_eq!(file_option("uris").unwrap().ty.to_string(), "ARRAY<STRING>");
    assert_eq!(
        file_option("compression").unwrap().formats,
        [Format::Csv, Format::Json]
    );
    assert_eq!(
        file_option("description").unwrap().place,
        OptionPlace::Table
    );
    for name in [
        "max_staleness",
        "metadata_cache_mode",
        "require_hive_partition_filter",
        "encoding",
        "date_format",
    ] {
        assert!(
            UNSUPPORTED_OPTIONS.iter().any(|option| option.name == name),
            "{name}"
        );
    }
    // The values that a data script takes anyway, as GoogleSQL writes them.
    let accepts = |name: &str| -> Vec<String> {
        let option = UNSUPPORTED_OPTIONS
            .iter()
            .find(|option| option.name == name);
        (option.unwrap().accepts.iter())
            .map(ToString::to_string)
            .collect()
    };
    assert_eq!(accepts("encoding"), ["'UTF8'", "'UTF-8'"]);
    assert_eq!(accepts("max_bad_records"), ["0"]);
    assert_eq!(accepts("allow_jagged_rows"), ["FALSE"]);
    assert!(accepts("date_format").is_empty());
    // No option is both supported and unsupported.
    for option in UNSUPPORTED_OPTIONS {
        assert!(file_option(option.name).is_none(), "{}", option.name);
    }
}

#[test]
fn formats_parse_without_regard_to_case_and_json_is_an_alias() {
    assert_eq!(Format::parse("csv"), Some(Format::Csv));
    assert_eq!(Format::parse("JSON"), Some(Format::Json));
    assert_eq!(Format::parse("Newline_Delimited_Json"), Some(Format::Json));
    assert_eq!(Format::parse("arrow"), Some(Format::Arrow));
    assert_eq!(Format::parse("xls"), None);
    assert_eq!(Format::Json.name(), "NEWLINE_DELIMITED_JSON");
    assert!(Format::Csv.available() && Format::Json.available() && Format::Arrow.available());
    assert_eq!(Format::Parquet.available(), cfg!(feature = "parquet"));
    assert_eq!(Format::Avro.available(), cfg!(feature = "avro"));

    // The two names of JSON read the same files.
    let text = "CREATE EXTERNAL TABLE a OPTIONS (format = 'JSON', uris = ['cars.json']);
CREATE EXTERNAL TABLE b OPTIONS (format = 'newline_delimited_json', uris = ['cars.json'])";
    let found = reads(text);
    assert_eq!(found[0].files.format, Format::Json);
    assert_eq!(found[0].files, found[1].files);
}

#[test]
fn file_reads_hold_the_files_of_external_tables_and_loads() {
    let text = "CREATE EXTERNAL TABLE sales
OPTIONS (
  format = 'csv',
  uris = ['data/files/sales/*.csv', 'data/files/extra.csv'],
  field_delimiter = ';',
  quote = \"'\",
  null_marker = '',
  allow_quoted_newlines = TRUE,
  description = 'The sales'
);
LOAD DATA INTO events (id INT64 NOT NULL, kind STRING)
OPTIONS (description = 'The events')
FROM FILES (format = 'CSV', uris = ['data/files/events.csv'], skip_leading_rows = 1);
LOAD DATA OVERWRITE trips
FROM FILES (format = 'ARROW', uris = ['data/files/trips/'], hive_partition_uri_prefix = 'data/files/trips/')
WITH PARTITION COLUMNS (year INT64);
LOAD DATA OVERWRITE broken FROM FILES (format = 'CSV')";
    let found = reads(text);
    let paths: Vec<String> = found.iter().map(|read| read.names.join(".")).collect();
    assert_eq!(paths, ["sales", "events", "trips"]);

    let sales = &found[0];
    assert_eq!(sales.files.format, Format::Csv);
    assert_eq!(
        sales.files.uris,
        ["data/files/sales/*.csv", "data/files/extra.csv"]
    );
    assert!(&text[sales.uris].starts_with("['data/files/sales/*.csv'"));
    assert!(text[sales.range].starts_with("CREATE EXTERNAL TABLE sales"));
    let csv = sales.files.csv.as_ref().unwrap();
    assert!(csv.header);
    assert_eq!((csv.delimiter, csv.quote), (b';', b'\''));
    assert!(csv.quoted_newlines);
    assert_eq!(sales.files.columns, None);

    // A column list names the columns, and skip_leading_rows = 1 skips the header.
    let events = &found[1];
    let columns = events.files.columns.as_ref().unwrap();
    assert_eq!(
        columns[0].as_ref(),
        &Field::new("id", DataType::Int64, false)
    );
    assert_eq!(
        columns[1].as_ref(),
        &Field::new("kind", DataType::Utf8, true)
    );
    assert!(events.files.csv.as_ref().unwrap().header);

    let trips = &found[2];
    assert_eq!(trips.files.csv, None);
    assert_eq!(
        trips.files.hive_partition_uri_prefix.as_deref(),
        Some("data/files/trips/")
    );
    let Some(Partitions::Columns(keys)) = &trips.files.partitions else {
        panic!("{:?}", trips.files.partitions);
    };
    assert_eq!(keys[0].as_ref(), &Field::new("year", DataType::Int64, true));
}

#[test]
fn uris_are_relative_paths_absolute_paths_or_urls_by_their_text() {
    let cases = [
        ("data/files/sales.csv", UriKind::Relative),
        ("./data/../sales.csv", UriKind::Relative),
        // `\` separates names too, as on Windows.
        ("data\\x.csv", UriKind::Relative),
        ("data\\..\\..\\x.csv", UriKind::AboveRoot),
        ("..\\..\\secret.csv", UriKind::AboveRoot),
        ("data/..\\../x.csv", UriKind::AboveRoot),
        ("data/../../x.csv", UriKind::AboveRoot),
        ("../x.csv", UriKind::AboveRoot),
        ("/home/me/x.csv", UriKind::Absolute),
        // A Windows path is absolute on any machine.
        ("C:/x.csv", UriKind::Absolute),
        ("c:\\x.csv", UriKind::Absolute),
        ("\\x.csv", UriKind::Absolute),
        ("file:///home/me/x.csv", UriKind::FileUrl),
        ("FILE:x.csv", UriKind::FileUrl),
        ("s3://bucket/x.csv", UriKind::Url),
        ("https://example.com/x.csv", UriKind::Url),
    ];
    for (uri, kind) in cases {
        assert_eq!(uri_kind(uri), kind, "{uri}");
    }
    // A Windows path and a path above the root are not errors of the options: a loader decides
    // whether it reads them.
    for uri in ["C:/x.csv", "data\\..\\..\\x.csv"] {
        let text = format!("CREATE EXTERNAL TABLE t OPTIONS (format = 'CSV', uris = [r'{uri}'])");
        assert_eq!(reads(&text).len(), 1, "{uri}");
    }
}

#[test]
fn skip_leading_rows_skips_the_header_of_a_column_list() {
    let header = |text: &str| reads(text)[0].files.csv.as_ref().unwrap().header;
    assert!(!header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'])"
    ));
    assert!(!header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 0)"
    ));
    assert!(header(
        "CREATE EXTERNAL TABLE t (a INT64) OPTIONS (format = 'CSV', uris = ['a.csv'], skip_leading_rows = 1)"
    ));
}

#[test]
fn partition_columns_read_the_keys_after_the_prefix() {
    let ok = "CREATE EXTERNAL TABLE t WITH PARTITION COLUMNS
OPTIONS (format = 'CSV', uris = ['events/*'], hive_partition_uri_prefix = 'events/')";
    assert_eq!(reads(ok)[0].files.partitions, Some(Partitions::Inferred));
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_reads_with_the_parquet_feature() {
    let found = reads("CREATE EXTERNAL TABLE t OPTIONS (format = 'PARQUET', uris = ['a.parquet'])");
    assert_eq!(found[0].files.format, Format::Parquet);
}

#[cfg(not(feature = "parquet"))]
#[test]
fn parquet_is_an_error_without_the_parquet_feature() {
    // A statement whose options have an error reads no files.
    assert!(
        reads("CREATE EXTERNAL TABLE t OPTIONS (format = 'parquet', uris = ['a.parquet'])")
            .is_empty()
    );
}

#[cfg(feature = "avro")]
#[test]
fn avro_reads_with_the_avro_feature() {
    let found = reads("CREATE EXTERNAL TABLE t OPTIONS (format = 'AVRO', uris = ['a.avro'])");
    assert_eq!(found[0].files.format, Format::Avro);
}

#[cfg(not(feature = "avro"))]
#[test]
fn avro_is_an_error_without_the_avro_feature() {
    assert!(
        reads("CREATE EXTERNAL TABLE t OPTIONS (format = 'AVRO', uris = ['a.avro'])").is_empty()
    );
}

#[cfg(feature = "compression")]
#[test]
fn compressed_files_read_with_the_compression_feature() {
    let text = "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv.zst'], compression = 'zstd')";
    assert_eq!(
        reads(text)[0].files.compression,
        Some(avenger_sql::data::Compression::Zstd)
    );
}

#[cfg(not(feature = "compression"))]
#[test]
fn compressed_files_are_an_error_without_the_compression_feature() {
    let text = "LOAD DATA OVERWRITE t FROM FILES (format = 'CSV', uris = ['a.csv.gz'], compression = 'GZIP')";
    assert!(reads(text).is_empty());
}
