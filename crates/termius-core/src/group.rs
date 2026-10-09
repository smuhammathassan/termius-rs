//! Group: a hierarchical folder organizing hosts.

use serde::{Deserialize, Serialize};

use crate::{RecordId, Timestamp};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Group {
    pub id: RecordId,
    pub title: String,
    /// Parent group id for nesting (None = root).
    pub parent_id: Option<RecordId>,
    /// Hosts directly in this group.
    pub host_ids: Vec<RecordId>,
    pub sort_order: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            parent_id: None,
            host_ids: Vec::new(),
            sort_order: 0,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
