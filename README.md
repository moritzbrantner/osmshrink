# osmshrink

`osmshrink` downloads OpenStreetMap regional extracts, filters nodes and ways from
`.osm.pbf` files, and writes normalized `.ndjson` or `.json` output for downstream
data pipelines.

The first supported download provider is Geofabrik. Sources can be direct URLs or
short names such as:

```bash
geofabrik:europe/germany/baden-wuerttemberg
```

which resolves to:

```text
https://download.geofabrik.de/europe/germany/baden-wuerttemberg-latest.osm.pbf
```

## Why

OSM extracts are rich but large. `osmshrink` is a small Rust CLI and reusable
library core for downloading an extract once, applying practical tag and bbox
filters, and exporting only the data needed by an application.

The implementation does not shell out to tools such as `osmium`.

## Install

From this repository:

```bash
cargo build --release
```

During development:

```bash
cargo build
```

## Usage

Download an extract:

```bash
cargo run -- fetch geofabrik:europe/germany/baden-wuerttemberg --output data/bw.osm.pbf
cargo run -- fetch https://download.geofabrik.de/europe/germany-latest.osm.pbf --output data/germany.osm.pbf
```

Filter a local extract:

```bash
cargo run -- filter --input data/bw.osm.pbf --spec examples/schools.json --output out/schools.ndjson
cargo run -- filter --input data/bw.osm.pbf --spec examples/roads.yaml --format json --output out/roads.json
```

Fetch and filter in one command:

```bash
cargo run -- run \
  --source geofabrik:europe/germany/baden-wuerttemberg \
  --spec examples/schools.json \
  --output out/schools.ndjson
```

Inspect an input file:

```bash
cargo run -- inspect --input data/bw.osm.pbf
```

Validate a spec:

```bash
cargo run -- validate-spec --spec examples/schools.json
```

Use `--verbose` for more logging and `--quiet` to suppress status output.

## Filter Specs

Specs can be JSON or YAML. A typical spec looks like:

```json
{
  "source": {
    "provider": "geofabrik",
    "region": "europe/germany/baden-wuerttemberg"
  },
  "filter": {
    "bbox": [8.5, 48.8, 9.3, 49.2],
    "types": ["node", "way"],
    "include": {
      "any": [
        { "key": "amenity", "values": ["school", "hospital"] },
        { "key": "highway", "values": ["primary", "secondary", "tertiary"] }
      ],
      "all": [
        { "key": "name", "exists": true }
      ]
    },
    "exclude": [
      { "key": "access", "values": ["private"] }
    ]
  },
  "output": {
    "format": "ndjson",
    "geometry": "full",
    "fields": ["id", "type", "tags", "geometry"]
  }
}
```

Supported filter features:

- `types`: `node` and `way`. If omitted, both are used.
- `bbox`: `[min_lon, min_lat, max_lon, max_lat]`.
- `include.any`: at least one condition must match.
- `include.all`: every condition must match.
- `exclude`: if any condition matches, the object is dropped.

Supported condition operators:

- `{ "key": "name", "exists": true }`
- `{ "key": "amenity", "value": "school" }`
- `{ "key": "amenity", "values": ["school", "hospital"] }`
- `{ "key": "name", "regex": "School|Hospital" }`
- add `"not": true` to invert a condition.

If `include` is omitted, objects pass tag filtering unless excluded. If both
`include.any` and `include.all` are present, both groups must pass.

BBox behavior:

- nodes match by their own coordinates.
- ways match when any resolved node coordinate is inside the bbox.

## Output

`ndjson` is the default and is preferred for large outputs. `json` writes one
array.

Each emitted object has a normalized shape:

```json
{
  "id": 123,
  "type": "way",
  "tags": {
    "highway": "primary",
    "name": "B10"
  },
  "geometry": {
    "type": "LineString",
    "coordinates": [
      [8.70, 48.89],
      [8.71, 48.90]
    ]
  }
}
```

Nodes emit `Point` geometry. Ways emit `LineString` by default. If
`output.geometry` is `polygon`, closed ways with at least four coordinates emit a
`Polygon`.

`output.fields` controls which fields are written, for example:

```json
["id", "geometry"]
```

## Architecture

The crate is split into a reusable library and a CLI:

- `cli`: clap command definitions.
- `fetch`: streaming HTTP download to a temporary file, then rename on success.
- `geofabrik`: shorthand parsing and URL resolution.
- `spec`: serde models and validation for JSON/YAML specs.
- `filter`: predicate compilation and PBF filtering pipeline.
- `geometry`: bbox checks and point/way geometry creation.
- `model`: normalized output model.
- `output`: JSON and NDJSON writers.
- `inspect`: cheap filesystem-level input inspection.
- `error`: typed application errors.

## Limitations

- Relations are explicitly rejected in v1.
- Multipolygon assembly is not implemented.
- Way geometry requires referenced nodes to exist in the extract. A way with
  missing node coordinates is skipped with a warning.
- The current implementation keeps a node coordinate index in memory so way
  geometry can be resolved. This is suitable for medium regional extracts, but
  very large extracts may need a disk-backed index in the future.
- Output is normalized JSON/NDJSON, not GeoJSON.

## Tests

```bash
cargo test
```

Tests cover Geofabrik shorthand parsing, JSON/YAML spec parsing, condition
matching, bbox logic, and field filtering. A binary fixture test is intentionally
not included yet because maintaining a tiny real `.osm.pbf` fixture adds
repository weight and brittleness.

## Roadmap

- Relation and multipolygon support.
- GeoJSON export mode.
- Streaming or disk-backed node indexes for larger extracts.
- Python and Node bindings over the Rust library core.
