//! "Did you mean …?" for typos in manifest keys and values.

/// The candidate closest to `word`, if it is close enough to plausibly be a
/// typo of it.
pub fn nearest<'a>(word: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).max(2);
    candidates
        .iter()
        .map(|c| (distance(word, c), *c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// Levenshtein edit distance.
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(distance("", ""), 0);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("idle_shutdwon", "idle_shutdown"), 2);
    }

    #[test]
    fn suggests_close_matches_only() {
        let keys = ["command", "args", "workspace", "idle_shutdown"];
        assert_eq!(nearest("idle_shutdwon", &keys), Some("idle_shutdown"));
        assert_eq!(nearest("comand", &keys), Some("command"));
        assert_eq!(nearest("banana", &keys), None);
    }
}
