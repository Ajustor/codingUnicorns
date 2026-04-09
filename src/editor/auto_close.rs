/// Returns the closing character for an opening bracket/quote, if any.
pub fn closing_pair(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Returns true if the character is a closing bracket/quote.
pub fn is_closing(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// Returns true if we should auto-close given the character after the cursor.
pub fn should_auto_close(_ch: char, next_char: Option<char>) -> bool {
    match next_char {
        None => true,
        Some(c) if c.is_whitespace() => true,
        Some(c) if is_closing(c) => true,
        _ => false,
    }
}

/// For quote characters, check if the previous character suggests we should NOT auto-close.
pub fn should_skip_quote_auto_close(ch: char, prev_char: Option<char>) -> bool {
    if ch == '\'' || ch == '"' || ch == '`' {
        if let Some(prev) = prev_char {
            return prev.is_alphanumeric();
        }
    }
    false
}
