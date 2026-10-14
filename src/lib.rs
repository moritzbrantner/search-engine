use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// The declared type of a field in a searchable projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldType {
    Keyword,
    Text,
    Integer,
    Boolean,
    TimestampMillis,
}

/// A typed field value; absent fields are distinct from every concrete value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Keyword(String),
    Text(String),
    Integer(i64),
    Boolean(bool),
    TimestampMillis(i64),
}

impl Value {
    pub fn field_type(&self) -> FieldType {
        match self {
            Self::Keyword(_) => FieldType::Keyword,
            Self::Text(_) => FieldType::Text,
            Self::Integer(_) => FieldType::Integer,
            Self::Boolean(_) => FieldType::Boolean,
            Self::TimestampMillis(_) => FieldType::TimestampMillis,
        }
    }
}

/// A consumer-owned projection schema, not a filesystem metadata extractor.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    fields: BTreeMap<String, FieldType>,
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, name: impl Into<String>, kind: FieldType) -> Self {
        self.fields.insert(name.into(), kind);
        self
    }

    pub fn field_type(&self, name: &str) -> Option<FieldType> {
        self.fields.get(name).copied()
    }
}

/// One derived search record with an application-provided stable ID and revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub id: String,
    pub revision: u64,
    pub fields: BTreeMap<String, Value>,
}

impl Document {
    pub fn new(id: impl Into<String>, revision: u64) -> Self {
        Self {
            id: id.into(),
            revision,
            fields: BTreeMap::new(),
        }
    }

    pub fn field(mut self, name: impl Into<String>, value: Value) -> Self {
        self.fields.insert(name.into(), value);
        self
    }
}

/// A mechanical query language. TextContains is a reference substring test,
/// not tokenization, BM25, fuzzy search, or model-driven retrieval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Predicate {
    All,
    Equals(String, Value),
    Exists(String),
    TextContains(String, String),
    Range {
        field: String,
        minimum: Option<Value>,
        maximum: Option<Value>,
    },
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sort {
    pub field: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    pub predicate: Predicate,
    pub sort: Option<Sort>,
    pub offset: usize,
    pub limit: usize,
}

impl SearchRequest {
    pub fn new(predicate: Predicate) -> Self {
        Self {
            predicate,
            sort: None,
            offset: 0,
            limit: 20,
        }
    }

    pub fn sort_by(mut self, field: impl Into<String>, descending: bool) -> Self {
        self.sort = Some(Sort {
            field: field.into(),
            descending,
        });
        self
    }

    pub fn page(mut self, offset: usize, limit: usize) -> Self {
        self.offset = offset;
        self.limit = limit;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResults {
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    EmptyDocumentId,
    UnknownField(String),
    WrongType {
        field: String,
        expected: FieldType,
        actual: FieldType,
    },
    UnsupportedTextField(String),
    UnsupportedRangeField(String),
    InvalidRange(String),
    StaleRevision(String),
    ConflictingRevision(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDocumentId => write!(f, "document ID must not be empty"),
            Self::UnknownField(field) => write!(f, "unknown field: {field}"),
            Self::WrongType {
                field,
                expected,
                actual,
            } => write!(f, "wrong type for {field}: expected {expected:?}, got {actual:?}"),
            Self::UnsupportedTextField(field) => {
                write!(f, "text search requires a text or keyword field: {field}")
            }
            Self::UnsupportedRangeField(field) => {
                write!(f, "range search requires an integer or timestamp field: {field}")
            }
            Self::InvalidRange(field) => write!(f, "lower bound exceeds upper bound: {field}"),
            Self::StaleRevision(id) => write!(f, "stale revision for document: {id}"),
            Self::ConflictingRevision(id) => {
                write!(f, "same revision has different content: {id}")
            }
        }
    }
}

impl Error for SearchError {}

/// Deterministic O(n) reference oracle. This is NOT the production index.
///
/// Keep this independent of the eventual Tantivy/other indexed backend so
/// differential tests have a straightforward correctness reference.
#[derive(Debug)]
pub struct ReferenceIndex {
    schema: Schema,
    documents: BTreeMap<String, Document>,
}

impl ReferenceIndex {
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            documents: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Returns true when the record changed, false for an identical replay.
    pub fn upsert(&mut self, document: Document) -> Result<bool, SearchError> {
        if document.id.trim().is_empty() {
            return Err(SearchError::EmptyDocumentId);
        }

        for (field, value) in &document.fields {
            self.validate_value(field, value)?;
        }

        if let Some(previous) = self.documents.get(&document.id) {
            if document.revision < previous.revision {
                return Err(SearchError::StaleRevision(document.id));
            }
            if document.revision == previous.revision {
                if document == *previous {
                    return Ok(false);
                }
                return Err(SearchError::ConflictingRevision(document.id));
            }
        }

        self.documents.insert(document.id.clone(), document);
        Ok(true)
    }

    /// Idempotently removes a derived record. Source content is not touched.
    pub fn delete(&mut self, id: &str) -> bool {
        self.documents.remove(id).is_some()
    }

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResults, SearchError> {
        self.validate_predicate(&request.predicate)?;
        if let Some(sort) = &request.sort {
            self.known_field(&sort.field)?;
        }

        let mut matching: Vec<&Document> = self
            .documents
            .values()
            .filter(|document| matches_predicate(document, &request.predicate))
            .collect();
        let total = matching.len();

        if let Some(sort) = &request.sort {
            matching.sort_by(|left, right| {
                compare_optional(
                    left.fields.get(&sort.field),
                    right.fields.get(&sort.field),
                    sort.descending,
                )
                .then_with(|| left.id.cmp(&right.id))
            });
        }

        let hits = matching
            .into_iter()
            .skip(request.offset)
            .take(request.limit)
            .map(|doc| SearchHit {
                id: doc.id.clone(),
                revision: doc.revision,
            })
            .collect();

        Ok(SearchResults { total, hits })
    }

    fn known_field(&self, field: &str) -> Result<FieldType, SearchError> {
        self.schema
            .field_type(field)
            .ok_or_else(|| SearchError::UnknownField(field.to_owned()))
    }

    fn validate_value(&self, field: &str, value: &Value) -> Result<(), SearchError> {
        let expected = self.known_field(field)?;
        let actual = value.field_type();
        if expected != actual {
            return Err(SearchError::WrongType {
                field: field.to_owned(),
                expected,
                actual,
            });
        }
        Ok(())
    }

    fn validate_predicate(&self, predicate: &Predicate) -> Result<(), SearchError> {
        match predicate {
            Predicate::All => Ok(()),
            Predicate::Equals(field, value) => self.validate_value(field, value),
            Predicate::Exists(field) => self.known_field(field).map(|_| ()),
            Predicate::TextContains(field, _) => {
                let kind = self.known_field(field)?;
                if matches!(kind, FieldType::Text | FieldType::Keyword) {
                    Ok(())
                } else {
                    Err(SearchError::UnsupportedTextField(field.clone()))
                }
            }
            Predicate::Range {
                field,
                minimum,
                maximum,
            } => {
                let kind = self.known_field(field)?;
                if !matches!(kind, FieldType::Integer | FieldType::TimestampMillis) {
                    return Err(SearchError::UnsupportedRangeField(field.clone()));
                }
                if let Some(value) = minimum {
                    self.validate_value(field, value)?;
                }
                if let Some(value) = maximum {
                    self.validate_value(field, value)?;
                }
                if let (Some(left), Some(right)) = (minimum, maximum)
                    && compare_values(left, right) == Some(Ordering::Greater)
                {
                    return Err(SearchError::InvalidRange(field.clone()));
                }
                Ok(())
            }
            Predicate::And(children) | Predicate::Or(children) => {
                for child in children {
                    self.validate_predicate(child)?;
                }
                Ok(())
            }
            Predicate::Not(child) => self.validate_predicate(child),
        }
    }
}

fn matches_predicate(document: &Document, predicate: &Predicate) -> bool {
    match predicate {
        Predicate::All => true,
        Predicate::Equals(field, value) => document.fields.get(field) == Some(value),
        Predicate::Exists(field) => document.fields.contains_key(field),
        Predicate::TextContains(field, text) => {
            let haystack = match document.fields.get(field) {
                Some(Value::Text(value) | Value::Keyword(value)) => value,
                _ => return false,
            };
            haystack.to_lowercase().contains(&text.to_lowercase())
        }
        Predicate::Range {
            field,
            minimum,
            maximum,
        } => {
            let Some(actual) = document.fields.get(field) else {
                return false;
            };
            minimum.as_ref().is_none_or(|lower| {
                compare_values(actual, lower).is_some_and(|order| order != Ordering::Less)
            }) && maximum.as_ref().is_none_or(|upper| {
                compare_values(actual, upper).is_some_and(|order| order != Ordering::Greater)
            })
        }
        Predicate::And(children) => children
            .iter()
            .all(|child| matches_predicate(document, child)),
        Predicate::Or(children) => children
            .iter()
            .any(|child| matches_predicate(document, child)),
        Predicate::Not(child) => !matches_predicate(document, child),
    }
}

fn compare_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Keyword(a), Value::Keyword(b)) | (Value::Text(a), Value::Text(b)) => {
            Some(a.cmp(b))
        }
        (Value::Integer(a), Value::Integer(b))
        | (Value::TimestampMillis(a), Value::TimestampMillis(b)) => Some(a.cmp(b)),
        (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn compare_optional(left: Option<&Value>, right: Option<&Value>, descending: bool) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            let order = compare_values(left, right).unwrap_or(Ordering::Equal);
            if descending { order.reverse() } else { order }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
