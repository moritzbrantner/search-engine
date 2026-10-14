# Search Engine

An embeddable, domain-neutral Rust search capability for typed metadata and optional text, independent of the NLP application.

## Current baseline

This bootstrap contains a deterministic in-memory **reference oracle**, not a production persistent index. It establishes query semantics and independent tests so a Tantivy-backed implementation can be validated rather than invented. Reference queries scan O(n) records.

```sh
cargo test --locked
cargo run --locked --example metadata_search
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

The example filters images by file type, pixel width/height, capture date and text. There is no model download, database, network service or sibling checkout.

## Boundaries

- **Search Engine:** typed indexed-document projections, mechanical query/range/boolean logic, lexical search, facets, pagination, index updates and reusable backend adapters.
- **Applications:** canonical assets and metadata, source ingestion, permissions, UI, and index lifecycle policy. Indexes are reconstructible derived state.
- **[Moenarch Foundation](https://github.com/moritzbrantner/moenarch-foundation):** corpus identity and provenance contracts.
- **[rust-kernels](https://github.com/moritzbrantner/rust-kernels):** reusable selection and similarity algorithms.
- **[nlp-stack](https://github.com/moritzbrantner/nlp-stack):** linguistic analysis, model-backed embeddings and NLP-specific relevance; never required for metadata filtering.
- **[media-intelligence](https://github.com/moritzbrantner/media-intelligence):** modality alignment and evidence/result fusion, not generic search storage.

The first production index backend should evaluate [Tantivy](https://github.com/quickwit-oss/tantivy) against the reference oracle. SQLite, Postgres and Qdrant may remain application-owned databases. No central/global search service is required.

## Authority

[Decision Atlas](docs/DECISION-ATLAS.md) records product decisions separately from implementation. Related bootstrap goal: [Media Intelligence #35](https://github.com/moritzbrantner/media-intelligence/issues/35). Follow-up implementation issues belong in this repository.

`conventions.json` selects live `rust-performance` policy; no managed `.conventions/` cache or convention lock was present when this empty repository was initialized. Install those only through `coding-tooling conventions update`, not by hand.
