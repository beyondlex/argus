use crate::fs::RawItem;

/// A matched item with match positions for highlighting.
#[derive(Debug, Clone)]
pub struct MatchedItem {
    pub name: String,
    pub is_dir: bool,
    pub match_positions: Vec<usize>,
}

/// Match items against a query string.
///
/// Matching algorithm:
/// 1. First character filter — item's first char must match query's first char (case-insensitive)
/// 2. Subsequent characters — fuzzy match (characters must appear in order)
/// 3. If query is a single character, skip fuzzy matching (just prefix filter)
/// 4. If query starts with `*`, treat it as a substring match anywhere in the name
///
/// Results are sorted by match quality (fewer gaps = better match).
pub fn match_items(items: &[RawItem], query: &str) -> Vec<MatchedItem> {
    if query.is_empty() {
        return Vec::new();
    }

    let mut matched: Vec<MatchedItem> = Vec::new();

    if query.starts_with('*') {
        let sub = &query[1..];
        if sub.is_empty() {
            return Vec::new();
        }
        let lower_sub = sub.to_lowercase();
        for item in items {
            let name_lower = item.name.to_lowercase();
            match name_lower.find(&lower_sub) {
                Some(pos) => {
                    let match_positions: Vec<usize> = (pos..pos + sub.len()).collect();
                    matched.push(MatchedItem {
                        name: item.name.clone(),
                        is_dir: item.is_dir,
                        match_positions,
                    });
                }
                None => {}
            }
        }
        return matched;
    }

    let query_lower = query.to_lowercase();
    let query_chars: Vec<char> = query_lower.chars().collect();
    let first_char = query_chars[0];

    for item in items {
        let name_lower = item.name.to_lowercase();
        let name_chars: Vec<char> = name_lower.chars().collect();

        // First character must match
        if name_chars.is_empty() || name_chars[0] != first_char {
            continue;
        }

        // Single character query — just prefix match
        if query_chars.len() == 1 {
            matched.push(MatchedItem {
                name: item.name.clone(),
                is_dir: item.is_dir,
                match_positions: vec![0],
            });
            continue;
        }

        // Fuzzy match: all query characters must appear in order in the name
        match fuzzy_find_positions(&name_lower, &query_lower) {
            Some(positions) => {
                matched.push(MatchedItem {
                    name: item.name.clone(),
                    is_dir: item.is_dir,
                    match_positions: positions,
                });
            }
            None => {}
        }
    }

    // Sort by match quality: fewer total gaps = better
    matched.sort_by(|a, b| {
        let a_score = fuzzy_score(&a.match_positions);
        let b_score = fuzzy_score(&b.match_positions);
        a_score.cmp(&b_score).then(a.name.cmp(&b.name))
    });

    matched
}

/// Fuzzy find positions: return the positions in `name` where each character of `query` appears in order.
fn fuzzy_find_positions(name: &str, query: &str) -> Option<Vec<usize>> {
    let name_chars: Vec<char> = name.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    let mut positions = Vec::new();
    let mut name_idx = 0;

    for qc in &query_chars {
        while name_idx < name_chars.len() {
            if &name_chars[name_idx] == qc {
                positions.push(name_idx);
                name_idx += 1;
                break;
            }
            name_idx += 1;
        }
        if positions.len() < query_chars.len() && name_idx >= name_chars.len() {
            return None;
        }
    }

    if positions.len() == query_chars.len() {
        Some(positions)
    } else {
        None
    }
}

/// Score match positions: lower is better. Sum of gaps between consecutive matches.
fn fuzzy_score(positions: &[usize]) -> usize {
    if positions.len() <= 1 {
        return 0;
    }
    let mut score = 0;
    for i in 1..positions.len() {
        let gap = positions[i].saturating_sub(positions[i - 1] + 1);
        score += gap;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str, is_dir: bool) -> RawItem {
        RawItem {
            name: name.to_string(),
            is_dir,
        }
    }

    #[test]
    fn test_empty_query() {
        let items = vec![raw("foo", false)];
        assert!(match_items(&items, "").is_empty());
    }

    #[test]
    fn test_prefix_match() {
        let items = vec![raw("foo", false), raw("bar", false), raw("foobar", false)];
        let result = match_items(&items, "fo");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "foo");
        assert_eq!(result[1].name, "foobar");
    }

    #[test]
    fn test_first_char_filter() {
        let items = vec![
            raw("alpha", false),
            raw("beta", false),
            raw("gamma", false),
        ];
        let result = match_items(&items, "b");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "beta");
    }

    #[test]
    fn test_case_insensitive() {
        let items = vec![raw("Alpha", false), raw("beta", false)];
        let result = match_items(&items, "a");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Alpha");
    }

    #[test]
    fn test_star_wildcard() {
        let items = vec![
            raw("hello.txt", false),
            raw("world.txt", false),
            raw("help.me", false),
        ];
        let result = match_items(&items, "*lo");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "hello.txt");
    }

    #[test]
    fn test_star_anywhere() {
        let items = vec![raw("abc.txt", false), raw("xabc", false), raw("ab", false)];
        let result = match_items(&items, "*bc");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_fuzzy_match_positions() {
        let positions = fuzzy_find_positions("hello", "hlo");
        assert_eq!(positions, Some(vec![0, 2, 4]));
    }

    #[test]
    fn test_fuzzy_no_match() {
        let positions = fuzzy_find_positions("hello", "hx");
        assert_eq!(positions, None);
    }

    #[test]
    fn test_match_positions_in_result() {
        let items = vec![raw("hello_world", false)];
        let result = match_items(&items, "hwd");
        assert_eq!(result.len(), 1);
        // h at 0, w at 6 (?), d at 10
        assert_eq!(result[0].match_positions, vec![0, 6, 10]);
    }

    #[test]
    fn test_sort_by_quality() {
        let items = vec![raw("abbbc", false), raw("abc", false)];
        let result = match_items(&items, "abc");
        // "abc" should rank higher (contiguous) than "abbbc" (gap at b->c)
        assert_eq!(result[0].name, "abc");
        assert_eq!(result[1].name, "abbbc");
    }
}