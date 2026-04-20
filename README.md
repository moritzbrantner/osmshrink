# osmshrink

`osmshrink` downloads OpenStreetMap regional extracts, filters nodes, ways, and
area relations from `.osm.pbf` files, and writes normalized `.ndjson`, `.json`,
or `.geojson` output for downstream data pipelines.

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

`fetch` reuses the output file when it already exists, so repeating the same
command does not download the extract again. Use `--force` to refresh it:

```bash
cargo run -- fetch geofabrik:europe/germany/baden-wuerttemberg --output data/bw.osm.pbf --force
```

Filter a local extract:

```bash
cargo run -- filter --input data/bw.osm.pbf --spec examples/schools.json --output out/schools.ndjson
cargo run -- filter --input data/bw.osm.pbf --spec examples/roads.yaml --format json --output out/roads.json
cargo run -- filter --input data/bw.osm.pbf --spec examples/areas_geojson.yaml --output out/areas.geojson
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

Use `--verbose` for more logging and `--quiet` to suppress status output. Use
`--index auto|memory|disk`, `--index-dir`, and `--memory-node-limit` to override
node index behavior from the spec.

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
    "types": ["node", "way", "relation"],
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
  "processing": {
    "index": {
      "mode": "auto",
      "memory_node_limit": 5000000,
      "disk_dir": null
    }
  },
  "output": {
    "format": "ndjson",
    "geometry": "full",
    "fields": ["id", "type", "tags", "geometry"]
  }
}
```

Supported filter features:

- `types`: `node`, `way`, and `relation`. If omitted, nodes and ways are used.
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
- relations match when any assembled polygon coordinate is inside the bbox.

Relation behavior:

- only area relations are emitted: `type=multipolygon` and `type=boundary`.
- relation members with role `outer`, role `inner`, or an empty role are used.
- other member roles and non-way relation members are ignored and counted.
- relations are skipped if required member ways or nodes are missing, rings do
  not close, or inner rings cannot be assigned to an outer ring.

Index behavior:

- `processing.index.mode` can be `auto`, `memory`, or `disk`.
- `auto` starts with an in-memory node coordinate index and spills to a temporary
  `redb` database after `memory_node_limit` nodes.
- `disk_dir` optionally chooses where temporary disk-backed indexes are created.

## Output

`ndjson` is the default and is preferred for large outputs. `json` writes one
array. `geojson` writes a GeoJSON `FeatureCollection`.

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
`Polygon`. Area relations emit `Polygon` or `MultiPolygon`.

For GeoJSON output, `output.fields` must include `geometry`. If `id` is included,
the GeoJSON feature id is formatted as `node/123`, `way/123`, or `relation/123`,
with `osm_id` added to properties. If `type` is included, `osm_type` is added to
properties. If `tags` is included, OSM tags are copied to properties.

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
- `filter`: predicate compilation, PBF filtering pipeline, and area relation assembly.
- `geometry`: bbox checks and point/way/relation geometry creation.
- `index`: in-memory, disk-backed, and auto-spilling node coordinate indexes.
- `model`: normalized output model.
- `output`: JSON, NDJSON, and GeoJSON writers.
- `inspect`: cheap filesystem-level input inspection.
- `error`: typed application errors.

## Limitations

- Way geometry requires referenced nodes to exist in the extract. A way with
  missing node coordinates is skipped with a warning.
- Relation support is limited to area relations (`multipolygon` and `boundary`).
  Other relation types are skipped and counted.
- Relation assembly supports way members only. Nested relations are ignored.
- Auto indexing can spill node coordinates to disk, but relation member ways are
  still retained in memory while assembling candidate area relations.

## Tests

```bash
cargo test
```

Tests cover Geofabrik shorthand parsing, JSON/YAML spec parsing, condition
matching, bbox logic, field filtering, GeoJSON output, node indexes, and
constructed-object relation assembly. Fetch caching is covered by unit,
integration, and CLI e2e tests using a local HTTP server.

## Benchmarks

Run the end-to-end filter benchmark:

```bash
cargo bench --bench filter
```

The benchmark generates a deterministic synthetic `.osm.pbf` fixture with
20,000 nodes and 5,000 matching ways, then measures the real `filter_pbf`
pipeline using the in-memory node index.

## Roadmap

- Python and Node bindings over the Rust library core.
- More relation types beyond area relations.
- Streaming relation member geometry storage for very large relation-heavy extracts.
