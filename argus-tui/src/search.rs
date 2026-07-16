/// Fuzzy substring match (case-insensitive) returning character indices for highlighting.
/// ASCII path avoids heap allocation; non-ASCII falls back to lowercase strings.
pub fn fuzzy_match_indices(query: &str, target: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return None;
    }
    if query.is_ascii() && target.is_ascii() {
        let target_bytes = target.as_bytes();
        let query_bytes = query.as_bytes();
        let qlen = query_bytes.len();
        if qlen > target_bytes.len() {
            return None;
        }
        for start in 0..=(target_bytes.len() - qlen) {
            if target_bytes[start..start + qlen].eq_ignore_ascii_case(query_bytes) {
                return Some((start..start + qlen).collect());
            }
        }
        return None;
    }

    let target_lc = target.to_lowercase();
    let query_lc = query.to_lowercase();
    let byte_pos = target_lc.find(&query_lc)?;
    let start = target_lc[..byte_pos].chars().count();
    let end = start + query_lc.chars().count();
    Some((start..end).collect())
}

/// Fuzzy substring match for command autocomplete filtering.
pub(crate) fn fuzzy_match(query: &str, target: &str) -> bool {
    let mut chars = target.chars();
    for qc in query.chars() {
        loop {
            match chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TreeNode;
    use argus_core::{FileNode, FileType, NodeIndex, Snapshot, ROOT_NODE};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn file_node(name: &str, size: u64) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: false,
            file_type: FileType::File,
            size,
            disk_usage: size,
            children: Vec::new(),
        }
    }

    fn dir_node(name: &str, children: Vec<(&str, NodeIndex)>) -> FileNode {
        FileNode {
            name: name.to_string(),
            parent: None,
            is_dir: true,
            file_type: FileType::Directory,
            size: 0,
            disk_usage: 0,
            children: children
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_fuzzy_match_indices_basic() {
        let result = fuzzy_match_indices("foo", "foobar");
        assert_eq!(result, Some(vec![0, 1, 2]));
    }

    #[test]
    fn test_fuzzy_match_indices_case_insensitive() {
        let result = fuzzy_match_indices("FOO", "foobar");
        assert_eq!(result, Some(vec![0, 1, 2]));
    }

    #[test]
    fn test_fuzzy_match_indices_no_match() {
        assert!(fuzzy_match_indices("xyz", "foobar").is_none());
    }

    #[test]
    fn test_fuzzy_match_indices_empty_query() {
        assert!(fuzzy_match_indices("", "foobar").is_none());
    }

    #[test]
    fn test_fuzzy_match_indices_query_longer_than_target() {
        assert!(fuzzy_match_indices("foobar", "foo").is_none());
    }

    #[test]
    fn test_fuzzy_match_basic() {
        assert!(fuzzy_match("sc", "scan"));
    }

    #[test]
    fn test_fuzzy_match_no_match() {
        assert!(!fuzzy_match("xyz", "scan"));
    }

    #[test]
    fn test_fuzzy_match_exact() {
        assert!(fuzzy_match("Scan", "Scan"));
    }
}
