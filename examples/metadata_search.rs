use search_engine::{Document, FieldType, Predicate, ReferenceIndex, Schema, SearchRequest, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = Schema::new()
        .field("kind", FieldType::Keyword)
        .field("name", FieldType::Text)
        .field("width", FieldType::Integer)
        .field("height", FieldType::Integer)
        .field("captured_at", FieldType::TimestampMillis);

    let mut index = ReferenceIndex::new(schema);
    index.upsert(
        Document::new("photo-4k", 1)
            .field("kind", Value::Keyword("image".into()))
            .field("name", Value::Text("Mountain landscape".into()))
            .field("width", Value::Integer(3840))
            .field("height", Value::Integer(2160))
            .field("captured_at", Value::TimestampMillis(1_735_689_600_000)),
    )?;
    index.upsert(
        Document::new("old-photo", 1)
            .field("kind", Value::Keyword("image".into()))
            .field("name", Value::Text("Mountain landscape".into()))
            .field("width", Value::Integer(4096))
            .field("height", Value::Integer(2160))
            .field("captured_at", Value::TimestampMillis(1_600_000_000_000)),
    )?;
    index.upsert(
        Document::new("notes", 1)
            .field("kind", Value::Keyword("document".into()))
            .field("name", Value::Text("Mountain notes".into())),
    )?;

    let query = SearchRequest::new(Predicate::And(vec![
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
        Predicate::TextContains("name".into(), "mountain".into()),
    ]))
    .page(0, 10);

    let results = index.search(&query)?;
    println!("{} matching file(s)", results.total);
    for hit in results.hits {
        println!("{}", hit.id);
    }
    Ok(())
}
