//! A service's environment as `.env` text (the Environment tab, templates):
//! `NAME=value` lines, `#` comments and blank lines skipped. A value may be
//! quoted: `"…"` takes the escapes `\n`, `\"` and `\\`; `'…'` is taken as
//! is. Pure functions.

use std::collections::BTreeMap;

use crate::services::deploy_service::valid_env_name;

/// What is wrong with a line (1-based), for the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotenvError {
    pub line: usize,
    pub message: String,
}

/// The variables of `text` (a name given twice: the last value).
pub fn parse(text: &str) -> Result<BTreeMap<String, String>, DotenvError> {
    let mut env = BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = |message: &str| DotenvError {
            line: index + 1,
            message: message.to_owned(),
        };
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            return Err(error("expected NAME=value"));
        };
        let name = name.trim();
        if !valid_env_name(name) {
            return Err(error(
                "a name is letters, digits and _, not starting with a digit",
            ));
        }
        let value = unquote(value.trim()).ok_or_else(|| error("a quote is not closed"))?;
        env.insert(name.to_owned(), value);
    }
    Ok(env)
}

fn unquote(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('\'') {
        return inner.strip_suffix('\'').map(str::to_owned);
    }
    let Some(inner) = value.strip_prefix('"') else {
        return Some(value.to_owned());
    };
    let inner = inner.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// `env` as `.env` text, one variable per line (quoted when it must be).
pub fn format(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(name, value)| format!("{name}={}\n", quote(value)))
        .collect()
}

fn quote(value: &str) -> String {
    // Parsing trims (Unicode whitespace too): such a value is quoted.
    let plain = !value.is_empty()
        && value.trim() == value
        && !value.starts_with(['"', '\''])
        && !value.contains(['\n', '\r', '\t', '#']);
    if plain || value.is_empty() {
        return value.to_owned();
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_comments_and_quotes() {
        let env = parse(
            "# a comment\n\nDB_HOST=shop_db\nexport MODE = prod\nGREETING=\"hello\\nworld\"\nRAW='a\\nb'\nURL=postgres://u:p@h/db?x=1\nEMPTY=\n",
        )
        .unwrap();
        assert_eq!(env["DB_HOST"], "shop_db");
        assert_eq!(env["MODE"], "prod");
        assert_eq!(env["GREETING"], "hello\nworld");
        assert_eq!(env["RAW"], "a\\nb");
        assert_eq!(env["URL"], "postgres://u:p@h/db?x=1");
        assert_eq!(env["EMPTY"], "");
    }

    #[test]
    fn errors_name_their_line() {
        assert_eq!(parse("OK=1\nnot a pair").unwrap_err().line, 2);
        assert_eq!(parse("1BAD=x").unwrap_err().line, 1);
        assert_eq!(parse("Q=\"open").unwrap_err().line, 1);
    }

    #[test]
    fn what_is_formatted_parses_back() {
        let env = BTreeMap::from([
            ("A".to_owned(), "plain".to_owned()),
            ("B".to_owned(), "two\nlines \"q\" \\".to_owned()),
            ("C".to_owned(), " padded ".to_owned()),
            ("D".to_owned(), "has # hash".to_owned()),
            ("E".to_owned(), String::new()),
            ("F".to_owned(), "\u{a0}nbsp\u{a0}".to_owned()),
        ]);
        assert_eq!(parse(&format(&env)).unwrap(), env);
    }
}
