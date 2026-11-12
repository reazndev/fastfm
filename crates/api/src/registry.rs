//! Method lookup by case-insensitive name.

use std::collections::HashMap;
use std::sync::Arc;

use crate::method::Method;

/// The set of methods the dispatcher can call.
#[derive(Clone, Default)]
pub struct Registry {
    methods: HashMap<String, Arc<dyn Method>>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a method, replacing one with the same name.
    #[must_use]
    pub fn with(mut self, method: impl Method + 'static) -> Self {
        self.methods
            .insert(method.name().to_ascii_lowercase(), Arc::new(method));
        self
    }

    /// The method with this name, ignoring ASCII case like Last.fm.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Method>> {
        self.methods.get(&name.to_ascii_lowercase())
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names: Vec<&str> = self.methods.values().map(|m| m.name()).collect();
        names.sort_unstable();
        f.debug_struct("Registry").field("methods", &names).finish()
    }
}
