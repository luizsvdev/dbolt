use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    /// Byte range of the statement, without its semicolon and the whitespace
    /// around it. Comments right before the statement belong to it.
    pub range: Range<usize>,
    /// Byte offset just past the semicolon (or the statement's end when it has
    /// none), so a cursor placed right after `;` still picks the statement.
    pub end_with_terminator: usize,
}

/// Splits SQL text on `;`, ignoring semicolons inside string literals, quoted
/// identifiers, dollar-quoted strings and comments. Statements made only of
/// comments are left out.
pub fn split_statements(text: &str) -> Vec<Statement> {
    let bytes = text.as_bytes();
    let mut statements = Vec::new();
    let mut start = None;
    let mut end = 0;
    let mut has_code = false;
    let mut index = 0;

    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        let token_start = index;
        let is_code = match byte {
            byte if byte.is_ascii_whitespace() => {
                index += 1;
                continue;
            }
            b'-' if next == Some(b'-') => {
                index = bytes[index..]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(bytes.len(), |offset| index + offset);
                false
            }
            b'/' if next == Some(b'*') => {
                index = skip_block_comment(bytes, index);
                false
            }
            b'\'' => {
                let backslash_escapes = is_escape_string_prefix(bytes, index);
                index = skip_quoted(bytes, index, b'\'', backslash_escapes);
                true
            }
            b'"' => {
                index = skip_quoted(bytes, index, b'"', false);
                true
            }
            b'$' => {
                index = skip_dollar_quoted(text, index).unwrap_or(index + 1);
                true
            }
            b';' => {
                if has_code && let Some(start) = start {
                    statements.push(Statement {
                        range: start..end,
                        end_with_terminator: index + 1,
                    });
                }
                start = None;
                has_code = false;
                index += 1;
                continue;
            }
            _ => {
                index += 1;
                true
            }
        };
        start.get_or_insert(token_start);
        end = index;
        has_code |= is_code;
    }

    if has_code && let Some(start) = start {
        statements.push(Statement {
            range: start..end,
            end_with_terminator: end,
        });
    }
    statements
}

/// The statement to run for a cursor at `offset`: the one containing it, else
/// the closest one before it, else the first one.
pub fn statement_at(statements: &[Statement], offset: usize) -> Option<&Statement> {
    statements
        .iter()
        .find(|statement| {
            statement.range.start <= offset && offset <= statement.end_with_terminator
        })
        .or_else(|| {
            statements
                .iter()
                .rev()
                .find(|statement| statement.end_with_terminator <= offset)
        })
        .or_else(|| statements.first())
}

/// The statements to run: every statement in the selection when there is one,
/// otherwise the statement under the cursor.
pub fn statements_to_run(text: &str, selection: Range<usize>) -> Vec<&str> {
    if selection.is_empty() {
        let statements = split_statements(text);
        return statement_at(&statements, selection.start)
            .and_then(|statement| text.get(statement.range.clone()))
            .into_iter()
            .collect();
    }
    text.get(selection)
        .map(all_statements)
        .unwrap_or_default()
}

pub fn all_statements(text: &str) -> Vec<&str> {
    split_statements(text)
        .into_iter()
        .filter_map(|statement| text.get(statement.range))
        .collect()
}

/// The relation named by the first top-level `FROM`, as written, so a result
/// can be titled after the table it came from.
pub fn from_target(statement: &str) -> Option<&str> {
    let bytes = statement.as_bytes();
    let mut depth = 0usize;
    let mut after_from = false;
    let mut index = 0;

    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        match byte {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if next == Some(b'-') => {
                index = bytes[index..]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(bytes.len(), |offset| index + offset);
            }
            b'/' if next == Some(b'*') => index = skip_block_comment(bytes, index),
            b'\'' => {
                let backslash_escapes = is_escape_string_prefix(bytes, index);
                index = skip_quoted(bytes, index, b'\'', backslash_escapes);
                after_from = false;
            }
            b'$' => {
                index = skip_dollar_quoted(statement, index).unwrap_or(index + 1);
                after_from = false;
            }
            b'(' => {
                depth += 1;
                index += 1;
                after_from = false;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b'"' => {
                if after_from && depth == 0 {
                    return Some(qualified_name_at(statement, index));
                }
                index = skip_quoted(bytes, index, b'"', false);
            }
            byte if is_identifier_byte(byte) => {
                let start = index;
                while bytes
                    .get(index)
                    .is_some_and(|&byte| is_identifier_byte(byte))
                {
                    index += 1;
                }
                if after_from && depth == 0 {
                    return Some(qualified_name_at(statement, start));
                }
                after_from = depth == 0
                    && statement
                        .get(start..index)
                        .is_some_and(|word| word.eq_ignore_ascii_case("from"));
            }
            _ => {
                index += 1;
                after_from = false;
            }
        }
    }
    None
}

/// Reads `name`, `"Quoted Name"` and dotted chains of them starting at `start`.
fn qualified_name_at(statement: &str, start: usize) -> &str {
    let bytes = statement.as_bytes();
    let mut index = start;
    loop {
        match bytes.get(index) {
            Some(b'"') => index = skip_quoted(bytes, index, b'"', false),
            Some(&byte) if is_identifier_byte(byte) => {
                while bytes
                    .get(index)
                    .is_some_and(|&byte| is_identifier_byte(byte))
                {
                    index += 1;
                }
            }
            _ => break,
        }
        if bytes.get(index) != Some(&b'.') {
            break;
        }
        index += 1;
    }
    statement
        .get(start..index)
        .unwrap_or_default()
        .trim_end_matches('.')
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// `E'...'` strings treat a backslash as an escape, so `E'\''` is one string.
fn is_escape_string_prefix(bytes: &[u8], quote: usize) -> bool {
    let Some(prefix) = quote.checked_sub(1) else {
        return false;
    };
    let is_e = matches!(bytes.get(prefix), Some(b'e' | b'E'));
    let starts_word = prefix
        .checked_sub(1)
        .and_then(|before| bytes.get(before))
        .is_none_or(|&byte| !is_identifier_byte(byte));
    is_e && starts_word
}

/// Returns the offset just past the closing quote, where a doubled quote is an
/// escaped one. An unterminated literal runs to the end of the text.
fn skip_quoted(bytes: &[u8], open: usize, quote: u8, backslash_escapes: bool) -> usize {
    let mut index = open + 1;
    while let Some(&byte) = bytes.get(index) {
        if backslash_escapes && byte == b'\\' {
            index += 2;
        } else if byte == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    bytes.len()
}

/// Block comments nest in PostgreSQL, so `/* /* */ ; */` is still one comment.
fn skip_block_comment(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut index = open;
    while let Some(&byte) = bytes.get(index) {
        let next = bytes.get(index + 1).copied();
        if byte == b'/' && next == Some(b'*') {
            depth += 1;
            index += 2;
        } else if byte == b'*' && next == Some(b'/') {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    bytes.len()
}

/// Skips a `$tag$ ... $tag$` string. Returns `None` when the `$` does not open
/// one, as in the parameter `$1` or the identifier `price$usd`.
fn skip_dollar_quoted(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open
        .checked_sub(1)
        .and_then(|before| bytes.get(before))
        .is_some_and(|&byte| is_identifier_byte(byte) || byte == b'$')
    {
        return None;
    }
    let mut index = open + 1;
    if bytes
        .get(index)
        .is_some_and(|&byte| byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80)
    {
        while bytes
            .get(index)
            .is_some_and(|&byte| is_identifier_byte(byte))
        {
            index += 1;
        }
    }
    if bytes.get(index) != Some(&b'$') {
        return None;
    }
    let tag = text.get(open..=index)?;
    let body_start = index + 1;
    let closing = text
        .get(body_start..)?
        .find(tag);
    Some(closing.map_or(bytes.len(), |offset| body_start + offset + tag.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statements(text: &str) -> Vec<&str> {
        split_statements(text)
            .into_iter()
            .map(|statement| &text[statement.range])
            .collect()
    }

    #[test]
    fn splits_on_semicolons_and_trims_whitespace() {
        assert_eq!(
            statements("select 1;\n  select 2 ;\n\nselect 3"),
            vec!["select 1", "select 2", "select 3"]
        );
    }

    #[test]
    fn skips_empty_and_comment_only_statements() {
        assert_eq!(
            statements(";; select 1;; -- trailing\n; /* only */ ;"),
            vec!["select 1"]
        );
        assert!(statements("").is_empty());
        assert!(statements("  \n -- nothing here").is_empty());
    }

    #[test]
    fn ignores_semicolons_in_string_literals() {
        assert_eq!(
            statements("select 'a;b', 'it''s; fine'; select 2"),
            vec!["select 'a;b', 'it''s; fine'", "select 2"]
        );
    }

    #[test]
    fn honours_backslash_escapes_only_in_escape_strings() {
        assert_eq!(
            statements(r"select E'\';'; select 2"),
            vec![r"select E'\';'", "select 2"]
        );
        // In a standard string a backslash is an ordinary character.
        assert_eq!(
            statements(r"select '\'; select 2"),
            vec![r"select '\'", "select 2"]
        );
        // `e` at the end of an identifier does not start an escape string.
        assert_eq!(
            statements(r"select name'\'; select 2"),
            vec![r"select name'\'", "select 2"]
        );
    }

    #[test]
    fn ignores_semicolons_in_quoted_identifiers() {
        assert_eq!(
            statements(r#"select "a;b", "say ""hi;"""; select 2"#),
            vec![r#"select "a;b", "say ""hi;""""#, "select 2"]
        );
    }

    #[test]
    fn ignores_semicolons_in_comments() {
        assert_eq!(
            statements("select 1 -- not; the end\n, 2; select 3"),
            vec!["select 1 -- not; the end\n, 2", "select 3"]
        );
        assert_eq!(
            statements("select /* outer /* inner; */ still; */ 1; select 2"),
            vec!["select /* outer /* inner; */ still; */ 1", "select 2"]
        );
    }

    #[test]
    fn leading_comments_belong_to_the_next_statement() {
        assert_eq!(
            statements("select 1;\n-- second\nselect 2;"),
            vec!["select 1", "-- second\nselect 2"]
        );
    }

    #[test]
    fn ignores_semicolons_in_dollar_quoted_strings() {
        let function =
            "create function f() returns int as $$ begin return 1; end; $$ language plpgsql";
        assert_eq!(
            statements(&format!("{function}; select 2")),
            vec![function, "select 2"]
        );
        let tagged = "select $body$ a; $$ b; $body$";
        assert_eq!(
            statements(&format!("{tagged}; select 2")),
            vec![tagged, "select 2"]
        );
    }

    #[test]
    fn parameters_and_identifiers_with_dollars_are_not_quotes() {
        assert_eq!(
            statements("select $1, price$usd; select 2"),
            vec!["select $1, price$usd", "select 2"]
        );
    }

    #[test]
    fn unterminated_literals_run_to_the_end() {
        assert_eq!(
            statements("select 'open; select 2"),
            vec!["select 'open; select 2"]
        );
        assert_eq!(
            statements("select $$ open; select 2"),
            vec!["select $$ open; select 2"]
        );
        assert_eq!(
            statements("select /* open; select 2"),
            vec!["select /* open; select 2"]
        );
    }

    #[test]
    fn keeps_multibyte_text_intact() {
        assert_eq!(
            statements("select 'café;' as ação; select 2"),
            vec!["select 'café;' as ação", "select 2"]
        );
    }

    #[test]
    fn picks_the_statement_under_the_cursor() {
        let text = "select 1;\n\nselect 2;\n";
        let statements = split_statements(text);
        let pick = |offset| {
            statement_at(&statements, offset).map(|statement| &text[statement.range.clone()])
        };
        assert_eq!(pick(0), Some("select 1"));
        assert_eq!(pick(4), Some("select 1"));
        // Right after the semicolon.
        assert_eq!(pick(9), Some("select 1"));
        // On the blank line between statements, the previous one.
        assert_eq!(pick(10), Some("select 1"));
        assert_eq!(pick(11), Some("select 2"));
        assert_eq!(pick(text.len()), Some("select 2"));
    }

    #[test]
    fn picks_the_first_statement_before_any_code() {
        let text = "\n\n  select 1";
        let statements = split_statements(text);
        assert_eq!(
            statement_at(&statements, 0).map(|statement| statement.range.clone()),
            Some(4..12)
        );
        assert_eq!(statement_at(&[], 0), None);
    }

    #[test]
    fn runs_the_selection_or_the_statement_under_the_cursor() {
        let text = "select 1;\nselect 2;";
        assert_eq!(statements_to_run(text, 12..12), vec!["select 2"]);
        assert_eq!(statements_to_run(text, 0..9), vec!["select 1"]);
        assert_eq!(
            statements_to_run(text, 0..text.len()),
            vec!["select 1", "select 2"]
        );
        assert!(statements_to_run("  ", 1..1).is_empty());
        assert!(statements_to_run(text, 9..10).is_empty());
    }

    #[test]
    fn selection_runs_every_statement_in_it() {
        let text = "select 1; select 2;\nselect 3;";
        assert_eq!(statements_to_run(text, 0..19), vec!["select 1", "select 2"]);
        assert_eq!(statements_to_run(text, 21..21), vec!["select 3"]);
        assert_eq!(all_statements(text).len(), 3);
    }

    #[test]
    fn from_target_reads_the_first_top_level_relation() {
        assert_eq!(
            from_target("select po.id\nfrom admin.platform_owner po"),
            Some("admin.platform_owner")
        );
        assert_eq!(from_target("SELECT * FROM users"), Some("users"));
        assert_eq!(
            from_target("select * from \"My Schema\".\"Orders\" o"),
            Some("\"My Schema\".\"Orders\"")
        );
        assert_eq!(
            from_target("select (select 1 from inner_table) from outer_table"),
            Some("outer_table")
        );
        assert_eq!(from_target("select 'from x' as label"), None);
        assert_eq!(from_target("-- from comment\nselect 1"), None);
        assert_eq!(from_target("select 1"), None);
    }
}
