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
    let byte_end = byte_pos + query_lc.len();
    // Map the matched range in lowered space back to original char indices.
    // `str::to_lowercase` can change the char count (`İ` → `i̇`), so counting
    // chars of the lowered string mis-locates the highlight in the original;
    // per-char lowercase lengths track which original chars overlap the range.
    // (Full case mapping's Final_Sigma rule (`Σ` → `ς` word-finally) is not
    // reproduced char-wise; in that rare case the highlight may be off by a
    // char, but the match itself is unaffected.)
    let mut indices = Vec::new();
    let mut lc_byte = 0usize;
    for (ci, ch) in target.chars().enumerate() {
        if lc_byte >= byte_end {
            break;
        }
        lc_byte += ch.to_lowercase().map(|c| c.len_utf8()).sum::<usize>();
        if lc_byte > byte_pos {
            indices.push(ci);
        }
    }
    Some(indices)
}

/// Fuzzy subsequence match for command autocomplete filtering: every query
/// char must appear in order in the target (unlike the substring-based
/// `fuzzy_match_indices`).
pub(crate) fn fuzzy_subsequence_match(query: &str, target: &str) -> bool {
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

    /// Non-ASCII targets whose lowercase form changes char count (`İ` →
    /// `i̇`, one char becomes two) must still map the highlight back to the
    /// original's char indices: counting chars of the lowered string pointed
    /// the highlight past the real position (here at a nonexistent index 3).
    #[test]
    fn test_fuzzy_match_indices_non_ascii_expanding_lowercase() {
        // "İf" lowercases to "i̇f" (3 chars from 2); matching "f" there
        // addresses original char 1, not 2.
        assert_eq!(fuzzy_match_indices("f", "İf"), Some(vec![1]));
        // Expansion *before* the match is the case that used to break:
        // "xİf" → lowered "xi̇f" (5 bytes), "f" matches at lowered byte 4,
        // original char index 2 — the old char count said 3 (out of range).
        assert_eq!(fuzzy_match_indices("f", "xİf"), Some(vec![2]));
        // The match may legitimately land inside the expansion: "i" matches
        // the 'i' that "İ" lowers into, so the highlight belongs to the İ.
        assert_eq!(fuzzy_match_indices("i", "İi"), Some(vec![0]));
        // CJK keeps a 1:1 char mapping; ordinary offsets are unchanged.
        assert_eq!(fuzzy_match_indices("界", "世界"), Some(vec![1]));
    }

    #[test]
    fn test_fuzzy_subsequence_match_basic() {
        assert!(fuzzy_subsequence_match("sc", "scan"));
    }

    #[test]
    fn test_fuzzy_subsequence_match_no_match() {
        assert!(!fuzzy_subsequence_match("xyz", "scan"));
    }

    #[test]
    fn test_fuzzy_subsequence_match_exact() {
        assert!(fuzzy_subsequence_match("Scan", "Scan"));
    }
}
