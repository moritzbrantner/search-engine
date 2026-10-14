# Repository agent instructions

Read `README.md`, `docs/DECISION-ATLAS.md` and `.repository.toml` before implementation. The Atlas is append-only owner intent, not acceptance. A checked entry requires independently observed behavior.

## Ownership

- This repository owns generic search contracts and index/query mechanics, **not** source ingestion, user permissions, or application interfaces.
- Do not duplicate `moenarch-foundation` corpus identity/provenance, `rust-kernels` generic algorithms, `nlp-stack` linguistic behavior or `media-intelligence` cross-modal evidence alignment.
- Preserve existing consumer functionality; migrations need separately scoped issues and parity evidence. Avoid mandatory global database, hosted service or NLP dependencies.

## Conventions

This repo was empty, with **no pinned/installed conventions**. `conventions.json` selects `rust-performance` (inheriting Rust, benchmarking and base). Before serious implementation, resolve the current source described at https://moritzbrantner.github.io/coding-agent-conventions/agent-tool.json and install via `coding-tooling conventions update`. Commit the actual generated `conventions.lock.json` and `.conventions/`; never fabricate them or assume cache integrity means source freshness. Repository-local exceptions belong here.

## Delivery

- Work on one bounded vertical slice with observable real CLI/library/consumer behavior; do not introduce orchestrators, adapter packs or microservices before needed.
- `ReferenceIndex` is an O(n) **correctness oracle**, not evidence of an efficient production search implementation. Compare future indexed backends against it on independent fixtures and work/latency budgets.
- Rust checks: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --locked`, and `cargo run --locked --example metadata_search`. Report checks that could not run; verify CI on the exact PR head.
- A release, merge or cross-repository migration requires its own explicit authorization. Do not overwrite original data or turn derived index records into authoritative metadata.
