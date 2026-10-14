use search_engine::{
    Document, FieldType, Predicate, ReferenceIndex, Schema, SearchError, SearchRequest, Value,
};

fn schema() -> Schema {
    Schema::new()
        .field("kind", FieldType::Keyword)
        .field("name", FieldType::Text)
        .field("width", FieldType::Integer)
        .field("height", FieldType::Integer)
        .field("created_at", FieldType::TimestampMillis)
        .field("captured_at", FieldType::TimestampMillis)
        .field("hidden", FieldType::Boolean)
}

fn fixture() -> ReferenceIndex {
    let mut index = ReferenceIndex::new(schema());
    index
        .upsert(
            Document::new("old-image", 1)
                .field("kind", Value::Keyword("image".into()))
                .field("name", Value::Text("Cat photograph".into()))
                .field("width", Value::Integer(1920))
                .field("height", Value::Integer(1080))
                .field("created_at", Value::TimestampMillis(1_750_000_000_000))
                .field("captured_at", Value::TimestampMillis(1_600_000_000_000)),
        )
        .expect("valid test document");
    index
        .upsert(
            Document::new("recent-image", 1)
                .field("kind", Value::Keyword("image".into()))
                .field("name", Value::Text("Landscape photo".into()))
                .field("width", Value::Integer(3840))
                .field("height", Value::Integer(2160))
                .field("captured_at", Value::TimestampMillis(1_735_689_600_000)),
        )
        .expect("valid test document");
    index
        .upsert(
            Document::new("text", 1)
                .field("kind", Value::Keyword("document".into()))
                .field("name", Value::Text("Rust manual".into()))
                .field("hidden", Value::Boolean(true)),
        )
        .expect("valid test document");
    index
}

fn min_value(field: &str, value: Value) -> Predicate {
    Predicate::Range {
        field: field.into(),
        minimum: Some(value),
        maximum: None,
    }
}

#[test]
fn combines_typed_filters_without_fabricating_capture_times() {
    let index = fixture();
    let request = SearchRequest::new(Predicate::And(vec![
        Predicate::Equals("kind".into(), Value::Keyword("image".into())),
        min_value("width", Value::Integer(1920)),
        min_value("height", Value::Integer(1080)),
        min_value("captured_at", Value::TimestampMillis(1_704_067_200_000)),
    ]));
    let result = index.search(&request).expect("valid query");
    assert_eq!(result.total, 1);
    assert_eq!(result.hits[0].id, "recent-image");
    // File creation after the cutoff does not imply camera capture after it.
    let old = index
        .search(&SearchRequest::new(Predicate::And(vec![
            Predicate::Equals("kind".into(), Value::Keyword("image".into())),
            min_value("created_at", Value::TimestampMillis(1_704_067_200_000)),
        ])))
        .expect("valid creation-time query");
    assert_eq!(old.total, 1);
    assert_eq!(old.hits[0].id, "old-image");
}

#[test]
fn sorts_stably_and_keeps_absent_values_last() {
    let index = fixture();
    let request = SearchRequest::new(Predicate::All)
        .sort_by("captured_at", true)
        .page(0, 2);
    let result = index.search(&request).expect("valid query");
    assert_eq!(result.total, 3);
    assert_eq!(
        result
            .hits
            .iter()
            .map(|hit| hit.id.as_str())
            .collect::<Vec<_>>(),
        vec!["recent-image", "old-image"],
    );
    let tail = index
        .search(
            &SearchRequest::new(Predicate::All)
                .sort_by("captured_at", true)
                .page(2, 3),
        )
        .expect("valid query");
    assert_eq!(tail.hits[0].id, "text");
}

#[test]
fn supports_boolean_text_and_missing_value_predicates() {
    let index = fixture();
    let query = Predicate::And(vec![
        Predicate::Or(vec![
            Predicate::TextContains("name".into(), "PHOTOGRAPH".into()),
            Predicate::TextContains("name".into(), "landscape".into()),
        ]),
        Predicate::Not(Box::new(Predicate::Exists("hidden".into()))),
    ]);
    assert_eq!(index.search(&SearchRequest::new(query)).unwrap().total, 2);
    assert_eq!(
        index
            .search(&SearchRequest::new(Predicate::Equals(
                "hidden".into(),
                Value::Boolean(false)
            )))
            .unwrap()
            .total,
        0,
        "missing Boolean values must not equal false",
    );
}

#[test]
fn rejects_incompatible_queries_and_invalid_ranges() {
    let index = fixture();
    assert!(matches!(
        index.search(&SearchRequest::new(min_value(
            "width",
            Value::Text("1920".into())
        ))),
        Err(SearchError::WrongType { .. })
    ));
    assert!(matches!(
        index.search(&SearchRequest::new(Predicate::TextContains(
            "width".into(),
            "a".into()
        ))),
        Err(SearchError::UnsupportedTextField(_))
    ));
    assert!(matches!(
        index.search(&SearchRequest::new(Predicate::Range {
            field: "width".into(),
            minimum: Some(Value::Integer(10)),
            maximum: Some(Value::Integer(2)),
        })),
        Err(SearchError::InvalidRange(_))
    ));
    assert!(matches!(
        index.search(&SearchRequest::new(Predicate::Exists(
            "unregistered".into()
        ))),
        Err(SearchError::UnknownField(_))
    ));
}

#[test]
fn update_and_delete_are_deterministic() {
    let mut index = ReferenceIndex::new(schema());
    let doc = Document::new("same-id", 3).field("width", Value::Integer(2560));
    assert!(index.upsert(doc.clone()).unwrap());
    assert!(
        !index.upsert(doc).unwrap(),
        "identical replay must not duplicate"
    );
    assert!(matches!(
        index.upsert(Document::new("same-id", 2)),
        Err(SearchError::StaleRevision(_))
    ));
    assert!(matches!(
        index.upsert(Document::new("same-id", 3).field("width", Value::Integer(1920))),
        Err(SearchError::ConflictingRevision(_))
    ));
    assert!(
        index
            .upsert(Document::new("same-id", 4).field("width", Value::Integer(1920)))
            .unwrap()
    );
    assert_eq!(index.len(), 1);
    assert_eq!(
        index
            .search(&SearchRequest::new(min_value(
                "width",
                Value::Integer(2000)
            )))
            .unwrap()
            .total,
        0,
        "old indexed values must not remain",
    );
    assert!(index.delete("same-id"));
    assert!(!index.delete("same-id"));
    assert!(index.is_empty());
}

#[test]
fn rejected_documents_do_not_change_the_index() {
    let mut index = fixture();
    let before = index.len();
    assert!(matches!(
        index.upsert(Document::new("bad", 1).field("width", Value::Text("wrong".into()))),
        Err(SearchError::WrongType { .. })
    ));
    assert!(matches!(
        index.upsert(Document::new(" ", 1)),
        Err(SearchError::EmptyDocumentId)
    ));
    assert_eq!(index.len(), before);
}
