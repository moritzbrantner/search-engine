//! Informational cold-index / warm-query measurements; no CI latency gate.
//! Run: cargo run --release --locked --example bench_indexed
use std::hint::black_box;
use std::time::Instant;

use search_engine::indexed::{Mutation, TantivyIndex};
use search_engine::{Document, FieldType, Predicate, ReferenceIndex, Schema, SearchRequest, Value};
use tempfile::tempdir;

fn schema() -> Schema {
    Schema::new()
        .field("kind", FieldType::Keyword)
        .field("path", FieldType::Keyword)
        .field("filename", FieldType::Text)
        .field("width", FieldType::Integer)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for size in [100usize, 1_000, 5_000] {
        let documents: Vec<_> = (0..size)
            .map(|id| {
                Document::new(format!("file-{id:06}"), 1)
                    .field("kind", Value::Keyword("image".into()))
                    .field("path", Value::Keyword(format!("photos/asset-{id:06}.jpg")))
                    .field(
                        "filename",
                        Value::Text(format!("Alpine landscape asset {id:06}.jpg")),
                    )
                    .field("width", Value::Integer(1000 + id as i64))
            })
            .collect();
        let request = SearchRequest::new(Predicate::And(vec![
            Predicate::Equals(
                "path".into(),
                Value::Keyword(format!("photos/asset-{:06}.jpg", size / 2)),
            ),
            Predicate::Range {
                field: "width".into(),
                minimum: Some(Value::Integer(1000 + size as i64 / 2)),
                maximum: None,
            },
        ]));

        let temp = tempdir()?;
        let mut index = TantivyIndex::open(temp.path(), schema())?;
        let cold_start = Instant::now();
        let mutations: Vec<_> = documents
            .iter()
            .cloned()
            .map(|document| Mutation::Upsert { document })
            .collect();
        index.apply_batch(&mutations)?;
        let index_ns = cold_start.elapsed().as_nanos();

        let mut oracle = ReferenceIndex::new(schema());
        for document in documents {
            oracle.upsert(document)?;
        }
        let oracle_start = Instant::now();
        let oracle_result = black_box(oracle.search(&request)?);
        let oracle_query_ns = oracle_start.elapsed().as_nanos();
        let indexed_start = Instant::now();
        let indexed_result = black_box(index.search_with_facets(&request, &[])?);
        let indexed_query_ns = indexed_start.elapsed().as_nanos();
        assert_eq!(indexed_result.results, oracle_result);
        println!(
            "{}",
            serde_json::json!({
                "workload":"selective-file-path-and-width",
                "records":size, "index_ns":index_ns.to_string(),
                "reference_query_ns":oracle_query_ns.to_string(),
                "indexed_query_ns":indexed_query_ns.to_string(),
                "candidate_documents":indexed_result.work.candidate_documents,
                "target_os":std::env::consts::OS,
                "target_arch":std::env::consts::ARCH,
                "build_profile":if cfg!(debug_assertions) { "dev" } else { "release" },
                "note":"single-run wall clock; informational, not a portable latency guarantee"
            })
        );
    }
    Ok(())
}
