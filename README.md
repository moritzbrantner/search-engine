# Search Engine

Embeddable Rust search across consumer-owned typed metadata and optional text.
No source file crawling, search server, NLP runtime, authorization policy, or
canonical record database is required.

## Backends

- `ReferenceIndex`: O(n) reference oracle for independent correctness tests.
- `indexed::TantivyIndex`: durable Tantivy posting-list index with
  keyword/text/integer/boolean/timestamp-millisecond projection, nested Boolean
  filters, exact matching, inclusive ranges, missing-field semantics, stable
  sorting, pagination and facets.
- Search queries return deterministic IDs and revisions; applications retain
  source identity/URI and any user-facing presentation. For the index's
  reconstructed projection, call `document(id)`. Source bytes remain elsewhere.

A caller supplies a `Schema` and an explicit index directory. The Tantivy schema
(including layout version) must match on reopen. Schema changes require a
new directory and a rebuild from canonical application records.

### Index lifecycle

- `upsert`, `delete` and `apply_batch` commit atomically at the batch boundary.
  Equal identical revisions are no-ops; stale/conflicting revisions fail.
- Committed deletion tombstones survive reopen and `rebuild`. New writes at or
  below the deletion revision cannot resurrect an obsolete source.
  `latest_revision(id)` reads the committed revision of either a live record or a
  deletion tombstone; unknown IDs return `None`. This is the caller-supplied index
  revision, not a source-file version discovered by the engine.
- `rebuild` expects a complete caller-owned live snapshot and explicit revisioned
  deletion events for previously active sources absent from that snapshot.
  Previously committed tombstones may be omitted and are retained. An
  incomplete snapshot fails before any mutation.
- The index stores derived lifecycle records, not original file bytes. One
  process holds the writer lock for a directory. After an ambiguous indexing
  I/O error, discard the writer and reopen before retrying.
- Indexable predicates use inverted postings; matching candidate documents
  are materialized for exact substring verification, sorting and facet counts.
  Text contains uses Unicode lowercase and 1–3 character ngrams, with exact
  substring verification to avoid trigram false positives. Keyword equality
  and stable IDs are case-sensitive. No stemming, relevance ranking or
  linguistic inference is implied.
- `SearchWork.candidate_documents` counts materialized posting-list
  candidates, **not** Tantivy's internal term enumeration or total CPU work.
  Unselective queries can still visit the full corpus.
- Maximum page size is 1,000 and maximum offset is 1,000,000.

## Local CLI acceptance

The committed `fixtures/mixed-files.json` contains source-backed file
projections with distinct creation, modification and capture timestamps,
pixel dimensions, extension/MIME, path and optional content. Missing metadata
is absent, never filled using other timestamps.

```sh
cargo test --locked
cargo run --locked --example metadata_search

INDEX="$(mktemp -d)"
cargo run --locked --example indexed_search -- "$INDEX" ingest
cargo run --locked --example indexed_search -- "$INDEX" query alpine
cargo run --locked --example indexed_search -- "$INDEX" mutate
cargo run --locked --example indexed_search -- "$INDEX" query
cargo run --locked --example indexed_search -- "$INDEX" rebuild
cargo run --locked --example indexed_search -- "$INDEX" query

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

The CLI emits JSON with input predicate, stable source IDs, indexed metadata,
facets and candidate counters. Each invocation opens/closes the persistent
index so the sequence verifies actual restart behavior.

## Measurements

```sh
cargo run --release --locked --example bench_indexed
```

The named synthetic workloads use 100, 1,000 and 5,000 records. Output includes
actual wall-clock indexing time, index query time, O(n) reference time,
candidate counts, OS, architecture and build profile. These single-run
measurements are **informational**, not portable performance guarantees or
blocking CI timing budgets. The focused candidate-count regression lives in
`tests/tantivy_index.rs`.

## Boundaries and authority

- Search Engine: typed projections, posting-list retrieval, Boolean/range
  mechanics, sorting, facets and durable rebuildable index lifecycle.
- Applications: canonical assets/metadata, permissions, ingestion, UI,
  index lifecycle policy and source provenance identity.
- [Moenarch Foundation](https://github.com/moritzbrantner/moenarch-foundation):
  shared corpus identity/provenance.
- [rust-kernels](https://github.com/moritzbrantner/rust-kernels):
  lower-level reusable algorithms.
- [nlp-stack](https://github.com/moritzbrantner/nlp-stack):
  linguistic/embedding capabilities; not needed for metadata filtering.
- [media-intelligence](https://github.com/moritzbrantner/media-intelligence):
  multimodal evidence alignment and fusion.

The [Decision Atlas](docs/DECISION-ATLAS.md) records approved owner intent,
separate from implementation and acceptance. `conventions.json`
selects live `rust-performance`. Generate managed policy files only through
`coding-tooling conventions update`, and verify with
`coding-tooling conventions check`. Repository instructions: [AGENTS.md](AGENTS.md).
