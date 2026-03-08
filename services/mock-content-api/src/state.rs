use crate::seed;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    content_type: String,
    items: HashMap<Uuid, String>,
}

impl AppState {
    pub fn new(content_type: String) -> Self {
        let items = seed::seeded_ids(&content_type);
        Self {
            content_type,
            items,
        }
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn get_title(&self, id: &Uuid) -> Option<&str> {
        self.items.get(id).map(|s| s.as_str())
    }
}
