//! `{placeholder}` substitution in plugin commands.
//!
//! Placeholders are the only templating in a manifest. A placeholder is a
//! name in braces, such as `{main}`. Write `{{` or `}}` for a literal brace.
//! Unknown names are rejected when the manifest loads, not when a command
//! runs.

/// Placeholders allowed in `[run]`, `[helper]`, `[lsp]`, and `[lsp.register]`.
pub const COMMAND: &[&str] = &[
    "main",
    "dir",
    "id",
    "name",
    "scratch",
    "plugin_dir",
    "version",
    "toolchain",
];

/// Placeholders allowed in `[toolchain].probe` entries. Nothing about a
/// playground or the toolchain is known yet when probing.
pub const PROBE: &[&str] = &["plugin_dir", "scratch"];

/// Placeholders allowed in `[deps]`. Here `{name}` and `{version}` mean the
/// package being added, not the playground or toolchain.
pub const DEPS: &[&str] = &["name", "version"];

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaceholderError {
    #[error("unknown placeholder `{{{name}}}`")]
    Unknown { name: String },
    #[error("`{{` at position {at} doesn't start a placeholder; write `{{{{` for a literal brace")]
    Malformed { at: usize },
}

enum Piece<'a> {
    Text(&'a str),
    Placeholder(&'a str),
}

fn parse(template: &str) -> Result<Vec<Piece<'_>>, PlaceholderError> {
    let mut pieces = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                pieces.push(Piece::Text(&template[text_start..i + 1]));
                i += 2;
                text_start = i;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                pieces.push(Piece::Text(&template[text_start..i + 1]));
                i += 2;
                text_start = i;
            }
            b'{' => {
                let name_start = i + 1;
                let mut j = name_start;
                while j < bytes.len() && is_name_byte(bytes[j]) {
                    j += 1;
                }
                if j == name_start || bytes.get(j) != Some(&b'}') {
                    return Err(PlaceholderError::Malformed { at: i });
                }
                pieces.push(Piece::Text(&template[text_start..i]));
                pieces.push(Piece::Placeholder(&template[name_start..j]));
                i = j + 1;
                text_start = i;
            }
            _ => i += 1,
        }
    }
    pieces.push(Piece::Text(&template[text_start..]));
    Ok(pieces)
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
}

/// Check that `template` only uses placeholders from `allowed`.
pub fn validate(template: &str, allowed: &[&str]) -> Result<(), PlaceholderError> {
    for piece in parse(template)? {
        if let Piece::Placeholder(name) = piece
            && !allowed.contains(&name)
        {
            return Err(PlaceholderError::Unknown {
                name: name.to_owned(),
            });
        }
    }
    Ok(())
}

/// Replace every placeholder with its value. A placeholder with no value in
/// `values` is an error.
pub fn substitute(template: &str, values: &[(&str, &str)]) -> Result<String, PlaceholderError> {
    let mut out = String::with_capacity(template.len());
    for piece in parse(template)? {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Placeholder(name) => {
                let value = values
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| PlaceholderError::Unknown {
                        name: name.to_owned(),
                    })?;
                out.push_str(value);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUES: &[(&str, &str)] = &[
        ("main", "/pg/fizz/main.cs"),
        ("dir", "/pg/fizz"),
        ("plugin_dir", "/plugins/csharp"),
    ];

    #[test]
    fn substitutes_placeholders() {
        assert_eq!(substitute("{main}", VALUES).unwrap(), "/pg/fizz/main.cs");
        assert_eq!(
            substitute("{plugin_dir}/helper/bin", VALUES).unwrap(),
            "/plugins/csharp/helper/bin"
        );
        assert_eq!(
            substitute("--root={dir} --file={main}", VALUES).unwrap(),
            "--root=/pg/fizz --file=/pg/fizz/main.cs"
        );
    }

    #[test]
    fn text_without_placeholders_is_unchanged() {
        assert_eq!(substitute("run", VALUES).unwrap(), "run");
        assert_eq!(substitute("", VALUES).unwrap(), "");
        assert_eq!(substitute("a } b", VALUES).unwrap(), "a } b");
    }

    #[test]
    fn doubled_braces_are_literal() {
        assert_eq!(
            substitute(r#"{{"root":"{dir}"}}"#, VALUES).unwrap(),
            r#"{"root":"/pg/fizz"}"#
        );
    }

    #[test]
    fn unknown_placeholder_is_an_error() {
        assert_eq!(
            validate("{mian}", COMMAND),
            Err(PlaceholderError::Unknown {
                name: "mian".into()
            })
        );
        assert_eq!(
            substitute("{scratch}", VALUES),
            Err(PlaceholderError::Unknown {
                name: "scratch".into()
            })
        );
    }

    #[test]
    fn context_decides_what_is_allowed() {
        assert!(validate("{name}@{version}", DEPS).is_ok());
        assert!(validate("{main}", DEPS).is_err());
        assert!(validate("{plugin_dir}/probe", PROBE).is_ok());
        assert!(validate("{toolchain}", PROBE).is_err());
    }

    #[test]
    fn malformed_braces_are_errors() {
        assert_eq!(
            validate("{", COMMAND),
            Err(PlaceholderError::Malformed { at: 0 })
        );
        assert_eq!(
            validate("x {main", COMMAND),
            Err(PlaceholderError::Malformed { at: 2 })
        );
        assert_eq!(
            validate("{Main}", COMMAND),
            Err(PlaceholderError::Malformed { at: 0 })
        );
        assert_eq!(
            validate("{}", COMMAND),
            Err(PlaceholderError::Malformed { at: 0 })
        );
    }
}
