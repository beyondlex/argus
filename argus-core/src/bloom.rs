use std::hash::{Hash, Hasher};

/// Bloom filter for inode dedup — detects hardlinks by tracking seen (device, inode) pairs.
///
/// 4 MB bit array (~32M bits), 9 hash functions.
/// At 2.4M entries → ~50% fill → ~0.12% false positive rate.
/// False positives cause ~0.12% undercount of unique files — acceptable per design choice.
pub(crate) struct SeenInodes {
    bits: Vec<u64>,
    bit_count: usize,
    count: usize,
}

impl SeenInodes {
    pub fn new() -> Self {
        let byte_size = 4 * 1024 * 1024;
        let bit_count = byte_size * 8;
        let elem_count = bit_count / 64;
        Self {
            bits: vec![0; elem_count],
            bit_count,
            count: 0,
        }
    }

    fn positions(&self, device: u64, inode: u64) -> [usize; 9] {
        let mut pos = [0usize; 9];
        for (i, p) in pos.iter_mut().enumerate() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            device.hash(&mut h);
            inode.hash(&mut h);
            (i as u32).hash(&mut h);
            *p = h.finish() as usize % self.bit_count;
        }
        pos
    }

    /// Returns `true` if key was NOT seen before (inserted now).
    /// Returns `false` if key was likely seen before (true hardlink or false positive).
    pub fn insert(&mut self, key: (u64, u64)) -> bool {
        let (device, inode) = key;
        let positions = self.positions(device, inode);

        let mut already_set = true;
        for &bit in &positions {
            let idx = bit / 64;
            let mask = 1u64 << (bit % 64);
            if self.bits[idx] & mask == 0 {
                already_set = false;
                self.bits[idx] |= mask;
            }
        }

        if already_set {
            return false;
        }
        self.count += 1;
        true
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.count
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_insert_and_detect_duplicates() {
        let mut s = SeenInodes::new();
        assert!(s.insert((1, 100)));
        assert!(!s.insert((1, 100)));
        assert!(s.insert((1, 200)));
        assert!(s.insert((2, 100)));
        assert!(!s.insert((1, 100)));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn test_no_false_negatives() {
        // After N unique inserts, re-inserting them all should return false (duplicate).
        // With 4 MB / 9 hashes, 10K entries has ~0% false positive rate.
        let mut s = SeenInodes::new();
        let keys: Vec<_> = (0..10_000).map(|i| (i as u64, i as u64 * 7 + 1)).collect();
        for &k in &keys {
            assert!(s.insert(k), "first insert of {k:?}");
        }
        let mut false_negatives = 0;
        for &k in &keys {
            if s.insert(k) {
                false_negatives += 1;
            }
        }
        assert!(
            false_negatives == 0,
            "expected 0 false negatives, got {false_negatives}"
        );
        assert_eq!(s.len(), keys.len());
    }

    #[test]
    fn test_empty() {
        let s = SeenInodes::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn test_false_positive_rate_within_budget() {
        let mut s = SeenInodes::new();
        let n = 50_000u64;
        for i in 0..n {
            s.insert((i, i * 3 + 1));
        }
        assert_eq!(s.len(), n as usize);

        // Test n new keys — count false positives (Bloom says "seen" for novel keys)
        let mut fps = 0u64;
        for i in n..2 * n {
            if !s.insert((i, i * 3 + 1)) {
                fps += 1;
            }
        }
        let rate = fps as f64 / n as f64;
        assert!(
            rate < 0.01,
            "false positive rate too high: {:.4} (expected < 0.01)",
            rate
        );
    }
}
