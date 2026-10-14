//! A multi-process local acceptance surface. Use an explicit index directory.
//! Ingest, query, mutate, query again in a new process, then rebuild.

use std::env;
use std::error::Error;
use std::path::Path;

use search_engine::indexed::{Mutation, TantivyIndex};
use search_engine::{Document, FieldType, Predicate, Schema, SearchRequest, Value};

fn schema() -> Schema {
    Schema::new()
        .field("kind", FieldType::Keyword)
        .field("filename", FieldType::Text)
        .field("path", FieldType::Keyword)
        .field("mime", FieldType::Keyword)
        .field("extension", FieldType::Keyword)
        .field("size", FieldType::Integer)
        .field("width", FieldType::Integer)
        .field("height", FieldType::Integer)
        .field("created_at", FieldType::TimestampMillis)
        .field("modified_at", FieldType::TimestampMillis)
        .field("captured_at", FieldType::TimestampMillis)
        .field("content", FieldType::Text)
        .field("source_uri", FieldType::Keyword)
}

fn fixture() -> Result<Vec<Document>, serde_json::Error> {
    serde_json::from_str(include_str!("../fixtures/mixed-files.json"))
}

fn changed(document: &Document) -> Document {
    if document.id == "alpine-4k" {
        let mut updated = document.clone();
        updated.revision = 2;
        updated.fields.insert("width".into(), Value::Integer(2560));
        updated
    } else {
        document.clone()
    }
}

fn query(needle: &str) -> SearchRequest {
    let mut filters = vec![
        Predicate::Equals("kind".into(), Value::Keyword("image".into())),
        Predicate::Range {
            field: "width".into(),
            minimum: Some(Value::Integer(1920)),
            maximum: None,
        },
        Predicate::Range {
            field: "height".into(),
            minimum: Some(Value::Integer(1080)),
            maximum: None,
        },
        Predicate::Range {
            field: "captured_at".into(),
            minimum: Some(Value::TimestampMillis(1_704_067_200_000)),
            maximum: None,
        },
    ];
    if !needle.is_empty() {
        filters.push(Predicate::TextContains("filename".into(), needle.into()));
    }
    SearchRequest::new(Predicate::And(filters)).sort_by("captured_at", true)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        return Err("usage: cargo run --locked --example indexed_search -- <index-directory> <ingest|query|mutate|rebuild> [text]".into());
    }

    let index_path = Path::new(&args[1]);
    let command = args[2].as_str();
    let documents = fixture()?;
    let mut index = TantivyIndex::open(index_path, schema())?;
    match command {
        "ingest" => {
            let operations: Vec<_> = documents
                .into_iter()
                .map(|document| Mutation::Upsert { document })
                .collect();
            let changed = index.apply_batch(&operations)?;
            println!(
                "{}",
                serde_json::json!({"status":"ingested","changed":changed,"live":index.len()})
            );
        }
        "query" => {
            let text = args.get(3).map_or("", String::as_str);
            let answer = index.search_with_facets(&query(text), &["kind", "extension"])?;
            let records: Vec<_> = answer
                .results
                .hits
                .iter()
                .filter_map(|hit| index.document(&hit.id))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "query": {"kind":"image","min_width":1920,"min_height":1080,"captured_since_utc":"2024-01-01","filename_contains":text},
                    "matches":answer.results,
                    "records":records,
                    "facets":answer.facets,
                    "work":answer.work
                }))?
            );
        }
        "mutate" => {
            let operations = vec![
                Mutation::Upsert {
                    document: changed(&documents[0]),
                },
                Mutation::Delete {
                    id: "old-4k".into(),
                    revision: 2,
                },
            ];
            let changed = index.apply_batch(&operations)?;
            println!(
                "{}",
                serde_json::json!({"status":"mutated","changed":changed,"live":index.len()})
            );
        }
        "rebuild" => {
            // The caller supplies a full live snapshot plus explicit deletion.
            let mut operations: Vec<_> = documents
                .into_iter()
                .filter(|doc| doc.id != "old-4k")
                .map(|doc| Mutation::Upsert {
                    document: changed(&doc),
                })
                .collect();
            operations.push(Mutation::Delete {
                id: "old-4k".into(),
                revision: 2,
            });
            index.rebuild(&operations)?;
            println!(
                "{}",
                serde_json::json!({"status":"rebuilt","live":index.len()})
            );
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }
    Ok(())
}
