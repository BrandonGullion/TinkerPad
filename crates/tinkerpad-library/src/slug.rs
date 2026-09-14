//! Folder names from display names.

use std::path::Path;

/// Longest folder name produced, in characters.
const MAX_LEN: usize = 60;

/// Names Windows refuses as file or folder names, whatever the extension.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A folder-safe name: lowercase letters and digits (any script), with runs of
/// anything else turned into single hyphens. `"Fizz Buzz!"` becomes
/// `"fizz-buzz"`. Never empty, never a name Windows reserves.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut pending_hyphen = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.extend(c.to_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    let mut slug: String = slug.chars().take(MAX_LEN).collect();
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug = "playground".into();
    }
    if RESERVED.contains(&slug.as_str()) {
        slug.push_str("-playground");
    }
    slug
}

/// `slug` if no entry of that name exists in `parent`, otherwise `slug-2`,
/// `slug-3`, and so on. `except` is a folder name to treat as free (the
/// playground being renamed).
pub fn unique_folder(parent: &Path, slug: &str, except: Option<&str>) -> String {
    let taken = |candidate: &str| {
        let is_self = except.is_some_and(|e| e.eq_ignore_ascii_case(candidate));
        !is_self && parent.join(candidate).exists()
    };
    if !taken(slug) {
        return slug.to_owned();
    }
    (2..)
        .map(|n| format!("{slug}-{n}"))
        .find(|c| !taken(c))
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("FizzBuzz"), "fizzbuzz");
        assert_eq!(slugify("Fizz Buzz!"), "fizz-buzz");
        assert_eq!(slugify("  JSON -- explore  "), "json-explore");
        assert_eq!(slugify("Ünïcödé Straße"), "ünïcödé-straße");
        assert_eq!(slugify("日本語 test"), "日本語-test");
        assert_eq!(slugify("!!!"), "playground");
        assert_eq!(slugify(""), "playground");
        assert_eq!(slugify("CON"), "con-playground");
        assert_eq!(slugify(&"a".repeat(200)).chars().count(), MAX_LEN);
        assert!(!slugify(&format!("{} x", "a".repeat(59))).ends_with('-'));
    }
}
