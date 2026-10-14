//! Durable, reconstructible Tantivy projection of caller-owned typed records.
//!
//! No filesystem crawling or source data ownership lives here. A single writer
//! owns the directory; committed lifecycle records (including tombstones) survive
//! process restarts. Search evaluates postings before reading candidate records.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Bound;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tantivy::collector::DocSetCollector;
use tantivy::query::{AllQuery, BooleanQuery, EmptyQuery, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{
    INDEXED, IndexRecordOption, STORED, STRING, Schema as TantivySchema, TantivyDocument,
    TextFieldIndexing, TextOptions, Value as TantivyValue,
};
use tantivy::tokenizer::{NgramTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, Term};

use crate::{
    Document, FieldType, Predicate, ReferenceIndex, Schema, SearchError, SearchHit, SearchRequest,
    SearchResults, Value, compare_optional, matches_predicate,
};

const PREFIX: &str = "_se_";
const ID: &str = "_se_id";
const STATE: &str = "_se_state";
const PAYLOAD: &str = "_se_payload";
const PRESENCE: &str = "_se_present";
const VERSION: &str = "_se_layout_v1";
const TOKENIZER: &str = "se_ngram_1_3";
const MAX_LIMIT: usize = 1_000;
const MAX_OFFSET: usize = 1_000_000;
const WRITER_MEMORY: usize = 50_000_000;

#[derive(Debug, thiserror::Error)]
pub enum IndexedError {
    #[error(transparent)]
    Contract(#[from] SearchError),
    #[error(transparent)]
    Engine(#[from] tantivy::TantivyError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("index schema or layout differs; rebuild in a new directory")]
    SchemaMismatch,
    #[error("index directory contains files but no Tantivy metadata")]
    UninitializedDirectory,
    #[error("reserved or invalid schema field: {0}")]
    InvalidField(String),
    #[error("invalid or duplicate stored lifecycle record: {0}")]
    CorruptIndex(String),
    #[error("rebuild omits live source ID; provide its record or explicit deletion: {0}")]
    IncompleteRebuild(String),
    #[error("offset or limit exceeds the supported request bound")]
    InvalidPage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Mutation {
    Upsert { document: Document },
    Delete { id: String, revision: u64 },
}

impl Mutation {
    fn id(&self) -> &str {
        match self {
            Self::Upsert { document } => &document.id,
            Self::Delete { id, .. } => id,
        }
    }

    fn revision(&self) -> u64 {
        match self {
            Self::Upsert { document } => document.revision,
            Self::Delete { revision, .. } => *revision,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct SearchWork {
    /// Number of indexed posting-list candidates retrieved and checked.
    /// Does not count Tantivy's internal term enumeration or segment work.
    pub candidate_documents: usize,
    pub exact_evaluations: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FacetedResults {
    pub results: SearchResults,
    pub facets: BTreeMap<String, BTreeMap<String, usize>>,
    pub work: SearchWork,
}

/// A versioned, in-process Tantivy index. Callers own the canonical records.
///
/// Writes are committed once per operation/batch. Readers see only committed
/// records, and an opened writer prevents another process from writing to the
/// same directory concurrently. Reopen after any ambiguous I/O failure.
pub struct TantivyIndex {
    schema: Schema,
    index: Index,
    reader: IndexReader,
    writer: IndexWriter,
    state: BTreeMap<String, Mutation>,
}

impl TantivyIndex {
    pub fn open(path: impl AsRef<Path>, schema: Schema) -> Result<Self, IndexedError> {
        for name in schema.fields.keys() {
            if name.is_empty() || name.starts_with(PREFIX) {
                return Err(IndexedError::InvalidField(name.clone()));
            }
        }

        let path = path.as_ref();
        fs::create_dir_all(path)?;
        let expected = build_schema(&schema);
        let index = if path.join("meta.json").is_file() {
            Index::open_in_dir(path)?
        } else {
            if fs::read_dir(path)?.next().is_some() {
                return Err(IndexedError::UninitializedDirectory);
            }
            Index::create_in_dir(path, expected.clone())?
        };
        if index.schema() != expected {
            return Err(IndexedError::SchemaMismatch);
        }
        register_tokenizer(&index)?;
        let reader = index.reader()?;
        reader.reload()?;
        let state = load_state(&reader, &index)?;
        let writer = index.writer(WRITER_MEMORY)?;

        Ok(Self {
            schema,
            index,
            reader,
            writer,
            state,
        })
    }

    pub fn len(&self) -> usize {
        self.state
            .values()
            .filter(|record| matches!(record, Mutation::Upsert { .. }))
            .count()
    }

    pub fn document(&self, id: &str) -> Option<&Document> {
        match self.state.get(id) {
            Some(Mutation::Upsert { document }) => Some(document),
            _ => None,
        }
    }

    pub fn upsert(&mut self, document: Document) -> Result<bool, IndexedError> {
        Ok(self.apply_batch(&[Mutation::Upsert { document }])? > 0)
    }

    pub fn delete(&mut self, id: &str, revision: u64) -> Result<bool, IndexedError> {
        Ok(self.apply_batch(&[Mutation::Delete {
            id: id.to_owned(),
            revision,
        }])? > 0)
    }

    /// Validate all changes before mutating the writer, and commit once.
    /// Identical source revisions are no-ops; equal-but-different revisions fail.
    pub fn apply_batch(&mut self, operations: &[Mutation]) -> Result<usize, IndexedError> {
        let changed = self.stage(operations)?;
        if changed.is_empty() {
            return Ok(0);
        }

        let names = self.index.schema();
        let id_field = names.get_field(ID)?;
        for (id, operation) in &changed {
            self.writer.delete_term(Term::from_field_text(id_field, id));
            self.writer.add_document(project(&names, operation)?)?;
        }
        self.writer.commit()?;
        self.reader.reload()?;
        let len = changed.len();
        self.state.extend(changed);
        Ok(len)
    }

    /// Full re-materialization from a caller-supplied authoritative snapshot.
    ///
    /// Every previously active ID must occur as an upsert or revisioned delete.
    /// Previously committed deletion tombstones are retained even when omitted,
    /// preventing delayed old writes from reviving removed sources.
    pub fn rebuild(&mut self, snapshot: &[Mutation]) -> Result<(), IndexedError> {
        let changes = self.stage(snapshot)?;
        let mentioned: BTreeSet<&str> = snapshot.iter().map(Mutation::id).collect();
        for (id, record) in &self.state {
            if matches!(record, Mutation::Upsert { .. }) && !mentioned.contains(id.as_str()) {
                return Err(IndexedError::IncompleteRebuild(id.clone()));
            }
        }
        let mut next = self.state.clone();
        next.extend(changes);

        self.writer.delete_all_documents()?;
        let names = self.index.schema();
        for operation in next.values() {
            self.writer.add_document(project(&names, operation)?)?;
        }
        self.writer.commit()?;
        self.reader.reload()?;
        self.state = next;
        Ok(())
    }

    fn stage(&self, operations: &[Mutation]) -> Result<BTreeMap<String, Mutation>, IndexedError> {
        let validator = ReferenceIndex::new(self.schema.clone());
        let mut changed = BTreeMap::new();
        for operation in operations {
            let id = operation.id();
            if id.trim().is_empty() {
                return Err(SearchError::EmptyDocumentId.into());
            }
            if let Mutation::Upsert { document } = operation {
                for (name, value) in &document.fields {
                    validator.validate_value(name, value)?;
                }
            }
            let previous = changed.get(id).or_else(|| self.state.get(id));
            if validate_revision(previous, operation)? {
                changed.insert(id.to_owned(), operation.clone());
            }
        }
        Ok(changed)
    }

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResults, IndexedError> {
        Ok(self.search_with_facets(request, &[])?.results)
    }

    pub fn search_with_facets(
        &self,
        request: &SearchRequest,
        facet_fields: &[&str],
    ) -> Result<FacetedResults, IndexedError> {
        let validator = ReferenceIndex::new(self.schema.clone());
        validator.validate_predicate(&request.predicate)?;
        if let Some(sort) = &request.sort {
            validator.known_field(&sort.field)?;
        }
        for field in facet_fields {
            validator.known_field(field)?;
        }
        if request.limit > MAX_LIMIT || request.offset > MAX_OFFSET {
            return Err(IndexedError::InvalidPage);
        }

        let names = self.index.schema();
        let live = term_query(names.get_field(STATE)?, "live");
        let predicate = self.query(&request.predicate)?;
        let query = BooleanQuery::new(vec![(Occur::Must, live), (Occur::Must, predicate)]);
        let searcher = self.reader.searcher();
        let candidates = searcher.search(&query, &DocSetCollector)?;
        let mut matching: Vec<Document> = Vec::new();
        let mut exact_evaluations = 0;

        for address in &candidates {
            let stored: TantivyDocument = searcher.doc(*address)?;
            let operation = decode(&names, &stored)?;
            if let Mutation::Upsert { document } = operation {
                exact_evaluations += 1;
                // Trigram posting lists can have false positives for long
                // substrings; this final check retains the reference contract.
                if matches_predicate(&document, &request.predicate) {
                    matching.push(document);
                }
            }
        }
        if let Some(sort) = &request.sort {
            matching.sort_by(|a, b| {
                compare_optional(
                    a.fields.get(&sort.field),
                    b.fields.get(&sort.field),
                    sort.descending,
                )
                .then_with(|| a.id.cmp(&b.id))
            });
        } else {
            matching.sort_by(|a, b| a.id.cmp(&b.id));
        }

        let mut facets: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
        for field in facet_fields {
            let mut counts = BTreeMap::new();
            for document in &matching {
                if let Some(value) = document.fields.get(*field) {
                    let key = facet_value(value);
                    *counts.entry(key).or_insert(0) += 1;
                }
            }
            facets.insert((*field).to_owned(), counts);
        }
        let total = matching.len();
        let hits = matching
            .into_iter()
            .skip(request.offset)
            .take(request.limit)
            .map(|d| SearchHit {
                id: d.id,
                revision: d.revision,
            })
            .collect();

        Ok(FacetedResults {
            results: SearchResults { total, hits },
            facets,
            work: SearchWork {
                candidate_documents: candidates.len(),
                exact_evaluations,
            },
        })
    }

    fn query(&self, predicate: &Predicate) -> Result<Box<dyn Query>, IndexedError> {
        let names = self.index.schema();
        Ok(match predicate {
            Predicate::All => Box::new(AllQuery),
            Predicate::Equals(field, Value::Text(value)) => {
                self.query(&Predicate::TextContains(field.clone(), value.clone()))?
            }
            Predicate::Equals(field, value) => {
                let f = names.get_field(field)?;
                let term = match value {
                    Value::Keyword(v) => Term::from_field_text(f, v),
                    Value::Text(_) => unreachable!("text equality handled above"),
                    Value::Integer(v) | Value::TimestampMillis(v) => Term::from_field_i64(f, *v),
                    Value::Boolean(v) => Term::from_field_bool(f, *v),
                };
                Box::new(TermQuery::new(term, IndexRecordOption::Basic))
            }
            Predicate::Exists(field) => term_query(names.get_field(PRESENCE)?, field),
            Predicate::TextContains(field, needle) => {
                if needle.is_empty() {
                    term_query(names.get_field(PRESENCE)?, field)
                } else {
                    let gram_field = names.get_field(&gram_name(field))?;
                    let grams = grams(&needle.to_lowercase());
                    Box::new(BooleanQuery::new(
                        grams
                            .into_iter()
                            .map(|gram| (Occur::Must, term_query(gram_field, &gram)))
                            .collect(),
                    ))
                }
            }
            Predicate::Range {
                field,
                minimum,
                maximum,
            } => {
                let f = names.get_field(field)?;
                let to_term = |value: &Value| match value {
                    Value::Integer(v) | Value::TimestampMillis(v) => Term::from_field_i64(f, *v),
                    _ => unreachable!("validated numeric/timestamp range"),
                };
                let low = minimum
                    .as_ref()
                    .map_or(Bound::Unbounded, |v| Bound::Included(to_term(v)));
                let high = maximum
                    .as_ref()
                    .map_or(Bound::Unbounded, |v| Bound::Included(to_term(v)));
                // Two unbounded ends mean "present" rather than "all".
                if minimum.is_none() && maximum.is_none() {
                    term_query(names.get_field(PRESENCE)?, field)
                } else {
                    Box::new(RangeQuery::new(low, high))
                }
            }
            Predicate::And(parts) => {
                if parts.is_empty() {
                    Box::new(AllQuery)
                } else {
                    Box::new(BooleanQuery::new(
                        parts
                            .iter()
                            .map(|p| self.query(p).map(|q| (Occur::Must, q)))
                            .collect::<Result<Vec<_>, _>>()?,
                    ))
                }
            }
            Predicate::Or(parts) => {
                if parts.is_empty() {
                    Box::new(EmptyQuery)
                } else {
                    Box::new(BooleanQuery::new(
                        parts
                            .iter()
                            .map(|p| self.query(p).map(|q| (Occur::Should, q)))
                            .collect::<Result<Vec<_>, _>>()?,
                    ))
                }
            }
            Predicate::Not(child) => Box::new(BooleanQuery::new(vec![
                (Occur::Must, Box::new(AllQuery) as Box<dyn Query>),
                (Occur::MustNot, self.query(child)?),
            ])),
        })
    }
}

fn validate_revision(previous: Option<&Mutation>, next: &Mutation) -> Result<bool, SearchError> {
    let Some(previous) = previous else {
        return Ok(true);
    };
    let id = next.id().to_owned();
    if next.revision() < previous.revision() {
        return Err(SearchError::StaleRevision(id));
    }
    if next.revision() == previous.revision() {
        return match (previous, next) {
            (Mutation::Upsert { document: a }, Mutation::Upsert { document: b }) if a == b => {
                Ok(false)
            }
            (Mutation::Delete { .. }, Mutation::Delete { .. }) => Ok(false),
            (Mutation::Delete { .. }, Mutation::Upsert { .. }) => {
                Err(SearchError::StaleRevision(id))
            }
            _ => Err(SearchError::ConflictingRevision(id)),
        };
    }
    Ok(true)
}

fn build_schema(schema: &Schema) -> TantivySchema {
    let mut builder = TantivySchema::builder();
    builder.add_text_field(ID, STRING | STORED);
    builder.add_text_field(STATE, STRING);
    builder.add_text_field(PAYLOAD, STORED);
    builder.add_text_field(PRESENCE, STRING);
    builder.add_u64_field(VERSION, INDEXED);
    let grams = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER)
            .set_index_option(IndexRecordOption::Basic),
    );
    for (name, field_type) in &schema.fields {
        match field_type {
            FieldType::Keyword | FieldType::Text => {
                if *field_type == FieldType::Keyword {
                    builder.add_text_field(name, STRING);
                } else {
                    builder.add_text_field(name, STORED);
                }
                builder.add_text_field(&gram_name(name), grams.clone());
            }
            FieldType::Integer | FieldType::TimestampMillis => {
                builder.add_i64_field(name, INDEXED);
            }
            FieldType::Boolean => {
                builder.add_bool_field(name, INDEXED);
            }
        }
    }
    builder.build()
}

fn gram_name(field: &str) -> String {
    format!("{PREFIX}gram_{field}")
}

fn register_tokenizer(index: &Index) -> Result<(), IndexedError> {
    let tokenizer = NgramTokenizer::new(1, 3, false)?;
    index
        .tokenizers()
        .register(TOKENIZER, TextAnalyzer::builder(tokenizer).build());
    Ok(())
}

fn project(names: &TantivySchema, record: &Mutation) -> Result<TantivyDocument, IndexedError> {
    let mut doc = TantivyDocument::default();
    doc.add_text(names.get_field(ID)?, record.id());
    doc.add_text(
        names.get_field(STATE)?,
        if matches!(record, Mutation::Upsert { .. }) {
            "live"
        } else {
            "deleted"
        },
    );
    doc.add_text(names.get_field(PAYLOAD)?, serde_json::to_string(record)?);
    doc.add_u64(names.get_field(VERSION)?, 1);
    if let Mutation::Upsert { document } = record {
        for (field, value) in &document.fields {
            let field_id = names.get_field(field)?;
            doc.add_text(names.get_field(PRESENCE)?, field);
            match value {
                Value::Keyword(v) | Value::Text(v) => {
                    doc.add_text(field_id, v);
                    doc.add_text(names.get_field(&gram_name(field))?, v.to_lowercase());
                }
                Value::Integer(v) | Value::TimestampMillis(v) => doc.add_i64(field_id, *v),
                Value::Boolean(v) => doc.add_bool(field_id, *v),
            }
        }
    }
    Ok(doc)
}

fn decode(names: &TantivySchema, doc: &TantivyDocument) -> Result<Mutation, IndexedError> {
    let value = doc
        .get_first(names.get_field(PAYLOAD)?)
        .and_then(|v| v.as_str())
        .ok_or_else(|| IndexedError::CorruptIndex("missing stored payload".into()))?;
    let record: Mutation = serde_json::from_str(value)?;
    Ok(record)
}

fn load_state(
    reader: &IndexReader,
    index: &Index,
) -> Result<BTreeMap<String, Mutation>, IndexedError> {
    let names = index.schema();
    let searcher = reader.searcher();
    let mut state = BTreeMap::new();
    for address in searcher.search(&AllQuery, &DocSetCollector)? {
        let document: TantivyDocument = searcher.doc(address)?;
        let operation = decode(&names, &document)?;
        let id = operation.id().to_owned();
        if id.trim().is_empty() || state.insert(id.clone(), operation).is_some() {
            return Err(IndexedError::CorruptIndex(id));
        }
    }
    Ok(state)
}

fn term_query(field: tantivy::schema::Field, text: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, text),
        IndexRecordOption::Basic,
    ))
}

fn grams(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let size = chars.len().min(3);
    chars
        .windows(size)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn facet_value(value: &Value) -> String {
    match value {
        Value::Keyword(v) | Value::Text(v) => v.clone(),
        Value::Integer(v) | Value::TimestampMillis(v) => v.to_string(),
        Value::Boolean(v) => v.to_string(),
    }
}
