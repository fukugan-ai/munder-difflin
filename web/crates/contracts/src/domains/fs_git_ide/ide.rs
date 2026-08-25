use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDocumentKind {
    Text,
    Image,
    WorkingTreeDiff,
    RevisionDiff,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IdeDocument {
    pub key: String,
    pub rel_path: String,
    pub kind: IdeDocumentKind,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeSaveState {
    Idle,
    Dirty,
    Saving,
    Saved,
    Failed,
}
