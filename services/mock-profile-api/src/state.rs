use crate::seed;
use std::collections::HashMap;

#[derive(Clone)]
pub struct AppState {
    tokens: HashMap<String, (String, String)>,
}

impl AppState {
    pub fn new() -> Self {
        let tokens = seed::tokens();
        Self { tokens }
    }

    pub fn tokens(&self) -> &HashMap<String, (String, String)> {
        &self.tokens
    }
}
