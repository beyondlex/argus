use std::collections::HashMap;

use crate::model::{AiContext, DiffNode};

pub fn extract_feature(diff_tree: &DiffNode, target_path: &str) -> Option<AiContext> {
    let target_components: Vec<&str> = target_path.split('/').filter(|s| !s.is_empty()).collect();

    let subtree = find_subtree(diff_tree, &target_components)?;

    let mut top_large_files: Vec<(String, u64)> = Vec::new();
    collect_large_files(subtree, &mut top_large_files);
    top_large_files.sort_by_key(|k| std::cmp::Reverse(k.1));
    top_large_files.truncate(5);

    let mut ext_map: HashMap<String, u64> = HashMap::new();
    let mut total_size_for_ext = 0u64;
    collect_extensions(subtree, &mut ext_map, &mut total_size_for_ext);

    let mut primary_extensions: Vec<(String, f32)> = ext_map
        .into_iter()
        .map(|(ext, size)| {
            let ratio = if total_size_for_ext > 0 {
                size as f32 / total_size_for_ext as f32
            } else {
                0.0
            };
            (ext, ratio)
        })
        .collect();
    primary_extensions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    primary_extensions.truncate(5);

    let size_delta_mb = subtree.size_delta as f64 / (1024.0 * 1024.0);
    let current_size_mb = subtree.current_size as f64 / (1024.0 * 1024.0);

    Some(AiContext {
        target_path: target_path.to_string(),
        size_delta_mb,
        current_size_mb,
        top_large_files: top_large_files.into_iter().collect(),
        primary_extensions,
    })
}

fn find_subtree<'a>(node: &'a DiffNode, components: &[&str]) -> Option<&'a DiffNode> {
    if components.is_empty() || (components.len() == 1 && components[0] == node.name) {
        return Some(node);
    }

    if components.is_empty() {
        return Some(node);
    }

    if components[0] != node.name {
        return None;
    }

    let remaining = &components[1..];
    if remaining.is_empty() {
        return Some(node);
    }

    node.children.get(remaining[0]).and_then(|child| {
        if remaining.len() == 1 {
            Some(child)
        } else {
            find_subtree(child, remaining)
        }
    })
}

fn collect_large_files(node: &DiffNode, results: &mut Vec<(String, u64)>) {
    if !node.is_dir && node.current_size > 0 {
        results.push((node.name.clone(), node.current_size));
    }
    for child in node.children.values() {
        collect_large_files(child, results);
    }
}

fn collect_extensions(node: &DiffNode, ext_map: &mut HashMap<String, u64>, total: &mut u64) {
    if !node.is_dir && node.current_size > 0 {
        let ext = node
            .name
            .rsplit('.')
            .next()
            .filter(|s| *s != node.name)
            .map(|s| format!(".{}", s))
            .unwrap_or_else(|| "(none)".to_string());

        *ext_map.entry(ext).or_insert(0) += node.current_size;
        *total += node.current_size;
    }
    for child in node.children.values() {
        collect_extensions(child, ext_map, total);
    }
}

pub fn generate_prompt(context: &AiContext) -> String {
    format!(
        r#"You are a disk space analysis assistant. The user wants to understand the purpose of a directory and whether it can be safely deleted.

## Directory Info

- **Path**: {path}
- **Current Size**: {current:.1} MB
- **Size Change**: {delta:+.1} MB

## Top 5 Largest Files

{files}

## File Type Distribution

{exts}

## Task

Analyze this directory and answer:
1. What is this directory for (which software or system component created it)?
2. What are the risks of deleting it?
3. Are there safer cleanup suggestions?
4. Do you recommend deleting it?

Respond in JSON format:

```json
{{
  "label": "entity name",
  "description": "purpose description",
  "risk_level": "Safe|Low|Medium|High",
  "suggestion": "cleanup suggestion",
  "deletable": true|false,
  "confidence": 0.0-1.0
}}
```"#,
        path = context.target_path,
        current = context.current_size_mb,
        delta = context.size_delta_mb,
        files = format_top_large_files(&context.top_large_files),
        exts = format_extensions(&context.primary_extensions),
    )
}

fn format_top_large_files(files: &[(String, u64)]) -> String {
    if files.is_empty() {
        return "  (no large file changes)".to_string();
    }
    files
        .iter()
        .map(|(name, size)| format!("  - {} ({} KB)", name, size / 1024))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_extensions(exts: &[(String, f32)]) -> String {
    if exts.is_empty() {
        return "  (no file type data)".to_string();
    }
    exts.iter()
        .map(|(ext, ratio)| format!("  - {} ({:.1}%)", ext, ratio * 100.0))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_diff_node(
        name: &str,
        is_dir: bool,
        current_size: u64,
        size_delta: i64,
        children: Vec<DiffNode>,
    ) -> DiffNode {
        let mut map = HashMap::new();
        for child in children {
            map.insert(child.name.clone(), child);
        }
        DiffNode {
            name: name.to_string(),
            is_dir,
            current_size,
            size_delta,
            children: map,
        }
    }

    #[test]
    fn test_extract_feature_finds_subtree() {
        let tree = make_diff_node(
            "root",
            true,
            1_500_000,
            500_000,
            vec![make_diff_node(
                "target_dir",
                true,
                1_000_000,
                300_000,
                vec![
                    make_diff_node("big_file.iso", false, 800_000, 200_000, vec![]),
                    make_diff_node("small.txt", false, 200_000, 100_000, vec![]),
                ],
            )],
        );

        let context = extract_feature(&tree, "root/target_dir");
        assert!(context.is_some());
        let ctx = context.unwrap();
        assert_eq!(ctx.target_path, "root/target_dir");
        let expected_mb = 1_000_000.0 / (1024.0 * 1024.0);
        assert!((ctx.current_size_mb - expected_mb).abs() < 0.01);
        assert_eq!(ctx.top_large_files.len(), 2);
    }

    #[test]
    fn test_extract_feature_nonexistent_path() {
        let tree = make_diff_node("root", true, 0, 0, vec![]);
        let context = extract_feature(&tree, "root/nonexistent");
        assert!(context.is_none());
    }

    #[test]
    fn test_extract_feature_root_path() {
        let tree = make_diff_node(
            "root",
            true,
            100,
            50,
            vec![make_diff_node("file.txt", false, 100, 50, vec![])],
        );
        let context = extract_feature(&tree, "root");
        assert!(context.is_some());
    }

    #[test]
    fn test_extract_feature_with_full_path_fails() {
        let tree = make_diff_node("Downloads", true, 1000, 500, vec![]);
        let context = extract_feature(&tree, "/home/user/Downloads");
        assert!(context.is_none());
    }

    #[test]
    fn test_extract_feature_with_relative_subpath() {
        let tree = make_diff_node(
            "Downloads",
            true,
            2000,
            1000,
            vec![make_diff_node("30mb", true, 30_000_000, 30_000_000, vec![])],
        );
        let context = extract_feature(&tree, "Downloads/30mb");
        assert!(context.is_some());
        assert_eq!(context.unwrap().target_path, "Downloads/30mb");
    }

    #[test]
    fn test_generate_prompt_contains_path() {
        let context = AiContext {
            target_path: "/home/user/cache".to_string(),
            size_delta_mb: 150.5,
            current_size_mb: 500.0,
            top_large_files: vec![("cache.db".to_string(), 50_000_000)],
            primary_extensions: vec![(".db".to_string(), 0.8)],
        };
        let prompt = generate_prompt(&context);
        assert!(prompt.contains("/home/user/cache"));
        assert!(prompt.contains("500.0"));
        assert!(prompt.contains("150.5"));
    }

    #[test]
    fn test_collect_extensions() {
        let tree = make_diff_node(
            "root",
            true,
            300,
            100,
            vec![
                make_diff_node("file.log", false, 200, 100, vec![]),
                make_diff_node("data.bin", false, 100, 0, vec![]),
            ],
        );
        let mut ext_map = HashMap::new();
        let mut total = 0u64;
        collect_extensions(&tree, &mut ext_map, &mut total);
        assert_eq!(*ext_map.get(".log").unwrap(), 200);
        assert_eq!(*ext_map.get(".bin").unwrap(), 100);
        assert_eq!(total, 300);
    }

    #[test]
    fn test_find_subtree_deep() {
        let tree = make_diff_node(
            "a",
            true,
            0,
            0,
            vec![make_diff_node(
                "b",
                true,
                0,
                0,
                vec![make_diff_node("c", true, 100, 50, vec![])],
            )],
        );
        let found = find_subtree(&tree, &["a", "b", "c"]);
        assert!(found.is_some());
        assert_eq!(found.unwrap().size_delta, 50);

        let not_found = find_subtree(&tree, &["a", "x"]);
        assert!(not_found.is_none());
    }
}
