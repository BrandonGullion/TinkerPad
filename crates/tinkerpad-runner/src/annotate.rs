//! Finding positions in a run's output with the plugin's `[output].error_regex`
//! (docs/PLUGINS.md §2), so the UI can make the line clickable and mark the
//! spot in the editor.
//!
//! Lines from both streams are fed in as they complete. A single-line pattern
//! is matched against each line. A multi-line pattern (`error_regex_multiline`)
//! is matched against the last [`WINDOW`] lines joined with newlines, which is
//! what rustc needs: the message on one line and the location on the next.

use std::collections::VecDeque;

use tinkerpad_core::{Annotation, Level};
use tinkerpad_plugin::OutputSpec;

/// How many lines a multi-line pattern can span.
///
/// Rust needs two (message, then location). Python needs more: a traceback is
/// a frame line, the source line under it, sometimes a `~~^~~` marker, and
/// then the exception — and it is the *last* frame that matters, so the window
/// has to still hold it when the exception arrives (work item 03-04).
pub const WINDOW: usize = 6;

#[derive(Debug)]
pub struct Annotator {
    spec: OutputSpec,
    /// The last [`WINDOW`] lines, oldest first; only used when multi-line.
    recent: VecDeque<String>,
}

impl Annotator {
    pub fn new(spec: OutputSpec) -> Self {
        Self {
            spec,
            recent: VecDeque::with_capacity(WINDOW),
        }
    }

    /// Feed one output line (without its newline) and get the annotation it
    /// completes, if any.
    pub fn line(&mut self, line: &str) -> Option<Annotation> {
        if !self.spec.multiline {
            return self.capture(line, line);
        }
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(line.to_owned());
        // Match over the window, but only accept a match that reaches this
        // line: anything ending earlier was already reported when that line
        // was the newest.
        let window = Vec::from_iter(self.recent.iter().cloned()).join("\n");
        let end = window.len();
        let matched = self
            .spec
            .error_regex
            .captures_iter(&window)
            .find(|c| c.get(0).is_some_and(|m| m.end() == end))?;
        let text = matched.get(0).map_or(line, |m| m.as_str()).to_owned();
        annotation(&matched, &text)
    }

    /// The output is over: nothing is buffered between runs.
    pub fn finish(&mut self) {
        self.recent.clear();
    }

    fn capture(&self, haystack: &str, text: &str) -> Option<Annotation> {
        let matched = self.spec.error_regex.captures(haystack)?;
        annotation(&matched, text)
    }
}

fn annotation(captures: &regex::Captures<'_>, text: &str) -> Option<Annotation> {
    let group = |name: &str| captures.name(name).map(|m| m.as_str().trim().to_owned());
    let line = group("line")?.parse().ok()?;
    let severity = match group("severity").as_deref() {
        Some(s) if s.eq_ignore_ascii_case("warning") || s.eq_ignore_ascii_case("warn") => {
            Level::Warning
        }
        _ => Level::Error,
    };
    Some(Annotation {
        file: group("file").filter(|f| !f.is_empty()),
        line,
        col: group("col").and_then(|c| c.parse().ok()),
        message: group("message").unwrap_or_default(),
        severity,
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::*;

    fn annotator(pattern: &str, multiline: bool) -> Annotator {
        Annotator::new(OutputSpec {
            error_regex: Regex::new(pattern).unwrap(),
            multiline,
        })
    }

    /// The C# plugin's pattern, against real `dotnet` output.
    #[test]
    fn single_line_pattern() {
        let mut a = annotator(
            r"^(?<file>.+?\.cs)\((?<line>\d+),(?<col>\d+)\): (?<severity>error|warning) (?<message>[A-Z]+\d+: .*?)(?: \[[^\]]*\])?$",
            false,
        );
        assert_eq!(a.line("Hello from C#"), None, "ordinary output");
        let found = a
            .line(r"C:\play\main.cs(3,23): error CS1002: ; expected")
            .expect("matched");
        assert_eq!(found.file.as_deref(), Some(r"C:\play\main.cs"));
        assert_eq!((found.line, found.col), (3, Some(23)));
        assert_eq!(found.message, "CS1002: ; expected");
        assert_eq!(found.severity, Level::Error);

        let warning = a
            .line("/home/me/main.cs(9,13): warning CS0219: unused [/home/me/main.csproj]")
            .expect("matched");
        assert_eq!(warning.severity, Level::Warning);
        assert_eq!(warning.message, "CS0219: unused");
    }

    /// The Rust plugin's pattern: message on one line, location on the next.
    #[test]
    fn multi_line_pattern_reports_once_when_the_window_completes() {
        let mut a = annotator(
            r"(?m)^(?<severity>error|warning)(?:\[[A-Z]\d+\])?: (?<message>.+)\n\s*--> (?<file>.+?):(?<line>\d+):(?<col>\d+)$",
            true,
        );
        assert_eq!(a.line("   Compiling fizz-buzz v0.1.0"), None);
        assert_eq!(
            a.line("error[E0308]: mismatched types"),
            None,
            "location still to come"
        );
        let found = a.line("  --> src/main.rs:3:21").expect("matched");
        assert_eq!(found.file.as_deref(), Some("src/main.rs"));
        assert_eq!((found.line, found.col), (3, Some(21)));
        assert_eq!(found.message, "mismatched types");
        assert_eq!(
            found.text,
            "error[E0308]: mismatched types\n  --> src/main.rs:3:21"
        );

        // The same match must not be reported again as the window slides.
        assert_eq!(a.line("   |"), None);
        assert_eq!(a.line("3 |     let s: String = n;"), None);
    }

    /// The Python plugin's pattern, against a real traceback: the frame that
    /// matters is the one nearest the message, not the first.
    #[test]
    fn python_traceback_points_at_the_deepest_frame() {
        let mut a = annotator(
            r#"(?m)^  File "(?<file>[^"]+)", line (?<line>\d+)(?:, in [^\n]*)?\n(?:    [^\n]*\n)*(?<message>[A-Za-z_][A-Za-z0-9_.]*(?:Error|Exception|Exit|Warning|Interrupt)[^\n]*)$"#,
            true,
        );
        assert_eq!(a.line("before"), None);
        assert_eq!(a.line("Traceback (most recent call last):"), None);
        assert_eq!(
            a.line(r#"  File "C:\play\main.py", line 8, in <module>"#),
            None
        );
        assert_eq!(a.line("    boom()"), None);
        assert_eq!(a.line("    ~~~~^^"), None);
        assert_eq!(a.line(r#"  File "C:\play\main.py", line 5, in boom"#), None);
        assert_eq!(a.line("    return 1 / 0"), None);
        assert_eq!(a.line("           ~~^~~"), None);
        let found = a
            .line("ZeroDivisionError: division by zero")
            .expect("matched");
        assert_eq!(found.file.as_deref(), Some(r"C:\play\main.py"));
        assert_eq!(found.line, 5, "the frame the error happened in");
        assert_eq!(found.message, "ZeroDivisionError: division by zero");
    }

    #[test]
    fn a_pattern_without_optional_groups_still_works() {
        let mut a = annotator(r"^line (?<line>\d+): (?<message>.*)$", false);
        let found = a.line("line 7: something happened").expect("matched");
        assert_eq!(found.file, None);
        assert_eq!(found.col, None);
        assert_eq!(
            (found.line, found.message.as_str()),
            (7, "something happened")
        );
    }
}
