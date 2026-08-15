//! Pure immutable policy for assigning a safe presentation category to a tool name.

use std::collections::BTreeMap;

use gent_types::ToolCategory;

/// An immutable tool-name registry supplied by composition or a signed catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolCatalog {
    categories: BTreeMap<String, ToolCategory>,
}

impl ToolCatalog {
    #[must_use]
    pub fn from_entries(entries: impl IntoIterator<Item = (String, ToolCategory)>) -> Self {
        Self {
            categories: entries.into_iter().collect(),
        }
    }

    /// Classifies only an exact registered name; unknown tools always remain `Other`.
    #[must_use]
    pub fn classify(&self, tool_name: &str) -> ToolCategory {
        self.categories
            .get(tool_name)
            .copied()
            .unwrap_or(ToolCategory::Other)
    }
}

#[cfg(test)]
mod tests {
    use gent_types::ToolCategory;

    use super::ToolCatalog;

    #[test]
    fn classification_is_exact_and_has_a_safe_fallback() {
        let catalog = ToolCatalog::from_entries([("read_file".into(), ToolCategory::File)]);
        assert_eq!(catalog.classify("read_file"), ToolCategory::File);
        assert_eq!(catalog.classify("read-file"), ToolCategory::Other);
    }
}
