//! Finding a playground by what is in it (work item 04-01).
//!
//! The index holds each playground's name, tags, and the text of its main
//! file, all also kept lowercased so a search allocates nothing. Only the main
//! file is indexed: `runs.jsonl`, `node_modules`, `target`, `bin` and `obj`
//! are a playground's working mess, not its content.
//!
//! Matching is substring, case-insensitive, every term having to match
//! somewhere. A name also matches as a subsequence, so `fzbz` finds
//! "Fizz Buzz" — cheap, and the one bit of fuzziness that earns its keep.
//! Ranking is name, then tags, then source, with a prefix worth more than a
//! match in the middle.

use std::path::Path;

use serde::Serialize;
use tinkerpad_core::{Playground, PlaygroundSummary};

/// How much of a main file is indexed. Longer files are searched up to here;
/// nobody finds a playground by something 64 KiB into it.
pub const INDEX_BYTES: usize = 64 * 1024;

/// What a hit matched on, strongest first.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum MatchField {
    Name,
    Tag,
    Source,
}

/// A playground that matched, with where it matched and what it looked like.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(flatten)]
    pub summary: PlaygroundSummary,
    /// The strongest place it matched. `None` for an empty search, which
    /// matches everything and ranks nothing.
    pub field: Option<MatchField>,
    /// The line of the main file a source match was found in, trimmed.
    pub snippet: Option<String>,
    pub score: u32,
}

/// A playground's searchable text, built when the folder is indexed.
#[derive(Clone, Debug, Default)]
pub struct Indexed {
    name: String,
    tags: Vec<String>,
    /// The main file as written, capped at [`INDEX_BYTES`].
    text: String,
    /// The same bytes, ASCII-lowercased — the same length, so an offset found
    /// here slices the text above.
    lower: String,
    /// The first non-empty line, for the listing.
    pub preview: String,
    /// When the main file last changed, as at indexing time. Cached because a
    /// search that stats two thousand files is a search nobody types into.
    pub modified_ms: u64,
}

impl Indexed {
    /// Read what can be searched about a playground.
    pub fn of(playground: &Playground) -> Self {
        Self::from_text(playground, head_of(&playground.main_file))
    }

    /// The same, for text the caller already has — a file just written.
    pub fn from_text(playground: &Playground, text: String) -> Self {
        let mut text = text;
        if text.len() > INDEX_BYTES {
            let cut = (0..=INDEX_BYTES).rev().find(|n| text.is_char_boundary(*n));
            text.truncate(cut.unwrap_or(0));
        }
        let preview = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(shorten)
            .unwrap_or_default();
        Self {
            name: playground.name.to_lowercase(),
            tags: playground.tags.iter().map(|t| t.to_lowercase()).collect(),
            lower: text.to_ascii_lowercase(),
            text,
            preview,
            modified_ms: modified_ms(playground),
        }
    }

    /// Score this playground against one already-lowercased term, and say
    /// where the best match was.
    fn score(&self, term: &str) -> Option<(u32, MatchField, Option<String>)> {
        if let Some(at) = self.name.find(term) {
            let exact = self.name.len() == term.len();
            let word_start = at == 0 || self.name.as_bytes()[at - 1].is_ascii_whitespace();
            let score = 100 + u32::from(word_start) * 50 + u32::from(exact) * 100;
            return Some((score, MatchField::Name, None));
        }
        if self.tags.iter().any(|tag| tag.contains(term)) {
            let exact = self.tags.iter().any(|tag| tag == term);
            return Some((60 + u32::from(exact) * 30, MatchField::Tag, None));
        }
        if let Some(at) = self.lower.find(term) {
            return Some((20, MatchField::Source, Some(self.line_at(at))));
        }
        // Last and least: the name as a subsequence, so an abbreviation finds
        // it. Nothing else is fuzzy — a fuzzy match on a whole source file
        // matches everything.
        if subsequence(&self.name, term) {
            return Some((10, MatchField::Name, None));
        }
        None
    }

    /// The line of the main file containing byte offset `at`, trimmed and
    /// shortened.
    fn line_at(&self, at: usize) -> String {
        let start = self.text[..at].rfind('\n').map_or(0, |n| n + 1);
        let end = self.text[at..]
            .find('\n')
            .map_or(self.text.len(), |n| at + n);
        shorten(self.text[start..end].trim())
    }
}

/// Rank `playgrounds` against `query`, best first, at most `limit` of them.
///
/// Every whitespace-separated term has to match somewhere, so a longer query
/// narrows rather than widens. Pinned playgrounds come first — they were
/// pinned to be found — then score, then the most recently modified.
pub fn rank(
    playgrounds: impl Iterator<Item = (PlaygroundSummary, Indexed)>,
    query: &str,
    limit: usize,
) -> Vec<SearchHit> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .collect();

    let mut hits: Vec<SearchHit> = playgrounds
        .filter_map(|(summary, indexed)| {
            if terms.is_empty() {
                return Some(SearchHit {
                    summary,
                    field: None,
                    snippet: None,
                    score: 0,
                });
            }
            let mut score = 0;
            let mut field = MatchField::Source;
            let mut snippet = None;
            for term in &terms {
                let (points, where_, line) = indexed.score(term)?;
                score += points;
                if where_ < field {
                    field = where_;
                }
                snippet = snippet.or(line);
            }
            Some(SearchHit {
                summary,
                field: Some(field),
                snippet,
                score,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.summary
            .playground
            .pinned
            .cmp(&a.summary.playground.pinned)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| b.summary.modified_ms.cmp(&a.summary.modified_ms))
            .then_with(|| a.summary.playground.name.cmp(&b.summary.playground.name))
    });
    hits.truncate(limit);
    hits
}

/// When a playground's main file last changed, falling back to its metadata
/// file and then to when it was created.
fn modified_ms(p: &Playground) -> u64 {
    std::fs::metadata(&p.main_file)
        .or_else(|_| std::fs::metadata(p.dir.join(crate::meta::FILE_NAME)))
        .and_then(|m| m.modified())
        .map_or(p.created_ms, crate::meta::to_ms)
}

/// The first [`INDEX_BYTES`] of a file, as text. A file that isn't UTF-8 is
/// indexed as what it looks like in UTF-8, which is what the editor shows.
fn head_of(file: &Path) -> String {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(file) = std::fs::File::open(file) {
        let _ = file.take(INDEX_BYTES as u64).read_to_end(&mut buf);
    }
    // A cap can land mid-character; lossy conversion is the point.
    String::from_utf8_lossy(&buf).into_owned()
}

/// How long a preview or snippet is allowed to be.
const SHORT: usize = 120;

fn shorten(line: &str) -> String {
    let mut short: String = line.chars().take(SHORT).collect();
    if line.chars().count() > SHORT {
        short.push('…');
    }
    short
}

/// Whether `term`'s characters appear in `text` in order.
fn subsequence(text: &str, term: &str) -> bool {
    let mut chars = text.chars();
    term.chars().all(|wanted| chars.any(|c| c == wanted))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tinkerpad_core::{KindId, LanguageId, PlaygroundId};

    use super::*;

    fn playground(name: &str, tags: &[&str]) -> Playground {
        Playground {
            id: PlaygroundId::new(name),
            name: name.to_owned(),
            language: LanguageId::new("csharp"),
            kind: KindId::new("statements"),
            dir: PathBuf::from("/library").join(name),
            main_file: PathBuf::from("/library").join(name).join("main.cs"),
            created_ms: 0,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            pinned: false,
        }
    }

    /// An entry with its text given rather than read from a file.
    fn entry(name: &str, tags: &[&str], text: &str) -> (PlaygroundSummary, Indexed) {
        let playground = playground(name, tags);
        let indexed = Indexed {
            name: playground.name.to_lowercase(),
            tags: playground.tags.iter().map(|t| t.to_lowercase()).collect(),
            lower: text.to_ascii_lowercase(),
            text: text.to_owned(),
            preview: text.lines().next().unwrap_or_default().to_owned(),
            modified_ms: 0,
        };
        let summary = PlaygroundSummary {
            playground,
            modified_ms: 0,
            preview: indexed.preview.clone(),
        };
        (summary, indexed)
    }

    fn names(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter()
            .map(|hit| hit.summary.playground.name.as_str())
            .collect()
    }

    fn library() -> Vec<(PlaygroundSummary, Indexed)> {
        vec![
            entry("Fizz Buzz", &["katas"], "for (var i = 1; i < 100; i++)\n"),
            entry("JSON shapes", &["json", "api"], "var doc = Parse(json);\n"),
            entry("Scratch", &[], "// fizz something\nvar x = 1;\n"),
        ]
    }

    #[test]
    fn a_name_beats_a_tag_which_beats_the_source() {
        let hits = rank(library().into_iter(), "fizz", 10);
        assert_eq!(names(&hits), ["Fizz Buzz", "Scratch"]);
        assert_eq!(hits[0].field, Some(MatchField::Name));
        assert_eq!(hits[1].field, Some(MatchField::Source));
        assert_eq!(hits[1].snippet.as_deref(), Some("// fizz something"));
        assert!(hits[0].score > hits[1].score);

        let tagged = rank(library().into_iter(), "api", 10);
        assert_eq!(names(&tagged), ["JSON shapes"]);
        assert_eq!(tagged[0].field, Some(MatchField::Tag));
    }

    #[test]
    fn case_does_not_matter_and_every_term_must_match() {
        assert_eq!(
            names(&rank(library().into_iter(), "JsOn", 10)),
            ["JSON shapes"]
        );
        assert_eq!(
            names(&rank(library().into_iter(), "json parse", 10)),
            ["JSON shapes"]
        );
        assert!(rank(library().into_iter(), "json nothing", 10).is_empty());
    }

    #[test]
    fn an_abbreviation_finds_a_name() {
        assert_eq!(
            names(&rank(library().into_iter(), "fzbz", 10)),
            ["Fizz Buzz"]
        );
        // But not a source file: everything would match everything.
        assert!(rank(library().into_iter(), "vrx", 10).is_empty());
    }

    #[test]
    fn an_empty_search_is_everything_it_has() {
        let hits = rank(library().into_iter(), "   ", 10);
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|hit| hit.field.is_none()));
    }

    #[test]
    fn pinned_playgrounds_come_first() {
        let mut entries = library();
        entries[2].0.playground.pinned = true;
        let hits = rank(entries.into_iter(), "", 10);
        assert_eq!(names(&hits)[0], "Scratch");
    }

    #[test]
    fn the_limit_is_honoured() {
        assert_eq!(rank(library().into_iter(), "", 2).len(), 2);
    }
}
