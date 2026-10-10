use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct FolderFilter {
    chat_ids: HashSet<i64>,
}

impl FolderFilter {
    pub fn clear(&mut self) {
        self.chat_ids.clear();
    }

    pub fn contains(&self, chat_id: i64) -> bool {
        self.chat_ids.contains(&chat_id)
    }

    pub fn update(&mut self, chat_ids: HashSet<i64>) {
        self.chat_ids = chat_ids;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clear_removes_all_chat_ids() {
        let mut filter = FolderFilter::default();
        filter.update(HashSet::from([1, 2]));

        filter.clear();

        assert!(!filter.contains(1));
        assert!(!filter.contains(2));
    }

    #[test]
    fn test_contains_reflects_updated_chat_ids() {
        let mut filter = FolderFilter::default();

        filter.update(HashSet::from([1, 2]));

        assert!(filter.contains(1));
        assert!(filter.contains(2));
        assert!(!filter.contains(3));
    }

    #[test]
    fn test_update_replaces_previous_chat_ids() {
        let mut filter = FolderFilter::default();
        filter.update(HashSet::from([1]));

        filter.update(HashSet::from([2]));

        assert!(!filter.contains(1));
        assert!(filter.contains(2));
    }
}
