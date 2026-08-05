#!/usr/bin/env -S uv run --python 3.12
# /// script
# requires-python = ">=3.12,<3.13"
# dependencies = [
#   "pyarrow==18.1.0",
#   "python-dateutil==2.9.0.post0",
#   "shapely==2.1.2",
# ]
# ///
"""Regenerate the pinned Vega-Lite gallery inventory and local data catalog.

This command is deliberately not part of a normal build or test. It consumes
local checkouts at the revisions recorded below, verifies their Git revisions
and the Vega Data Package hashes, and writes deterministic fixture artifacts.

Usage:
    uv run --python 3.12 sync_gallery.py \
      --vega-lite-root /path/to/vega-lite \
      --vega-datasets-root /path/to/vega-datasets
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import json
import math
import shutil
import subprocess
import sys
from collections import OrderedDict
from pathlib import Path
from typing import Any, Iterable
from urllib.parse import urlparse

import pyarrow as pa
import pyarrow.parquet as pq
import shapely
from dateutil import parser as date_parser
from shapely.geometry import shape


VEGA_LITE_VERSION = "6.4.3"
VEGA_LITE_REVISION = "c4ae590b77bac6e44c20aa432e39cd53a6710287"
VEGA_DATASETS_VERSION = "3.2.1"
VEGA_DATASETS_REVISION = "dedfc126e87dfde2df0332744689844314911d5d"
EXPECTED_PLACEMENTS = 203
EXPECTED_EXAMPLES = 189
EXPECTED_DATA_PATHS = 43
EXPECTED_INLINE_DATASETS = 4

TOPOLOGY_RELATIONS = {
    ("us-10m.json", "states"): "us_states",
    ("us-10m.json", "counties"): "us_counties",
    ("world-110m.json", "countries"): "world_countries",
    ("londonBoroughs.json", "boroughs"): "london_boroughs",
    ("londonTubeLines.json", "line"): "london_tube_lines",
}

PROPERTY_RELATIONS = {
    ("earthquakes.json", "features"): "earthquake_features",
}

KNOWN_MISSING_FEATURES = {
    "mark:arc": "AV-GALLERY-MARK-001",
    "transform:regression": "AV-GALLERY-TRANSFORM-001",
    "transform:loess": "AV-GALLERY-TRANSFORM-002",
    "transform:quantile": "AV-GALLERY-TRANSFORM-003",
    "transform:flatten": "AV-GALLERY-TRANSFORM-004",
    "data:sequence": "AV-GALLERY-DATAFLOW-001",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vega-lite-root", type=Path, required=True)
    parser.add_argument("--vega-datasets-root", type=Path, required=True)
    parser.add_argument(
        "--output-root",
        type=Path,
        default=Path(__file__).resolve().parent,
    )
    return parser.parse_args()


def git_revision(root: Path) -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def require_revision(root: Path, expected: str, label: str) -> None:
    actual = git_revision(root)
    if actual != expected:
        raise RuntimeError(f"{label} checkout is {actual}; expected {expected}")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=False, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def verify_data_package_hash(path: Path, expected: str | None) -> None:
    if not expected:
        raise RuntimeError(f"{path.name} has no pinned Data Package hash")
    algorithm, digest = expected.split(":", 1)
    if algorithm != "sha1":
        raise RuntimeError(f"unsupported Data Package hash {expected!r}")
    contents = path.read_bytes()
    # Vega's Data Package records the Git object ID, not the raw file SHA-1.
    actual = hashlib.sha1(
        f"blob {len(contents)}\0".encode("ascii") + contents
    ).hexdigest()
    if actual != digest:
        raise RuntimeError(
            f"{path.name} SHA-1 mismatch: got {actual}, expected {digest}"
        )


def normalize_identifier(value: str) -> str:
    result = []
    for character in value:
        if character.isalnum():
            result.append(character.lower())
        elif not result or result[-1] != "_":
            result.append("_")
    return "".join(result).strip("_")


def source_path_from_url(url: str) -> str:
    parsed = urlparse(url)
    path = parsed.path if parsed.scheme else url
    marker = "/data/"
    if marker in path:
        return path.rsplit(marker, 1)[1]
    if path.startswith("data/"):
        return path.removeprefix("data/")
    raise RuntimeError(f"gallery data URL is outside the Vega data namespace: {url}")


def relation_for(source_path: str, data_format: dict[str, Any] | None) -> str:
    data_format = data_format or {}
    if data_format.get("type") == "topojson":
        feature = data_format.get("feature")
        try:
            return TOPOLOGY_RELATIONS[(source_path, feature)]
        except KeyError as error:
            raise RuntimeError(
                f"no stable relation name for TopoJSON {source_path}#{feature}"
            ) from error
    if data_format.get("property"):
        property_name = data_format["property"]
        return PROPERTY_RELATIONS.get(
            (source_path, property_name),
            f"{normalize_identifier(Path(source_path).stem)}_{normalize_identifier(property_name)}",
        )
    return normalize_identifier(Path(source_path).stem)


def iter_placements(manifest: dict[str, Any]) -> Iterable[dict[str, Any]]:
    position = 0
    for section, subsections in manifest.items():
        for subsection, examples in subsections.items():
            for subsection_index, example in enumerate(examples):
                yield {
                    "position": position,
                    "section": section,
                    "subsection": subsection,
                    "subsection_index": subsection_index,
                    "name": example["name"],
                    "title": example.get("title"),
                    "description": example.get("description"),
                }
                position += 1


def walk(value: Any) -> Iterable[Any]:
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def data_references(spec: dict[str, Any]) -> list[dict[str, Any]]:
    references: OrderedDict[tuple[str, str], dict[str, Any]] = OrderedDict()
    for value in walk(spec):
        if not isinstance(value, dict) or not isinstance(value.get("url"), str):
            continue
        url = value["url"]
        if not (url.startswith("data/") or "/data/" in url):
            continue
        source_path = source_path_from_url(url)
        data_format = value.get("format") or {}
        key = (source_path, json.dumps(data_format, sort_keys=True))
        references.setdefault(
            key,
            {
                "url": url,
                "source_path": source_path,
                "format": data_format or None,
                "relation": relation_for(source_path, data_format),
            },
        )
    return list(references.values())


def inline_assets(spec: dict[str, Any]) -> list[str]:
    assets = set()
    for value in walk(spec):
        if (
            isinstance(value, str)
            and value.startswith("data/")
            and value.endswith(".png")
        ):
            assets.add(value.removeprefix("data/"))
    return sorted(assets)


def feature_inventory(spec: dict[str, Any]) -> list[str]:
    features: set[str] = set()
    for value in walk(spec):
        if not isinstance(value, dict):
            continue
        mark = value.get("mark")
        if isinstance(mark, str):
            features.add(f"mark:{mark}")
        elif isinstance(mark, dict) and isinstance(mark.get("type"), str):
            features.add(f"mark:{mark['type']}")
        transform = value.get("transform")
        if isinstance(transform, list):
            for item in transform:
                if not isinstance(item, dict):
                    continue
                for kind in (
                    "aggregate",
                    "bin",
                    "calculate",
                    "density",
                    "filter",
                    "flatten",
                    "fold",
                    "impute",
                    "joinaggregate",
                    "loess",
                    "lookup",
                    "pivot",
                    "quantile",
                    "regression",
                    "stack",
                    "timeUnit",
                    "window",
                ):
                    if kind in item:
                        features.add(f"transform:{kind}")
        for composition in ("facet", "repeat", "hconcat", "vconcat", "concat", "layer"):
            if composition in value:
                features.add(f"composition:{composition}")
        if "projection" in value:
            features.add("coordinate:geo")
        if isinstance(value.get("params"), list):
            features.add("interaction:param")
            if any(
                isinstance(item, dict) and "select" in item for item in value["params"]
            ):
                features.add("interaction:selection")
        data = value.get("data")
        if isinstance(data, dict) and "sequence" in data:
            features.add("data:sequence")
    return sorted(features)


def build_gallery_manifest(
    vega_lite_root: Path,
    placements: list[dict[str, Any]],
    existing: dict[str, Any] | None,
) -> tuple[dict[str, Any], set[str], set[str], dict[str, dict[str, Any]]]:
    specs_root = vega_lite_root / "examples/specs"
    references_root = vega_lite_root / "examples/compiled"
    metadata: OrderedDict[str, dict[str, Any]] = OrderedDict()
    for placement in placements:
        metadata.setdefault(
            placement["name"],
            {
                "title": placement.get("title"),
                "description": placement.get("description"),
            },
        )
    if len(placements) != EXPECTED_PLACEMENTS or len(metadata) != EXPECTED_EXAMPLES:
        raise RuntimeError(
            f"gallery census changed: {len(placements)} placements, {len(metadata)} examples"
        )

    old_entries = {
        entry["name"]: entry for entry in (existing or {}).get("examples", [])
    }
    source_paths: set[str] = set()
    asset_paths: set[str] = set()
    inline_datasets: dict[str, dict[str, Any]] = {}
    examples = []
    for name, descriptive in metadata.items():
        spec_path = specs_root / f"{name}.vl.json"
        reference_path = references_root / f"{name}.png"
        if not spec_path.is_file() or not reference_path.is_file():
            raise RuntimeError(f"missing pinned spec/reference for {name}")
        spec = read_json(spec_path)
        references = data_references(spec)
        assets = inline_assets(spec)
        named_datasets = spec.get("datasets") or {}
        if not isinstance(named_datasets, dict):
            raise RuntimeError(f"{name} has a non-object datasets declaration")
        for dataset_name, values in named_datasets.items():
            if dataset_name in inline_datasets:
                previous = inline_datasets[dataset_name]["example"]
                raise RuntimeError(
                    f"inline dataset {dataset_name!r} is declared by both "
                    f"{previous} and {name}"
                )
            if not isinstance(values, list) or not values:
                raise RuntimeError(
                    f"{name} inline dataset {dataset_name!r} must be a non-empty array"
                )
            inline_datasets[dataset_name] = {
                "example": name,
                "values": values,
            }
        source_paths.update(reference["source_path"] for reference in references)
        asset_paths.update(assets)
        features = feature_inventory(spec)
        old = old_entries.get(name, {})
        blockers = old.get("blockers")
        if blockers is None:
            blockers = [
                {
                    "id": KNOWN_MISSING_FEATURES[feature],
                    "feature": feature,
                }
                for feature in features
                if feature in KNOWN_MISSING_FEATURES
            ]
        examples.append(
            {
                "name": name,
                "title": descriptive.get("title"),
                "description": descriptive.get("description"),
                "upstream_spec": f"provenance/specs/{name}.vl.json",
                "upstream_spec_sha256": sha256_file(spec_path),
                "upstream_reference": f"reference/{name}.png",
                "upstream_reference_sha256": sha256_file(reference_path),
                "chart": f"charts/{name}.avenger",
                "data": references,
                "datasets": sorted(
                    {
                        *(reference["relation"] for reference in references),
                        *named_datasets,
                    }
                ),
                "assets": assets,
                "features": features,
                "implementation": old.get("implementation", "unported"),
                "blockers": blockers,
                "review": old.get("review", "unreviewed"),
                "accepted_differences": old.get("accepted_differences", []),
            }
        )
    return (
        {
            "schema_version": 1,
            "upstream": {
                "vega_lite_version": VEGA_LITE_VERSION,
                "vega_lite_revision": VEGA_LITE_REVISION,
            },
            "counts": {
                "placements": len(placements),
                "examples": len(examples),
            },
            "placements": placements,
            "examples": examples,
        },
        source_paths,
        asset_paths,
        inline_datasets,
    )


def decode_topology_arc(topology: dict[str, Any], arc_index: int) -> list[list[float]]:
    reverse = arc_index < 0
    source_index = ~arc_index if reverse else arc_index
    arc = topology["arcs"][source_index]
    transform = topology.get("transform")
    x = 0.0
    y = 0.0
    decoded = []
    for point in arc:
        x += point[0]
        y += point[1]
        if transform:
            decoded.append(
                [
                    x * transform["scale"][0] + transform["translate"][0],
                    y * transform["scale"][1] + transform["translate"][1],
                    *point[2:],
                ]
            )
        else:
            decoded.append([x, y, *point[2:]])
    if reverse:
        decoded.reverse()
    return decoded


def stitch_arcs(topology: dict[str, Any], indexes: list[int]) -> list[list[float]]:
    coordinates: list[list[float]] = []
    for index in indexes:
        arc = decode_topology_arc(topology, index)
        if coordinates and arc and coordinates[-1][:2] == arc[0][:2]:
            coordinates.extend(arc[1:])
        else:
            coordinates.extend(arc)
    return coordinates


def decode_point(topology: dict[str, Any], point: list[float]) -> list[float]:
    transform = topology.get("transform")
    if not transform:
        return point
    return [
        point[0] * transform["scale"][0] + transform["translate"][0],
        point[1] * transform["scale"][1] + transform["translate"][1],
        *point[2:],
    ]


def decode_topology_geometry(
    topology: dict[str, Any], geometry: dict[str, Any]
) -> dict[str, Any] | None:
    kind = geometry.get("type")
    if kind is None:
        return None
    if kind == "Point":
        coordinates = decode_point(topology, geometry["coordinates"])
    elif kind == "MultiPoint":
        coordinates = [
            decode_point(topology, point) for point in geometry["coordinates"]
        ]
    elif kind == "LineString":
        coordinates = stitch_arcs(topology, geometry["arcs"])
    elif kind == "MultiLineString":
        coordinates = [stitch_arcs(topology, line) for line in geometry["arcs"]]
    elif kind == "Polygon":
        coordinates = [stitch_arcs(topology, ring) for ring in geometry["arcs"]]
    elif kind == "MultiPolygon":
        coordinates = [
            [stitch_arcs(topology, ring) for ring in polygon]
            for polygon in geometry["arcs"]
        ]
    else:
        raise RuntimeError(f"unsupported TopoJSON geometry type {kind!r}")
    return {"type": kind, "coordinates": coordinates}


def topology_features(
    topology: dict[str, Any], object_name: str
) -> list[dict[str, Any]]:
    try:
        root = topology["objects"][object_name]
    except KeyError as error:
        raise RuntimeError(f"missing TopoJSON object {object_name!r}") from error
    geometries = (
        root.get("geometries") if root.get("type") == "GeometryCollection" else [root]
    )
    return [
        {
            "type": "Feature",
            "id": geometry.get("id"),
            "properties": geometry.get("properties") or {},
            "geometry": decode_topology_geometry(topology, geometry),
        }
        for geometry in geometries
    ]


def ring_area(ring: list[list[float]]) -> float:
    if len(ring) < 4:
        return 0.0
    return abs(
        sum(
            first[0] * second[1] - second[0] * first[1]
            for first, second in zip(ring, ring[1:])
        )
        / 2.0
    )


def sanitize_geojson_geometry(geometry: dict[str, Any]) -> dict[str, Any]:
    """Match Avenger's ingest rule by dropping zero-area polygon rings."""

    kind = geometry.get("type")
    if kind == "Polygon":
        rings = geometry.get("coordinates", [])
        if not rings or ring_area(rings[0]) < 1e-10:
            return {"type": "MultiPolygon", "coordinates": []}
        return {
            **geometry,
            "coordinates": [
                rings[0],
                *[ring for ring in rings[1:] if ring_area(ring) >= 1e-10],
            ],
        }
    if kind == "MultiPolygon":
        polygons = []
        for rings in geometry.get("coordinates", []):
            if not rings or ring_area(rings[0]) < 1e-10:
                continue
            polygons.append(
                [rings[0], *[ring for ring in rings[1:] if ring_area(ring) >= 1e-10]]
            )
        return {**geometry, "coordinates": polygons}
    return geometry


def geo_rows(features: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows = []
    for feature in features:
        properties = dict(feature.get("properties") or {})
        if feature.get("id") is not None and "id" not in properties:
            properties["id"] = feature["id"]
        geometry_value = feature.get("geometry")
        if geometry_value is None:
            geometry = None
            bounds = (None, None, None, None)
        else:
            geometry = shape(sanitize_geojson_geometry(geometry_value))
            if not geometry.is_empty:
                geometry = shapely.orient_polygons(geometry, exterior_cw=True)
                bounds = geometry.bounds
            else:
                bounds = (None, None, None, None)
        row = {
            "geometry": None if geometry is None else shapely.to_wkb(geometry),
            "bbox_xmin": bounds[0],
            "bbox_ymin": bounds[1],
            "bbox_xmax": bounds[2],
            "bbox_ymax": bounds[3],
            **properties,
        }
        if geometry_value and geometry_value.get("type") == "Point":
            coordinates = geometry_value.get("coordinates", [])
            if len(coordinates) >= 2:
                row.setdefault("longitude", coordinates[0])
                row.setdefault("latitude", coordinates[1])
            if len(coordinates) >= 3:
                row.setdefault("depth", coordinates[2])
        rows.append(row)
    return rows


def schema_fields(resource: dict[str, Any]) -> list[dict[str, Any]]:
    return resource.get("schema", {}).get("fields", [])


def parse_date(value: Any) -> dt.date | None:
    if value in (None, ""):
        return None
    return date_parser.parse(str(value).strip()).date()


def parse_datetime(value: Any) -> dt.datetime | None:
    if value in (None, ""):
        return None
    parsed = date_parser.parse(str(value).strip())
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.timezone.utc)
    return parsed.astimezone(dt.timezone.utc)


def coerce_value(value: Any, field_type: str) -> Any:
    if value in (None, ""):
        return None
    if field_type == "integer":
        return int(float(value))
    if field_type == "number":
        number = float(value)
        return None if math.isnan(number) else number
    if field_type == "date":
        return parse_date(value)
    if field_type == "datetime":
        return parse_datetime(value)
    if field_type == "boolean":
        if isinstance(value, bool):
            return value
        normalized = str(value).strip().lower()
        if normalized in ("true", "1"):
            return True
        if normalized in ("false", "0"):
            return False
        raise ValueError(f"invalid boolean {value!r}")
    if isinstance(value, (dict, list)):
        return json.dumps(value, sort_keys=True, separators=(",", ":"))
    return str(value)


def arrow_type(field_type: str) -> pa.DataType:
    return {
        "integer": pa.int64(),
        "number": pa.float64(),
        "date": pa.date32(),
        "datetime": pa.timestamp("ms", tz="UTC"),
        "boolean": pa.bool_(),
        "string": pa.string(),
    }[field_type]


def infer_field_type(values: list[Any]) -> str:
    present = [value for value in values if value is not None and value != ""]
    if not present:
        return "string"
    if all(isinstance(value, bool) for value in present):
        return "boolean"
    if all(isinstance(value, int) and not isinstance(value, bool) for value in present):
        return "integer"
    if all(
        isinstance(value, (int, float)) and not isinstance(value, bool)
        for value in present
    ):
        return "number"
    return "string"


def table_from_rows(rows: list[dict[str, Any]], resource: dict[str, Any]) -> pa.Table:
    declared = {field["name"]: field["type"] for field in schema_fields(resource)}
    names = list(declared)
    discovered = sorted({name for row in rows for name in row if name not in declared})
    names.extend(discovered)
    types = {
        name: declared.get(name) or infer_field_type([row.get(name) for row in rows])
        for name in names
    }
    schema = pa.schema(
        [pa.field(name, arrow_type(types[name]), nullable=True) for name in names]
    )
    normalized = [
        {name: coerce_value(row.get(name), types[name]) for name in names}
        for row in rows
    ]
    return pa.Table.from_pylist(normalized, schema=schema)


def read_tabular_rows(path: Path, resource: dict[str, Any]) -> list[dict[str, Any]]:
    if path.suffix in (".csv", ".tsv"):
        delimiter = "\t" if path.suffix == ".tsv" else ","
        with path.open(newline="", encoding="utf-8") as stream:
            return list(csv.DictReader(stream, delimiter=delimiter))
    value = read_json(path)
    if not isinstance(value, list):
        raise RuntimeError(f"{path.name} is not a tabular JSON array")
    if not all(isinstance(row, dict) for row in value):
        raise RuntimeError(f"{path.name} does not contain object rows")
    return value


def build_relations(
    dataset_root: Path,
    resources: dict[str, dict[str, Any]],
    examples: list[dict[str, Any]],
    inline_datasets: dict[str, dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    declarations: OrderedDict[str, tuple[str, dict[str, Any] | None]] = OrderedDict()
    for example in examples:
        for reference in example["data"]:
            declarations.setdefault(
                reference["relation"],
                (reference["source_path"], reference.get("format")),
            )
    relations = {}
    for relation, (source_path, data_format) in sorted(declarations.items()):
        source = dataset_root / "data" / source_path
        resource = resources[source_path]
        data_format = data_format or {}
        if data_format.get("type") == "topojson":
            rows = geo_rows(
                topology_features(read_json(source), data_format["feature"])
            )
        elif data_format.get("property"):
            value = read_json(source)
            for component in data_format["property"].split("."):
                value = value[component]
            if (
                value
                and isinstance(value[0], dict)
                and value[0].get("type") == "Feature"
            ):
                rows = geo_rows(value)
            else:
                rows = value
        else:
            rows = read_tabular_rows(source, resource)
        table = table_from_rows(rows, resource)
        relations[relation] = {
            "source_path": source_path,
            "format": data_format or None,
            "rows": table.num_rows,
            "schema": [
                {
                    "name": field.name,
                    "type": str(field.type),
                    "nullable": field.nullable,
                }
                for field in table.schema
            ],
            "table": table,
        }
    for relation, declaration in sorted(inline_datasets.items()):
        if relation in relations:
            raise RuntimeError(
                f"inline dataset {relation!r} conflicts with a Vega dataset relation"
            )
        values = declaration["values"]
        if all(isinstance(value, dict) for value in values):
            rows = values
        elif all(not isinstance(value, dict) for value in values):
            rows = [{"data": value} for value in values]
        else:
            raise RuntimeError(
                f"inline dataset {relation!r} mixes object and primitive rows"
            )
        table = table_from_rows(rows, {})
        example = declaration["example"]
        relations[relation] = {
            "source_path": f"examples/specs/{example}.vl.json#datasets.{relation}",
            "format": {"type": "inline_named_dataset"},
            "rows": table.num_rows,
            "schema": [
                {
                    "name": field.name,
                    "type": str(field.type),
                    "nullable": field.nullable,
                }
                for field in table.schema
            ],
            "table": table,
        }
    return relations


def write_catalog(output_root: Path, relations: dict[str, dict[str, Any]]) -> None:
    lines = ["avenger 1;", "export schema tables as vega {"]
    for name in sorted(relations):
        lines.extend(
            [
                f"  table parquet as {name} {{",
                f"    path: 'data/{name}.parquet';",
                "  }",
                "",
            ]
        )
    lines.append("}")
    (output_root / "catalog.avenger").write_text("\n".join(lines) + "\n")


def markdown_link(title: str, path: str | None) -> str:
    title = " ".join(title.split()).replace("|", "\\|")
    return f"[{title}]({path})" if path else title


def write_third_party_data(
    output_root: Path,
    resources: dict[str, dict[str, Any]],
    paths: set[str],
) -> None:
    lines = [
        "# Third-Party Vega Gallery Data",
        "",
        "This inventory is generated from the pinned Vega datasets 3.2.1 Data Package.",
        "The repository-level BSD-3-Clause license covers package code and infrastructure,",
        "not every dataset. The upstream metadata is a reference starting point and does",
        "not guarantee that a particular downstream use is permitted.",
        "",
    ]
    for path in sorted(paths):
        resource = resources[path]
        lines.extend([f"## `{path}`", ""])
        if description := resource.get("description"):
            lines.extend([" ".join(description.split()), ""])
        lines.append(f"- Data Package name: `{resource.get('name', '')}`")
        lines.append(f"- Format: `{resource.get('format', '')}`")
        lines.append(f"- Git-blob hash: `{resource.get('hash', '')}`")
        licenses = resource.get("licenses") or []
        if licenses:
            rendered = ", ".join(
                markdown_link(
                    license.get("title") or license.get("name") or "unspecified",
                    license.get("path"),
                )
                for license in licenses
            )
            lines.append(f"- Licenses: {rendered}")
        else:
            lines.append("- Licenses: not specified in the pinned Data Package")
        sources = resource.get("sources") or []
        if sources:
            rendered = ", ".join(
                markdown_link(
                    source.get("title") or source.get("path") or "source",
                    source.get("path"),
                )
                for source in sources
            )
            lines.append(f"- Sources: {rendered}")
        else:
            lines.append("- Sources: not specified in the pinned Data Package")
        lines.append("")
    (output_root / "provenance/THIRD_PARTY_DATA.md").write_text(
        "\n".join(lines), encoding="utf-8"
    )


def main() -> None:
    args = parse_args()
    output_root = args.output_root.resolve()
    vega_lite_root = args.vega_lite_root.resolve()
    dataset_root = args.vega_datasets_root.resolve()
    require_revision(vega_lite_root, VEGA_LITE_REVISION, "Vega-Lite")
    require_revision(dataset_root, VEGA_DATASETS_REVISION, "Vega datasets")

    gallery_source = read_json(vega_lite_root / "site/_data/examples.json")
    placements = list(iter_placements(gallery_source))
    existing_path = output_root / "gallery.json"
    existing = read_json(existing_path) if existing_path.exists() else None
    gallery, source_paths, asset_paths, inline_datasets = build_gallery_manifest(
        vega_lite_root, placements, existing
    )
    if len(source_paths) != EXPECTED_DATA_PATHS:
        raise RuntimeError(
            f"gallery data closure changed: got {len(source_paths)}, expected {EXPECTED_DATA_PATHS}"
        )
    if len(inline_datasets) != EXPECTED_INLINE_DATASETS:
        raise RuntimeError(
            "gallery inline dataset closure changed: "
            f"got {len(inline_datasets)}, expected {EXPECTED_INLINE_DATASETS}"
        )

    data_package_path = dataset_root / "datapackage.json"
    data_package = read_json(data_package_path)
    resources = {resource["path"]: resource for resource in data_package["resources"]}
    missing = sorted((source_paths | asset_paths) - resources.keys())
    if missing:
        raise RuntimeError(f"gallery resources missing from Data Package: {missing}")
    for source_path in sorted(source_paths | asset_paths):
        verify_data_package_hash(
            dataset_root / "data" / source_path,
            resources[source_path].get("hash"),
        )

    relations = build_relations(
        dataset_root, resources, gallery["examples"], inline_datasets
    )

    for path in (
        output_root / "data",
        output_root / "assets",
        output_root / "reference",
        output_root / "provenance/specs",
    ):
        path.mkdir(parents=True, exist_ok=True)

    relation_manifest = []
    for name, relation in sorted(relations.items()):
        output = output_root / "data" / f"{name}.parquet"
        pq.write_table(
            relation.pop("table"),
            output,
            compression="zstd",
            version="2.6",
            write_statistics=True,
        )
        relation["path"] = f"data/{name}.parquet"
        relation["sha256"] = sha256_file(output)
        relation_manifest.append({"name": name, **relation})

    for asset_path in sorted(asset_paths):
        shutil.copyfile(
            dataset_root / "data" / asset_path,
            output_root / "assets" / asset_path,
        )

    for example in gallery["examples"]:
        name = example["name"]
        shutil.copyfile(
            vega_lite_root / "examples/specs" / f"{name}.vl.json",
            output_root / "provenance/specs" / f"{name}.vl.json",
        )
        shutil.copyfile(
            vega_lite_root / "examples/compiled" / f"{name}.png",
            output_root / "reference" / f"{name}.png",
        )

    shutil.copyfile(data_package_path, output_root / "provenance/datapackage.json")
    shutil.copyfile(
        vega_lite_root / "LICENSE",
        output_root / "provenance/VEGA_LITE_LICENSE",
    )
    write_json(output_root / "gallery.json", gallery)
    write_json(
        output_root / "upstream.lock.json",
        {
            "schema_version": 1,
            "vega_lite": {
                "version": VEGA_LITE_VERSION,
                "revision": VEGA_LITE_REVISION,
                "repository": "https://github.com/vega/vega-lite",
                "placements": EXPECTED_PLACEMENTS,
                "examples": EXPECTED_EXAMPLES,
            },
            "vega_datasets": {
                "version": VEGA_DATASETS_VERSION,
                "revision": VEGA_DATASETS_REVISION,
                "repository": "https://github.com/vega/vega-datasets",
                "gallery_source_paths": EXPECTED_DATA_PATHS,
            },
        },
    )
    write_json(
        output_root / "provenance/source-manifest.json",
        {
            "schema_version": 1,
            "sources": [
                {
                    **{
                        key: value
                        for key, value in resources[path].items()
                        if key != "schema"
                    },
                    "sha256": sha256_file(dataset_root / "data" / path),
                }
                for path in sorted(source_paths | asset_paths)
            ],
            "relations": relation_manifest,
        },
    )
    write_third_party_data(output_root, resources, source_paths | asset_paths)
    write_catalog(output_root, relations)
    print(
        f"wrote {len(gallery['examples'])} examples, {len(source_paths)} data sources, "
        f"{len(relations)} relations ({len(inline_datasets)} inline), and "
        f"{len(asset_paths)} assets to {output_root}"
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"sync failed: {error}", file=sys.stderr)
        raise
