use std::str::FromStr;

use automerge as am;

use crate::DocumentId;

#[derive(Clone, PartialEq, Eq)]
pub struct AutomergeUrl {
    document_id: DocumentId,
    path: Option<Vec<am::Prop>>,
}

impl AutomergeUrl {
    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }
}

impl From<&DocumentId> for AutomergeUrl {
    fn from(id: &DocumentId) -> Self {
        AutomergeUrl {
            document_id: id.clone(),
            path: None,
        }
    }
}

impl std::fmt::Display for AutomergeUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "automerge:{}", self.document_id)?;
        if let Some(path) = &self.path {
            for prop in path {
                match prop {
                    am::Prop::Seq(i) => write!(f, "/{i}")?,
                    am::Prop::Map(s) => write!(f, "/{s}")?,
                }
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for AutomergeUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AutomergeUrl({self})")
    }
}

impl FromStr for AutomergeUrl {
    type Err = InvalidUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some(suffix) = s.strip_prefix("automerge:") else {
            return Err(InvalidUrlError(format!("invalid automerge url: {s}")));
        };
        let mut parts = suffix.split("/");
        let Some(doc_id_part) = parts.next() else {
            return Err(InvalidUrlError(format!("invalid automerge url: {s}")));
        };
        let Ok(id) = DocumentId::from_str(doc_id_part) else {
            return Err(InvalidUrlError(format!("invalid automerge url: {s}")));
        };
        let props = parts
            .map(|p| {
                if let Ok(i) = usize::from_str(p) {
                    am::Prop::Seq(i)
                } else {
                    am::Prop::Map(p.to_string())
                }
            })
            .collect::<Vec<_>>();
        Ok(AutomergeUrl {
            document_id: id,
            path: if props.is_empty() { None } else { Some(props) },
        })
    }
}

pub struct InvalidUrlError(String);

impl std::fmt::Display for InvalidUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid Automerge URL: {}", self.0)
    }
}

impl std::fmt::Debug for InvalidUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InvalidUrlError({})", self.0)
    }
}

impl std::error::Error for InvalidUrlError {}
