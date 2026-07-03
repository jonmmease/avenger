# Test Datasets

Most top-level Parquet datasets are sourced from
[vega/vega-datasets](https://github.com/vega/vega-datasets) and converted to
Parquet format for efficient loading in tests. Dataset-specific subdirectories
may carry their own source and license notes.

## License

All datasets are from vega-datasets, which is MIT licensed.
Copyright (c) 2015-2023 University of Washington Interactive Data Lab

See: https://github.com/vega/vega-datasets/blob/main/LICENSE

## Datasets

- **seattle-weather.parquet** - Daily weather observations from Seattle (2012-2015)
- **stocks.parquet** - Daily stock prices for 5 tech companies (2000-2010)
- **iris.parquet** - Classic Iris flower dataset (Fisher, 1936) from UCI ML Repository
- **cars.parquet** - Automotive data from the 1970s-1980s
- **barley.parquet** - Agricultural yield data from Minnesota (1931-1932)
- **airports.parquet** - US airport locations and metadata
- **co2-concentration.parquet** - Atmospheric CO2 concentration measurements
- **unemployment.parquet** - US unemployment rates by county
- **movies.parquet** - IMDB movie ratings and metadata
- **nyc_taxi_2015/** - NYC taxi fixture from Datashader for rasterization and
  map-tile baselines (originally for the retired WebMercator suite, now the geo_mercator parity suite)

## Updating

To re-sync datasets from vega-datasets:

```bash
uv run scripts/sync_test_data.py
```

## Format

The top-level Vega-derived files are stored as Parquet with Snappy compression
for:
- Efficient storage (typically 50-70% smaller than CSV)
- Fast loading with DataFusion
- Preserved schema and data types
- No parsing ambiguity

**Note**: All `.parquet` files are tracked with Git LFS. Make sure you have `git-lfs` installed:
```bash
git lfs install
```
