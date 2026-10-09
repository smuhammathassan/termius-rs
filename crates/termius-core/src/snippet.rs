//! Snippet: a reusable command/script with quick-insert bindings.

use serde::{Deserialize, Serialize};

use crate::{RecordId, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snippet {
    pub id: RecordId,
    pub title: String,
    pub body: String,
    /// Hosts this snippet is quick-inserted on.
    pub host_ids: Vec<RecordId>,
    /// Keyboard shortcut / command name, if bound.
    pub command: Option<String>,
    pub sort_order: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Snippet {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            body: String::new(),
            host_ids: Vec::new(),
            command: None,
            sort_order: 0,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
