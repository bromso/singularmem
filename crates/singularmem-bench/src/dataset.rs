//! `LongMemEval` dataset loader. One JSON array; each element is a question
//! with its own haystack of sessions. Unknown fields are ignored.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// `LongMemEval` question categories. Unknown strings are preserved in
/// [`QuestionType::Other`] so a new dataset revision still loads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum QuestionType {
    SingleSessionUser,
    SingleSessionAssistant,
    SingleSessionPreference,
    MultiSession,
    TemporalReasoning,
    KnowledgeUpdate,
    Other(String),
}

impl QuestionType {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::SingleSessionUser => "single-session-user",
            Self::SingleSessionAssistant => "single-session-assistant",
            Self::SingleSessionPreference => "single-session-preference",
            Self::MultiSession => "multi-session",
            Self::TemporalReasoning => "temporal-reasoning",
            Self::KnowledgeUpdate => "knowledge-update",
            Self::Other(s) => s,
        }
    }
}

impl From<&str> for QuestionType {
    fn from(s: &str) -> Self {
        match s {
            "single-session-user" => Self::SingleSessionUser,
            "single-session-assistant" => Self::SingleSessionAssistant,
            "single-session-preference" => Self::SingleSessionPreference,
            "multi-session" => Self::MultiSession,
            "temporal-reasoning" => Self::TemporalReasoning,
            "knowledge-update" => Self::KnowledgeUpdate,
            other => Self::Other(other.to_string()),
        }
    }
}

impl From<String> for QuestionType {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

impl From<QuestionType> for String {
    fn from(k: QuestionType) -> Self {
        k.as_str().to_string()
    }
}

impl fmt::Display for QuestionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub date: Option<String>,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub kind: QuestionType,
    /// `question_id` ends with `_abs`: no evidence session exists.
    pub abstention: bool,
    pub text: String,
    pub date: Option<String>,
    pub haystack: Vec<Session>,
    pub evidence: HashSet<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a LongMemEval JSON array: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("question {index} ({id}): {field} has {actual} entries, expected {expected}")]
    Shape {
        index: usize,
        id: String,
        field: &'static str,
        expected: usize,
        actual: usize,
    },
}

#[derive(Deserialize)]
struct RawTurn {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct RawQuestion {
    question_id: String,
    #[serde(default)]
    question_type: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    question_date: Option<String>,
    #[serde(default)]
    haystack_session_ids: Vec<String>,
    #[serde(default)]
    haystack_dates: Vec<String>,
    #[serde(default)]
    haystack_sessions: Vec<Vec<RawTurn>>,
    #[serde(default)]
    answer_session_ids: Vec<String>,
}

/// Load a `LongMemEval` file.
///
/// # Errors
/// [`Error::Io`] when the file cannot be read, [`Error::Json`] when it is
/// not the expected array, [`Error::Shape`] when a question's parallel
/// haystack arrays disagree in length.
pub fn load(path: &Path) -> Result<Vec<Question>, Error> {
    let shown = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|source| Error::Io {
        path: shown.clone(),
        source,
    })?;
    let reader = std::io::BufReader::new(file);
    let raw: Vec<RawQuestion> = serde_json::from_reader(reader).map_err(|source| Error::Json {
        path: shown,
        source,
    })?;
    raw.into_iter()
        .enumerate()
        .map(|(i, q)| convert(i, q))
        .collect()
}

fn convert(index: usize, raw: RawQuestion) -> Result<Question, Error> {
    let n = raw.haystack_session_ids.len();
    let check = |field: &'static str, actual: usize| {
        if actual == n {
            Ok(())
        } else {
            Err(Error::Shape {
                index,
                id: raw.question_id.clone(),
                field,
                expected: n,
                actual,
            })
        }
    };
    check("haystack_dates", raw.haystack_dates.len())?;
    check("haystack_sessions", raw.haystack_sessions.len())?;

    let haystack = raw
        .haystack_session_ids
        .into_iter()
        .zip(raw.haystack_dates)
        .zip(raw.haystack_sessions)
        .map(|((id, date), turns)| Session {
            id,
            date: Some(date),
            turns: turns
                .into_iter()
                .map(|t| Turn {
                    role: t.role,
                    content: t.content,
                })
                .collect(),
        })
        .collect();

    Ok(Question {
        abstention: raw.question_id.ends_with("_abs"),
        id: raw.question_id,
        kind: QuestionType::from(raw.question_type),
        text: raw.question,
        date: raw.question_date,
        haystack,
        evidence: raw.answer_session_ids.into_iter().collect(),
    })
}
