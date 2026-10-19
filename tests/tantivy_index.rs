use search_engine::indexed::{IndexedError, Mutation, TantivyIndex};
use search_engine::{
    Document, FieldType, Predicate, ReferenceIndex, Schema, SearchError, SearchRequest, Value,
};
use tempfile::tempdir;

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
        .field("hidden", FieldType::Boolean)
}

fn documents() -> Vec<Document> {
    serde_json::from_str(include_str!("../fixtures/mixed-files.json")).expect("committed fixture")
}

fn reference(docs: &[Document]) -> ReferenceIndex {
    let mut index = ReferenceIndex::new(schema());
    for doc in docs {
        index.upsert(doc.clone()).expect("fixture schema");
    }
    index
}

fn request(predicate: Predicate) -> SearchRequest {
    SearchRequest::new(predicate)
}

fn min(field: &str, value: Value) -> Predicate {
    Predicate::Range {
        field: field.into(),
        minimum: Some(value),
        maximum: None,
    }
}

fn assert_same(reference: &ReferenceIndex, indexed: &TantivyIndex, queries: &[SearchRequest]) {
    for query in queries {
        assert_eq!(
            indexed.search(query).expect("indexed query"),
            reference.search(query).expect("reference query"),
            "query: {query:?}"
        );
    }
}

fn queries() -> Vec<SearchRequest> {
    vec![
        request(Predicate::All),
        request(Predicate::Equals(
            "kind".into(),
            Value::Keyword("image".into()),
        )),
        request(Predicate::Equals("width".into(), Value::Integer(3840))),
        request(Predicate::Equals("hidden".into(), Value::Boolean(false))),
        request(Predicate::Exists("hidden".into())),
        request(Predicate::Not(Box::new(Predicate::Exists(
            "captured_at".into(),
        )))),
        request(Predicate::TextContains("filename".into(), "SUNRISE".into())),
        request(Predicate::TextContains("filename".into(), "st se".into())),
        request(Predicate::TextContains("content".into(), "indexed".into())),
        request(Predicate::TextContains("filename".into(), "".into())),
        request(Predicate::TextContains("path".into(), "photos/".into())),
        request(Predicate::Or(vec![
            Predicate::Equals("kind".into(), Value::Keyword("document".into())),
            Predicate::Equals("kind".into(), Value::Keyword("video".into())),
        ])),
        request(Predicate::And(vec![
            Predicate::Equals("kind".into(), Value::Keyword("image".into())),
            min("width", Value::Integer(1920)),
            min("height", Value::Integer(1080)),
            min("captured_at", Value::TimestampMillis(1_704_067_200_000)),
        ])),
        request(Predicate::Range {
            field: "width".into(),
            minimum: None,
            maximum: None,
        }),
        request(Predicate::Range {
            field: "width".into(),
            minimum: None,
            maximum: Some(Value::Integer(1280)),
        }),
        request(Predicate::Not(Box::new(Predicate::Equals(
            "kind".into(),
            Value::Keyword("video".into()),
        )))),
        request(Predicate::And(vec![])),
        request(Predicate::Or(vec![])),
        request(Predicate::Not(Box::new(Predicate::Or(vec![])))),
        request(Predicate::Or(vec![
            Predicate::Not(Box::new(Predicate::Exists("width".into()))),
            Predicate::TextContains("filename".into(), "old".into()),
        ])),
        request(Predicate::All)
            .sort_by("captured_at", true)
            .page(0, 3),
        request(Predicate::All).sort_by("size", false).page(2, 2),
    ]
}

#[test]
fn typed_queries_and_facets_match_independent_oracle() {
    let temp = tempdir().unwrap();
    let docs = documents();
    let reference = reference(&docs);
    let mut index = TantivyIndex::open(temp.path(), schema()).unwrap();
    assert_eq!(
        index
            .apply_batch(
                &docs
                    .iter()
                    .cloned()
                    .map(|document| Mutation::Upsert { document })
                    .collect::<Vec<_>>()
            )
            .unwrap(),
        docs.len()
    );
    assert_same(&reference, &index, &queries());
    let media = request(Predicate::Equals(
        "kind".into(),
        Value::Keyword("image".into()),
    ));
    let answer = index
        .search_with_facets(&media, &["kind", "extension", "captured_at"])
        .unwrap();
    assert_eq!(answer.results.total, 3);
    assert_eq!(answer.facets["kind"]["image"], 3);
    assert_eq!(answer.facets["extension"]["jpg"], 2);
    assert_eq!(answer.facets["extension"]["png"], 1);
    assert_eq!(
        answer.facets["captured_at"].values().sum::<usize>(),
        2,
        "missing capture date does not become creation or modification date"
    );
    assert!(
        answer.work.candidate_documents < 5,
        "indexable filters must avoid visiting unrelated file records"
    );
    drop(index);
    let reopened = TantivyIndex::open(temp.path(), schema()).unwrap();
    assert_same(&reference, &reopened, &queries());
}

#[test]
fn restart_delete_reindex_and_rebuild_preserve_revisions() {
    let temp = tempdir().unwrap();
    let docs = documents();
    let mut oracle = reference(&docs);
    {
        let mut index = TantivyIndex::open(temp.path(), schema()).unwrap();
        index
            .apply_batch(
                &docs
                    .iter()
                    .cloned()
                    .map(|document| Mutation::Upsert { document })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let mut changed = docs[0].clone();
        changed.revision = 3;
        changed.fields.insert("width".into(), Value::Integer(1280));
        assert!(oracle.upsert(changed.clone()).unwrap());
        assert!(index.upsert(changed.clone()).unwrap());
        assert!(!index.upsert(changed.clone()).unwrap());
        assert!(matches!(
            index.upsert(docs[0].clone()),
            Err(IndexedError::Contract(SearchError::StaleRevision(_)))
        ));
        assert!(matches!(
            index.upsert(Document::new("alpine-4k", 3)),
            Err(IndexedError::Contract(SearchError::ConflictingRevision(_)))
        ));
        assert!(oracle.delete("old-4k", 4).unwrap());
        assert!(index.delete("old-4k", 4).unwrap());
        assert!(!index.delete("old-4k", 4).unwrap());
        assert_same(&oracle, &index, &queries());
    }
    {
        let mut reopened = TantivyIndex::open(temp.path(), schema()).unwrap();
        assert_same(&oracle, &reopened, &queries());
        assert!(matches!(
            reopened.upsert(Document::new("old-4k", 4)),
            Err(IndexedError::Contract(SearchError::StaleRevision(_)))
        ));
        assert!(matches!(
            reopened.delete("old-4k", 3),
            Err(IndexedError::Contract(SearchError::StaleRevision(_)))
        ));
        let active: Vec<_> = docs
            .iter()
            .filter(|doc| doc.id != "old-4k")
            .map(|doc| {
                if doc.id == "alpine-4k" {
                    Mutation::Upsert {
                        document: reopened.document("alpine-4k").unwrap().clone(),
                    }
                } else {
                    Mutation::Upsert {
                        document: doc.clone(),
                    }
                }
            })
            .collect();
        assert!(
            matches!(reopened.rebuild(&active), Ok(())),
            "previous tombstones may be omitted from full live snapshot"
        );
        assert_same(&oracle, &reopened, &queries());
    }
    let mut reopened = TantivyIndex::open(temp.path(), schema()).unwrap();
    assert!(matches!(
        reopened.upsert(Document::new("old-4k", 3)),
        Err(IndexedError::Contract(SearchError::StaleRevision(_)))
    ));
    let mut new_doc = docs[2].clone();
    new_doc.revision = 5;
    assert!(oracle.upsert(new_doc.clone()).unwrap());
    assert!(reopened.upsert(new_doc).unwrap());
    assert_same(&oracle, &reopened, &queries());
}

#[test]
fn incomplete_rebuild_and_failed_batches_do_not_mutate_visible_state() {
    let temp = tempdir().unwrap();
    let docs = documents();
    let mut index = TantivyIndex::open(temp.path(), schema()).unwrap();
    index
        .apply_batch(
            &docs
                .iter()
                .cloned()
                .map(|document| Mutation::Upsert { document })
                .collect::<Vec<_>>(),
        )
        .unwrap();

    assert!(matches!(
        index.rebuild(&[Mutation::Upsert {
            document: docs[0].clone()
        }]),
        Err(IndexedError::IncompleteRebuild(_))
    ));
    let bad_batch = vec![
        Mutation::Delete {
            id: "alpine-4k".into(),
            revision: 100,
        },
        Mutation::Upsert {
            document: Document::new("invalid", 1).field("width", Value::Text("no".into())),
        },
    ];
    assert!(matches!(
        index.apply_batch(&bad_batch),
        Err(IndexedError::Contract(SearchError::WrongType { .. }))
    ));
    assert_same(&reference(&docs), &index, &queries());
    let mismatched = Schema::new().field("kind", FieldType::Keyword);
    drop(index);
    assert!(matches!(
        TantivyIndex::open(temp.path(), mismatched),
        Err(IndexedError::SchemaMismatch)
    ));
}

#[test]
fn indexed_work_stays_below_full_scan_for_selective_queries() {
    let temp = tempdir().unwrap();
    let mut index = TantivyIndex::open(temp.path(), schema()).unwrap();
    let docs: Vec<_> = (0..600)
        .map(|i| {
            Document::new(format!("asset-{i:04}"), 1)
                .field("path", Value::Keyword(format!("assets/file-{i:04}.jpg")))
                .field("kind", Value::Keyword("image".into()))
                .field("width", Value::Integer(1000 + i))
        })
        .collect();
    index
        .apply_batch(
            &docs
                .iter()
                .cloned()
                .map(|document| Mutation::Upsert { document })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let query = request(Predicate::And(vec![
        Predicate::Equals("path".into(), Value::Keyword("assets/file-0042.jpg".into())),
        min("width", Value::Integer(1042)),
    ]));
    assert_eq!(
        index.search(&query).unwrap(),
        reference(&docs).search(&query).unwrap()
    );
    let result = index.search_with_facets(&query, &["kind"]).unwrap();
    assert_eq!(result.results.total, 1);
    assert!(
        result.work.candidate_documents < 10,
        "selective inverted postings should not visit all 600 documents"
    );
}

#[test]
fn invalid_query_and_page_bounds_report_errors() {
    let temp = tempdir().unwrap();
    let index = TantivyIndex::open(temp.path(), schema()).unwrap();
    let bad = request(Predicate::Range {
        field: "width".into(),
        minimum: Some(Value::Integer(2000)),
        maximum: Some(Value::Integer(1000)),
    });
    assert!(matches!(
        index.search(&bad),
        Err(IndexedError::Contract(SearchError::InvalidRange(_)))
    ));
    assert!(matches!(
        index.search(&request(Predicate::TextContains(
            "width".into(),
            "hi".into()
        ))),
        Err(IndexedError::Contract(SearchError::UnsupportedTextField(_)))
    ));
    assert!(matches!(
        index.search(&request(Predicate::All).page(0, 1001)),
        Err(IndexedError::InvalidPage)
    ));
}

#[test]
fn negated_text_queries_do_not_drop_ngram_false_positives() {
    let temp = tempdir().unwrap();
    // Both texts have the "abc" and "bcd" trigrams, but only one
    // contains the uninterrupted "abcd" substring.
    let docs = vec![
        Document::new("false-positive", 1).field("filename", Value::Text("abcXbcd".into())),
        Document::new("matching", 1).field("filename", Value::Text("abcd".into())),
        Document::new("unrelated", 1)
            .field("kind", Value::Keyword("document".into()))
            .field("filename", Value::Text("other".into())),
        Document::new("missing", 1),
    ];
    let mut index = TantivyIndex::open(temp.path(), schema()).unwrap();
    let reference = reference(&docs);
    index
        .apply_batch(
            &docs
                .iter()
                .cloned()
                .map(|document| Mutation::Upsert { document })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let contains = Predicate::TextContains("filename".into(), "abcd".into());
    let exact = Predicate::Equals("filename".into(), Value::Text("abcd".into()));
    assert_same(
        &reference,
        &index,
        &[
            request(Predicate::Not(Box::new(contains.clone()))),
            request(Predicate::Not(Box::new(exact.clone()))),
            request(Predicate::Not(Box::new(Predicate::And(vec![
                Predicate::Equals("kind".into(), Value::Keyword("document".into())),
                contains.clone(),
            ])))),
            request(Predicate::Or(vec![
                Predicate::Not(Box::new(contains.clone())),
                Predicate::Equals("kind".into(), Value::Keyword("document".into())),
            ])),
            request(Predicate::Not(Box::new(Predicate::Not(Box::new(contains))))),
        ],
    );
}

#[test]
fn latest_revision_includes_committed_deletions_across_restart_and_rebuild() {
    let directory = tempdir().unwrap();
    let mut index = TantivyIndex::open(directory.path(), schema()).unwrap();
    assert_eq!(index.latest_revision("source"), None);
    index.upsert(Document::new("source", 7)).unwrap();
    assert_eq!(index.latest_revision("source"), Some(7));
    assert!(index.upsert(Document::new("source", 6)).is_err());
    assert_eq!(index.latest_revision("source"), Some(7));

    index.delete("source", 8).unwrap();
    assert_eq!(index.latest_revision("source"), Some(8));
    assert!(index.document("source").is_none());
    drop(index);

    let mut index = TantivyIndex::open(directory.path(), schema()).unwrap();
    assert_eq!(index.latest_revision("source"), Some(8));
    assert!(index.upsert(Document::new("source", 8)).is_err());
    index.rebuild(&[]).unwrap();
    assert_eq!(index.latest_revision("source"), Some(8));
    index.upsert(Document::new("source", 9)).unwrap();
    assert_eq!(index.latest_revision("source"), Some(9));

    assert!(
        index
            .upsert(Document::new("source", 9).field("kind", Value::Keyword("image".into())))
            .is_err()
    );
    assert!(
        index
            .apply_batch(&[
                Mutation::Upsert {
                    document: Document::new("source", 10),
                },
                Mutation::Upsert {
                    document: Document::new("invalid", 1).field("undeclared", Value::Boolean(true)),
                },
            ])
            .is_err()
    );
    assert_eq!(index.latest_revision("source"), Some(9));
    assert_eq!(index.latest_revision("invalid"), None);
}
