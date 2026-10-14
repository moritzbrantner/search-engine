# Search Engine Decision Atlas

This file records approved product intent, **not implementation**. Engineering details in issues and PRs do not become owner decisions automatically.

## Editing

- Decision and Question are the only leaf-entry kinds. Feature headings group them.
- IDs are stable: append rather than renumber/delete. Resolve a Question by appending a Decision marked `Answers <ID>`; change an older Decision with a new one marked `Supersedes <ID>`.
- `[x]` denotes independently verified and observable implementation, **not** agreement, scaffolding or a passing isolated unit test.
- Keep entries brief. Implementation belongs in issues and code, acceptance in executable tests.

## 1. Ownership

- **1.1 [ ] Decision — Independent Rust search capability.**  
  Status: adopted 2026-10-14.  
  Generic search lives in its own repository. Typed file metadata search must work without NLP, model runtimes or a search server.

- **1.2 [ ] Decision — Consumers retain source authority.**  
  Status: adopted 2026-10-14.  
  Applications retain canonical data, metadata, permissions, ingestion and search UI. Reusable indexes are projections, not compulsory shared source databases. No global index is mandated.

- **1.3 [ ] Decision — Respect existing reusable capabilities.**  
  Status: adopted 2026-10-14.  
  The engine reuses neutral corpus/provenance contracts and lower-level algorithms, without reimplementing NLP, OCR, embeddings or multimodal analysis. Existing search paths survive migration until parity is proven.

## 2. Query capabilities

- **2.1 [ ] Decision — Structured metadata is first-class searchable data.**  
  Status: adopted 2026-10-14.  
  Query filenames, paths, types, sizes, timestamps and image pixel dimensions, along with other available typed metadata. Support combinations of exact, numeric/temporal range, Boolean and text conditions, with stable sorting.

- **2.2 [ ] Decision — Text retrieval is optional.**  
  Status: adopted 2026-10-14.  
  A metadata-only corpus is useful by itself. Source text, OCR/transcripts and optional semantic results can extend search without making linguistic processing foundational.

- **2.3 [ ] Decision — Preserve meaning of fields and missing values.**  
  Status: adopted 2026-10-14 as a correctness requirement for 2.1.  
  Filesystem creation time, filesystem modification time and EXIF capture time have distinct identities. Unsupported or unavailable fields remain absent, not fabricated. Hits retain stable source references.

## 3. Index lifecycle

- **3.1 [ ] Decision — Deterministic incremental and rebuildable projections.**  
  Status: adopted 2026-10-14.  
  Repeated indexing of unchanged sources does not duplicate hits. Updates/deletions replace stale indexed data. Rebuilding from authoritative records must be possible, and partial/failed indexing must be explicit.

- **3.2 [ ] Decision — Embeddable library first.**  
  Status: adopted 2026-10-14.  
  The Rust library can be consumed without a mandatory service, deployment or shared storage. CLI/headless acceptance is valid for the capability; optional WASM/server/UI adapters need actual consumers.

## 4. Open questions

- **4.1 [ ] Question — Is cross-application federated search a separate product?**  
  Status: open since 2026-10-14, not a blocker.  
  A common UI over independent sources may be valuable, but federation, cross-application permissions and a global index have not been approved.

- **4.2 [ ] Question — Should the human query DSL be standardized?**  
  Status: open since 2026-10-14, not a blocker.  
  Start with a typed Rust query API. Decide on a shared textual/URL grammar only after distinct consumers demonstrate compatible needs.
