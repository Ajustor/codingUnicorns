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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_pair_maps_brackets_and_quotes() {
        assert_eq!(closing_pair('('), Some(')'));
        assert_eq!(closing_pair('['), Some(']'));
        assert_eq!(closing_pair('{'), Some('}'));
        assert_eq!(closing_pair('"'), Some('"'));
        assert_eq!(closing_pair('\''), Some('\''));
        assert_eq!(closing_pair('`'), Some('`'));
        assert_eq!(closing_pair('a'), None);
        assert_eq!(closing_pair(')'), None);
        assert_eq!(closing_pair('<'), None);
    }

    #[test]
    fn is_closing_recognises_closers_only() {
        for ch in [')', ']', '}', '"', '\'', '`'] {
            assert!(is_closing(ch), "{ch} should be a closer");
        }
        for ch in ['(', '[', '{', 'a', ' ', '>'] {
            assert!(!is_closing(ch), "{ch} should not be a closer");
        }
    }

    #[test]
    fn should_auto_close_depends_on_next_char() {
        assert!(should_auto_close('(', None));
        assert!(should_auto_close('(', Some(' ')));
        assert!(should_auto_close('(', Some('\t')));
        assert!(should_auto_close('(', Some(')')));
        assert!(should_auto_close('"', Some('}')));
        assert!(!should_auto_close('(', Some('a')));
        assert!(!should_auto_close('(', Some('(')));
        assert!(!should_auto_close('[', Some('1')));
    }

    #[test]
    fn quotes_after_word_chars_are_not_auto_closed() {
        assert!(should_skip_quote_auto_close('\'', Some('t'))); // don't
        assert!(should_skip_quote_auto_close('"', Some('9')));
        assert!(should_skip_quote_auto_close('`', Some('x')));
        assert!(!should_skip_quote_auto_close('\'', Some(' ')));
        assert!(!should_skip_quote_auto_close('"', Some('(')));
        assert!(!should_skip_quote_auto_close('"', None));
        // Non-quote characters are never skipped.
        assert!(!should_skip_quote_auto_close('(', Some('a')));
    }
}
