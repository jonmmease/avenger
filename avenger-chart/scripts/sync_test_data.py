#!/usr/bin/env python3
# /// script
# dependencies = [
#   "pandas>=2.0.0",
#   "pyarrow>=12.0.0",
#   "requests>=2.31.0",
# ]
# ///
"""
Download datasets from vega-datasets and convert to Parquet format.

This script downloads canonical visualization datasets and saves them as Parquet files
for use in visual regression tests. Run this script to sync/update the test datasets.

Usage:
    uv run scripts/sync_test_data.py

Or with standard Python:
    pip install pandas pyarrow requests
    python scripts/sync_test_data.py
"""

import os
import sys
from pathlib import Path
from typing import Dict, List
import requests
import pandas as pd

# Base URL for vega-datasets raw files
VEGA_DATA_BASE = "https://raw.githubusercontent.com/vega/vega-datasets/main/data"

# Dataset configurations: name -> (source_file, read_kwargs)
# For vega-datasets, use the main branch
VEGA_DATASETS: Dict[str, tuple] = {
    # CSV files
    "seattle-weather": ("seattle-weather.csv", {"parse_dates": ["date"]}),
    "stocks": ("stocks.csv", {"parse_dates": ["date"]}),
    "airports": ("airports.csv", {}),
    "co2-concentration": ("co2-concentration.csv", {}),

    # JSON files (regular JSON arrays)
    "cars": ("cars.json", {}),
    "barley": ("barley.json", {}),
    "movies": ("movies.json", {}),

    # TSV files
    "unemployment": ("unemployment.tsv", {}),
}

# Other datasets from different sources
OTHER_DATASETS: Dict[str, tuple] = {
    # Iris from UCI ML Repository (classic dataset)
    "iris": (
        "https://archive.ics.uci.edu/ml/machine-learning-databases/iris/iris.data",
        {
            "names": ["sepal_length", "sepal_width", "petal_length", "petal_width", "species"],
            "header": None,
        }
    ),
}

def download_file(url: str, dest_path: Path) -> bool:
    """Download a file from URL to destination path."""
    try:
        print(f"  Downloading {url}...")
        response = requests.get(url, timeout=30)
        response.raise_for_status()

        dest_path.parent.mkdir(parents=True, exist_ok=True)
        dest_path.write_bytes(response.content)
        print(f"  ✓ Downloaded to {dest_path}")
        return True
    except Exception as e:
        print(f"  ✗ Failed to download: {e}", file=sys.stderr)
        return False

def read_dataset(source_path: Path, read_kwargs: dict) -> pd.DataFrame:
    """Read dataset into pandas DataFrame based on file extension."""
    suffix = source_path.suffix.lower()

    if suffix == ".csv":
        return pd.read_csv(source_path, **read_kwargs)
    elif suffix == ".json":
        # Vega datasets use regular JSON arrays
        return pd.read_json(source_path, **read_kwargs)
    elif suffix == ".tsv":
        return pd.read_csv(source_path, sep="\t", **read_kwargs)
    else:
        raise ValueError(f"Unsupported file format: {suffix}")

def convert_to_parquet(name: str, source_file_or_url: str, read_kwargs: dict,
                       temp_dir: Path, output_dir: Path, is_full_url: bool = False) -> bool:
    """Download dataset, convert to Parquet, and save."""
    if is_full_url:
        url = source_file_or_url
        # Extract filename from URL or use a default
        temp_filename = f"{name}.csv"
    else:
        url = f"{VEGA_DATA_BASE}/{source_file_or_url}"
        temp_filename = source_file_or_url

    temp_path = temp_dir / temp_filename
    output_path = output_dir / f"{name}.parquet"

    print(f"\n📦 Processing {name}...")

    # Download source file
    if not download_file(url, temp_path):
        return False

    try:
        # Read into pandas
        print(f"  Reading {temp_filename}...")
        df = read_dataset(temp_path, read_kwargs)

        # Show info
        rows, cols = df.shape
        size_kb = temp_path.stat().st_size / 1024
        print(f"  ℹ  {rows} rows × {cols} columns ({size_kb:.1f} KB)")
        print(f"  ℹ  Columns: {', '.join(df.columns)}")

        # Clean up object columns with mixed types (convert to string)
        for col in df.columns:
            if df[col].dtype == 'object':
                df[col] = df[col].astype(str)

        # Convert to Parquet
        print(f"  Converting to Parquet...")
        df.to_parquet(output_path, index=False, compression="snappy")

        parquet_size_kb = output_path.stat().st_size / 1024
        compression_ratio = (1 - parquet_size_kb / size_kb) * 100 if size_kb > 0 else 0
        print(f"  ✓ Saved to {output_path} ({parquet_size_kb:.1f} KB, {compression_ratio:.0f}% compression)")

        return True

    except Exception as e:
        print(f"  ✗ Failed to convert: {e}", file=sys.stderr)
        return False

def create_readme(output_dir: Path):
    """Create README documenting the datasets."""
    readme_content = """# Test Datasets

These datasets are sourced from [vega/vega-datasets](https://github.com/vega/vega-datasets)
and converted to Parquet format for efficient loading in tests.

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

## Updating

To re-sync datasets from vega-datasets:

```bash
uv run scripts/sync_test_data.py
```

## Format

All files are stored as Parquet with Snappy compression for:
- Efficient storage (typically 50-70% smaller than CSV)
- Fast loading with DataFusion
- Preserved schema and data types
- No parsing ambiguity

**Note**: All `.parquet` files are tracked with Git LFS. Make sure you have `git-lfs` installed:
```bash
git lfs install
```
"""

    readme_path = output_dir / "README.md"
    readme_path.write_text(readme_content)
    print(f"\n📝 Created {readme_path}")

def main():
    # Setup paths
    script_dir = Path(__file__).parent
    repo_root = script_dir.parent
    output_dir = repo_root / "tests" / "data"
    temp_dir = script_dir / "temp_downloads"

    print("=" * 70)
    print("Vega Datasets → Parquet Converter")
    print("=" * 70)
    print(f"\nOutput directory: {output_dir}")
    print(f"Temp directory: {temp_dir}")

    # Create directories
    output_dir.mkdir(parents=True, exist_ok=True)
    temp_dir.mkdir(parents=True, exist_ok=True)

    # Process each dataset
    success_count = 0
    fail_count = 0
    total_datasets = len(VEGA_DATASETS) + len(OTHER_DATASETS)

    # Process vega-datasets
    for name, (source_file, read_kwargs) in VEGA_DATASETS.items():
        if convert_to_parquet(name, source_file, read_kwargs, temp_dir, output_dir, is_full_url=False):
            success_count += 1
        else:
            fail_count += 1

    # Process other datasets (full URLs)
    for name, (url, read_kwargs) in OTHER_DATASETS.items():
        if convert_to_parquet(name, url, read_kwargs, temp_dir, output_dir, is_full_url=True):
            success_count += 1
        else:
            fail_count += 1

    # Create README
    create_readme(output_dir)

    # Cleanup temp directory
    print(f"\n🧹 Cleaning up temporary files...")
    for temp_file in temp_dir.glob("*"):
        temp_file.unlink()
    temp_dir.rmdir()

    # Summary
    print("\n" + "=" * 70)
    print(f"✓ Successfully converted: {success_count}/{total_datasets} datasets")
    if fail_count > 0:
        print(f"✗ Failed: {fail_count}/{total_datasets} datasets")
    print("=" * 70)

    # Calculate total size
    total_size = sum(f.stat().st_size for f in output_dir.glob("*.parquet"))
    print(f"\nTotal size: {total_size / 1024:.1f} KB ({total_size / 1024 / 1024:.2f} MB)")
    print(f"\nDatasets ready in: {output_dir}")

    return 0 if fail_count == 0 else 1

if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n\n⚠️  Interrupted by user")
        sys.exit(130)
    except Exception as e:
        print(f"\n❌ Error: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        sys.exit(1)
