use crate::types::{KeySpec, KeyStroke, Layout, Modifiers, Plane, Rc, Section, Token};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, warn};

pub fn load_yab<P: AsRef<Path>>(path: P) -> Result<Layout> {
    let raw = std::fs::read(path)?;
    let text = decode_yab_bytes(&raw);
    parse_yab_content(text.as_ref())
}

fn decode_yab_bytes<'a>(raw: &'a [u8]) -> std::borrow::Cow<'a, str> {
    // 1. Check BOM
    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(raw) {
        debug!("Decoded using BOM: {}", enc.name());
        let (cow, _, had_errors) = enc.decode(&raw[bom_len..]);
        if had_errors {
            warn!("Decode had errors (replacement characters used)");
        }
        return cow;
    }

    // 2. Try UTF-8
    match std::str::from_utf8(raw) {
        Ok(s) => {
            debug!("Decoded as UTF-8");
            std::borrow::Cow::Borrowed(s)
        }
        Err(_) => {
            // 3. Fallback to Shift_JIS
            debug!("UTF-8 decode failed, falling back to Shift_JIS");
            let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(raw);
            if had_errors {
                warn!("Shift_JIS decode had errors");
            }
            cow
        }
    }
}

pub fn parse_yab_content(content: &str) -> Result<Layout> {
    let mut layout = Layout::default();

    let mut current_section_name: Option<String> = None;
    let mut current_section = Section::default();

    // State within a section
    let mut current_plane_tag: Option<String> = None; // None means base plane
    let mut current_rows: Vec<Vec<String>> = Vec::new();

    // Helper to flush current plane
    let flush_plane = |sec: &mut Section, tag: Option<String>, rows: &[Vec<String>]| {
        if rows.is_empty() {
            return;
        }

        // Build map
        let mut map = HashMap::new();
        for (r_idx, row_tokens) in rows.iter().enumerate() {
            if r_idx > 255 {
                continue;
            }
            for (c_idx, token_str) in row_tokens.iter().enumerate() {
                if c_idx > 255 {
                    continue;
                }
                let token = parse_token(token_str);
                if token != Token::None {
                    map.insert(Rc::new(r_idx as u8, c_idx as u8), token);
                }
            }
        }
        let plane = Plane { map };

        if let Some(t) = tag {
            sec.sub_planes.insert(t, plane);
        } else {
            sec.base_plane = plane;
        }
    };

    for line in content.lines() {
        let line = line.trim();
        if layout.name.is_none() && line.starts_with(';') {
            let name = line.trim_start_matches(';').trim().to_string();
            if !name.is_empty() {
                layout.name = Some(name);
            }
            continue;
        }

        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            // New Section
            // Flush previous section
            if let Some(name) = current_section_name.take() {
                // Flush last plane of previous section
                flush_plane(
                    &mut current_section,
                    current_plane_tag.take(),
                    &current_rows,
                );
                current_rows.clear();

                current_section.name = name.clone();
                layout.sections.insert(name, current_section);
                current_section = Section::default();
            }

            // Start new
            let name = line[1..line.len() - 1].to_string();
            current_section_name = Some(name);
            current_plane_tag = None; // Reset to base plane
            continue;
        }

        if line.starts_with('<') && line.ends_with('>') {
            // New Plane within current section
            // Flush previous plane
            if current_section_name.is_some() {
                flush_plane(
                    &mut current_section,
                    current_plane_tag.take(),
                    &current_rows,
                );
                current_rows.clear();

                let tag = line.to_string(); // Keep the brackets, e.g. "<k>"
                current_plane_tag = Some(tag);
            }
            continue;
        }

        if current_section_name
            .as_deref()
            .is_some_and(is_function_key_section_name)
        {
            if let Some((left, right)) = parse_function_key_swap_line(line) {
                layout.function_key_swaps.push((left, right));
            }
            continue;
        }

        let tokens: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
        current_rows.push(tokens);
    }

    // Flush final
    if let Some(name) = current_section_name {
        flush_plane(&mut current_section, current_plane_tag, &current_rows);
        current_section.name = name.clone();
        layout.sections.insert(name, current_section);
    }

    layout.max_chord_size = detect_max_chord_size(&layout);

    Ok(layout)
}

fn detect_max_chord_size(layout: &Layout) -> usize {
    for (section_name, section) in &layout.sections {
        if count_valid_chord_keys(section_name) >= 2 {
            return 3;
        }
        if section
            .sub_planes
            .keys()
            .any(|tag| count_valid_chord_keys(tag) >= 2)
        {
            return 3;
        }
    }
    2
}

fn count_valid_chord_keys(tag: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(open_rel) = tag[start..].find('<') {
        let open = start + open_rel;
        let Some(close_rel) = tag[open..].find('>') else {
            break;
        };
        let close = open + close_rel;
        if close > open + 1 {
            let key_name = &tag[open + 1..close];
            if crate::jis_map::key_name_to_sc(key_name).is_some() {
                count += 1;
            }
        }
        start = close + 1;
    }
    count
}

fn parse_token(raw: &str) -> Token {
    if raw.is_empty() || raw == "無" || raw.eq_ignore_ascii_case("xx") {
        return Token::None;
    }

    // If double-quoted, it was returned as DirectString.
    // If single-quoted, it was returned as ImeChar (currently treated as expanded sequence).

    // We want to support mixed sequences like "【】"Left.
    // So we need to parse the string into tokens.
    // But current logic parses the whole string at once.
    // Examples: "ni", "'ni'", "\"ni\"", "SLeft", "\"【】\"Left"

    // If it starts with quote, we can try to parse a quoted string first.
    // But bare keys can also be mixed? e.g. Left"【】" -> Left, DirectString.

    // Let's implement a simple loop to consume valid tokens.
    let mut seq = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // 1. Check for quoted string
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            // Find closing quote
            let mut j = i + 1;
            let mut escaped = false;
            let mut close_found = false;
            while j < chars.len() {
                if escaped {
                    escaped = false;
                } else if chars[j] == '\\' {
                    escaped = true;
                } else if chars[j] == quote {
                    close_found = true;
                    break;
                }
                j += 1;
            }

            if close_found {
                let inner: String = chars[i + 1..j].iter().collect();
                let unquoted = parse_quoted(&inner, quote);
                if quote == '"' {
                    seq.push(KeyStroke {
                        key: KeySpec::DirectString(unquoted),
                        mods: Modifiers::none(),
                    });
                } else {
                    // Single quote -> Expand as before
                    let sub_seq = parse_key_sequence_expanded(&unquoted);
                    seq.extend(sub_seq);
                }
                i = j + 1;
                continue;
            } else {
                // Mismatched quote -> fallback to old logic or error?
                // Old logic: treat as part of sequence if not valid quote block?
                // For now, let's treat it as part of bare sequence if not closed.
            }
        }

        // 2. Parse as bare key sequence until next quote or end
        // But wait, "SLeft" is parsed by parse_key_sequence_expanded which handles modifiers like 'S'.
        // If we have "S\"a\"", 'S' is Modifier, '"a"' is DirectString.
        // Can we apply Shift to DirectString? No.
        // So Modifiers should only apply to the next KEY.

        // Let's grab a chunk of chars until a quote is seen.
        let mut j = i;
        while j < chars.len() && chars[j] != '"' && chars[j] != '\'' {
            j += 1;
        }

        if j > i {
            let chunk: String = chars[i..j].iter().collect();
            // This chunk might contain modifiers and keys.
            let sub_seq = parse_key_sequence_expanded(&chunk);
            seq.extend(sub_seq);
            i = j;
        } else {
            // j == i. This happens if chars[i] is quote (and not handled by 1).
            // This implies an unclosed quote or a logic error.
            // To prevent infinite loop, we MUST advance i.
            // We can treat the quote as a literal char or just skip it.
            // Let's treat it as a part of the sequence (literal quote).
            let chunk: String = chars[i..i + 1].iter().collect();
            let sub_seq = parse_key_sequence_expanded(&chunk);
            seq.extend(sub_seq);
            i += 1;
        }
    }

    if seq.is_empty() {
        Token::None
    } else {
        Token::KeySequence(seq)
    }
}

pub fn parse_key_sequence_expanded(raw: &str) -> Vec<KeyStroke> {
    let mut seq = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;

    let mut current_mods = Modifiers::none();

    while i < chars.len() {
        let c = chars[i];

        // Check for modifiers
        match c {
            'S' => {
                current_mods.shift = true;
                i += 1;
                continue;
            }
            'C' => {
                current_mods.ctrl = true;
                i += 1;
                continue;
            }
            'A' => {
                current_mods.alt = true;
                i += 1;
                continue;
            }
            'W' => {
                current_mods.win = true;
                i += 1;
                continue;
            }
            _ => {}
        }

        let (mut strokes, consumed) = parse_unit(&chars[i..]);

        // Apply accumulated modifiers to the first stroke of the sequence
        if let Some(first) = strokes.first_mut() {
            first.mods.ctrl |= current_mods.ctrl;
            first.mods.shift |= current_mods.shift;
            first.mods.alt |= current_mods.alt;
            first.mods.win |= current_mods.win;
        }

        // Reset modifiers after applying (or discarding if no strokes)
        current_mods = Modifiers::none();

        seq.extend(strokes);
        i += consumed;
    }
    seq
}

fn parse_unit(chars: &[char]) -> (Vec<KeyStroke>, usize) {
    if chars.is_empty() {
        return (Vec::new(), 0);
    }
    let c = chars[0];

    // 1. Try Kana -> Romaji
    if let Some(romaji) = crate::romaji_map::kana_to_romaji(c) {
        let mut seq = Vec::new();
        for r in romaji.chars() {
            seq.push(KeyStroke {
                key: KeySpec::Char(r),
                mods: Modifiers::none(),
            });
        }
        return (seq, 1);
    }

    // 2. Try normalized symbol
    if let Some(norm) = crate::romaji_map::normalize_symbol(c) {
        return (
            vec![KeyStroke {
                key: KeySpec::Char(norm),
                mods: Modifiers::none(),
            }],
            1,
        );
    }

    // 3. Fallback to existing logic
    if let Some(stroke) = fullwidth_shifted_keystroke(c) {
        return (vec![stroke], 1);
    }

    if c == '機' {
        let mut j = 1;
        let mut digits = String::new();
        while j < chars.len() && chars[j].is_ascii_digit() {
            digits.push(chars[j]);
            j += 1;
        }
        if let Ok(num) = digits.parse::<u8>() {
            if let Some(sc) = function_key_scancode(num) {
                return (
                    vec![KeyStroke {
                        key: KeySpec::Scancode(sc, false),
                        mods: Modifiers::none(),
                    }],
                    j,
                );
            }
        }
        return (Vec::new(), j);
    } else if c == 'V' {
        let mut j = 1;
        let mut digits = String::new();
        while j < chars.len() && chars[j].is_ascii_hexdigit() {
            digits.push(chars[j]);
            j += 1;
        }
        if !digits.is_empty() {
            if let Ok(value) = u32::from_str_radix(&digits, 16) {
                if value <= 0xFF {
                    return (
                        vec![KeyStroke {
                            key: KeySpec::VirtualKey(value as u16),
                            mods: Modifiers::none(),
                        }],
                        j,
                    );
                }
            }
        }
        return (Vec::new(), j);
    }

    // Default single char
    (
        vec![KeyStroke {
            key: parse_single_key_char(c),
            mods: Modifiers::none(),
        }],
        1,
    )
}

fn parse_quoted(raw: &str, quote: char) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }

        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::new();
                while let Some(next) = chars.clone().next() {
                    if next.is_ascii_hexdigit() {
                        hex.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if hex.is_empty() {
                    out.push('u');
                } else if let Ok(value) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(value) {
                        out.push(ch);
                    }
                }
            }
            Some('\'') if quote == '\'' => out.push('\''),
            Some('"') if quote == '"' => out.push('"'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn is_function_key_section_name(name: &str) -> bool {
    compact_function_key_name(name) == "機能キー"
}

fn compact_function_key_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{3000}')
        .collect()
}

fn parse_function_key_swap_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split(',');
    let left = compact_function_key_name(parts.next()?);
    let right = compact_function_key_name(parts.next()?);
    if parts.next().is_some() || left.is_empty() || right.is_empty() {
        return None;
    }
    Some((left, right))
}

fn parse_single_key_char(c: char) -> KeySpec {
    if let Some((sc, ext)) = special_key_scancode(c) {
        return KeySpec::Scancode(sc, ext);
    }
    match c {
        '日' => return KeySpec::ImeOn,
        '英' => return KeySpec::ImeOff,
        _ => {}
    }
    KeySpec::Char(normalize_key_char(c))
}

fn fullwidth_shifted_keystroke(c: char) -> Option<KeyStroke> {
    let key_char = match c {
        '（' => '8',
        '）' => '9',
        '＋' => ';',
        '＊' => ':',
        _ => return None,
    };
    let mut mods = Modifiers::none();
    mods.shift = true;
    Some(KeyStroke {
        key: KeySpec::Char(key_char),
        mods,
    })
}

fn normalize_key_char(c: char) -> char {
    match c {
        '\u{FF41}'..='\u{FF5A}' => {
            std::char::from_u32(c as u32 - 0xFEE0).unwrap_or(c) // Lowercase fullwidth -> Lowercase halfwidth
        }
        '\u{FF21}'..='\u{FF3A}' => {
            std::char::from_u32(c as u32 - 0xFEE0).unwrap_or(c) // Uppercase fullwidth -> Uppercase halfwidth
        }
        'A'..='Z' => c, // Preserver uppercase
        '\u{FF10}'..='\u{FF19}' => std::char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '，' => ',',
        '．' => '.',
        '／' => '/',
        '：' => ':',
        '；' => ';',
        '＠' => '@',
        '［' => '[',
        '］' => ']',
        '＼' | '￥' => '\\',
        '＾' => '^',
        '－' => '-',
        '＋' => '+',
        '＊' => '*',
        '（' => '(',
        '）' => ')',
        '　' => ' ',
        _ => c.to_ascii_lowercase(),
    }
}

fn special_key_scancode(c: char) -> Option<(u16, bool)> {
    match c {
        '\n' | '\r' => Some((0x1C, false)), // Enter
        '逃' => Some((0x01, false)),        // Esc
        '入' => Some((0x1C, false)),        // Enter
        '空' => Some((0x39, false)),        // Space
        '後' => Some((0x0E, false)),        // Backspace
        '消' => Some((0x53, true)),         // Delete
        '挿' => Some((0x52, true)),         // Insert
        '上' => Some((0x48, true)),         // Up
        '左' => Some((0x4B, true)),         // Left
        '右' => Some((0x4D, true)),         // Right
        '下' => Some((0x50, true)),         // Down
        '家' => Some((0x47, true)),         // Home
        '終' => Some((0x4F, true)),         // End
        '前' => Some((0x49, true)),         // Page Up
        '次' => Some((0x51, true)),         // Page Down
        '変' => Some((0x79, false)),        // Convert
        _ => None,
    }
}

fn function_key_scancode(num: u8) -> Option<u16> {
    match num {
        1 => Some(0x3B),
        2 => Some(0x3C),
        3 => Some(0x3D),
        4 => Some(0x3E),
        5 => Some(0x3F),
        6 => Some(0x40),
        7 => Some(0x41),
        8 => Some(0x42),
        9 => Some(0x43),
        10 => Some(0x44),
        11 => Some(0x57),
        12 => Some(0x58),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke_char(c: char) -> KeyStroke {
        KeyStroke {
            key: KeySpec::Char(c),
            mods: Modifiers::none(),
        }
    }

    fn stroke_scancode(sc: u16, ext: bool) -> KeyStroke {
        KeyStroke {
            key: KeySpec::Scancode(sc, ext),
            mods: Modifiers::none(),
        }
    }

    fn stroke_vk(vk: u16) -> KeyStroke {
        KeyStroke {
            key: KeySpec::VirtualKey(vk),
            mods: Modifiers::none(),
        }
    }

    #[test]
    fn test_parse_token() {
        assert_eq!(
            parse_token("ni"),
            Token::KeySequence(vec![stroke_char('n'), stroke_char('i')])
        );
        assert_eq!(
            parse_token("'あ'"),
            Token::KeySequence(vec![stroke_char('a')])
        );

        // "です" -> DirectString (Unicode)
        assert_eq!(
            parse_token("\"です\""),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::DirectString("です".to_string()),
                mods: Modifiers::none(),
            }])
        );

        // 'です' -> Expanded to d,e,s,u
        assert_eq!(
            parse_token("'です'"),
            Token::KeySequence(vec![
                stroke_char('d'),
                stroke_char('e'),
                stroke_char('s'),
                stroke_char('u')
            ])
        );

        assert_eq!(parse_token("無"), Token::None);
        assert_eq!(parse_token(""), Token::None);

        // 'a\n' -> a, Enter (0x1C) because \n is likely normalized?
        // Wait, parse_quoted handles escapes. 'a\n' -> string "a\n".
        // Then expanded. 'a' -> 'a'. '\n' -> ...?
        // normalize_key_char handles '\u{000D}' -> Enter(0x1C).
        // Let's verify expansion logic for '\n'.
        // parse_key_sequence_expanded iterates chars.
        // '\n' is not in romaji map, not in normalize_symbol.
        // falls to fullwidth_shifted -> no.
        // falls to default single char -> parse_single_key_char('\n').
        // char_to_scancode('\n' aka CR 0x0D) -> 0x1C.
        // So it becomes Scancode(0x1C).
        assert_eq!(
            parse_token("'a\\n'"),
            Token::KeySequence(vec![stroke_char('a'), stroke_scancode(0x1C, false)])
        );
        assert_eq!(
            parse_token("\"\\u0041\""),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::DirectString("A".to_string()),
                mods: Modifiers::none(),
            }])
        );

        // Full-width conversion
        assert_eq!(
            parse_token("ｎｏ"),
            Token::KeySequence(vec![stroke_char('n'), stroke_char('o')])
        );
        assert_eq!(
            parse_token("ａｂｃ"),
            Token::KeySequence(vec![stroke_char('a'), stroke_char('b'), stroke_char('c')])
        );

        // Case sensitivity check
        // "A" is now treated as Alt modifier, so it produces no keystroke locally if not followed by a key.
        assert_eq!(parse_token("A"), Token::None);
        assert_eq!(
            parse_token("Ａ"), // Fullwidth A
            Token::KeySequence(vec![stroke_char('A')])
        );

        // Special tokens
        assert_eq!(
            parse_token("後"),
            Token::KeySequence(vec![stroke_scancode(0x0E, false)])
        );
        assert_eq!(
            parse_token("入"),
            Token::KeySequence(vec![stroke_scancode(0x1C, false)])
        );
        assert_eq!(
            parse_token("左"),
            Token::KeySequence(vec![stroke_scancode(0x4B, true)])
        );
        assert_eq!(
            parse_token("右"),
            Token::KeySequence(vec![stroke_scancode(0x4D, true)])
        );

        // Mixed
        assert_eq!(
            parse_token("a後b"),
            Token::KeySequence(vec![
                stroke_char('a'),
                stroke_scancode(0x0E, false),
                stroke_char('b')
            ])
        );

        // Punctuation
        assert_eq!(
            parse_token("，．"),
            Token::KeySequence(vec![stroke_char(','), stroke_char('.')])
        );
        assert_eq!(
            parse_token("（）"),
            Token::KeySequence(vec![stroke_char('('), stroke_char(')')])
        );
        assert_eq!(
            parse_token("＋＊"),
            Token::KeySequence(vec![stroke_char('+'), stroke_char('*')])
        );

        // Function key / VK
        assert_eq!(
            parse_token("機10"),
            Token::KeySequence(vec![stroke_scancode(0x44, false)])
        );
        assert_eq!(
            parse_token("V1B"),
            Token::KeySequence(vec![stroke_vk(0x1B)])
        );

        // Modifiers (single-stroke)

        // Modifiers (single-stroke)
        // "CA" -> Ctrl + Alt (accumulated modifiers, no key)
        assert_eq!(parse_token("CA"), Token::None);

        // "Cａ" -> Ctrl + a
        assert_eq!(
            parse_token("Cａ"),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::Char('a'),
                mods: Modifiers {
                    ctrl: true,
                    shift: false,
                    alt: false,
                    win: false,
                },
            }])
        );

        // IME Control
        assert_eq!(
            parse_token("日"),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::ImeOn,
                mods: Modifiers::none(),
            }])
        );
        assert_eq!(
            parse_token("英"),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::ImeOff,
                mods: Modifiers::none(),
            }])
        );
    }

    #[test]
    fn test_parse_token_extended() {
        // "変" -> Scancode 0x79
        assert_eq!(
            parse_token("変"),
            Token::KeySequence(vec![stroke_scancode(0x79, false)])
        );

        // "S左" -> Shift + Left
        assert_eq!(
            parse_token("S左"),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::Scancode(0x4B, true),
                mods: Modifiers {
                    shift: true,
                    ..Modifiers::none()
                }
            }])
        );

        // "S左S左" -> Shift+Left, Shift+Left
        assert_eq!(
            parse_token("S左S左"),
            Token::KeySequence(vec![
                KeyStroke {
                    key: KeySpec::Scancode(0x4B, true),
                    mods: Modifiers {
                        shift: true,
                        ..Modifiers::none()
                    }
                },
                KeyStroke {
                    key: KeySpec::Scancode(0x4B, true),
                    mods: Modifiers {
                        shift: true,
                        ..Modifiers::none()
                    }
                }
            ])
        );

        // "SCS左" -> Shift+Ctrl+Left
        assert_eq!(
            parse_token("SCS左"),
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::Scancode(0x4B, true),
                mods: Modifiers {
                    shift: true,
                    ctrl: true,
                    ..Modifiers::none()
                }
            }])
        );

        // "Sａ" -> Shift + a
        assert_eq!(
            parse_token("Sａ"), // Fullwidth a -> stroke_char('a')
            Token::KeySequence(vec![KeyStroke {
                key: KeySpec::Char('a'),
                mods: Modifiers {
                    shift: true,
                    ..Modifiers::none()
                }
            }])
        );

        // "S" -> Empty (No key following)
        assert_eq!(parse_token("S"), Token::None);
    }

    #[test]
    fn test_parse_mixed_string_and_keys() {
        // "【】"左 -> DirectString("【】") + Left
        assert_eq!(
            parse_token("\"【】\"左"),
            Token::KeySequence(vec![
                KeyStroke {
                    key: KeySpec::DirectString("【】".to_string()),
                    mods: Modifiers::none(),
                },
                KeyStroke {
                    key: KeySpec::Scancode(0x4B, true),
                    mods: Modifiers::none(),
                }
            ])
        );

        // Mixed with modifiers: S"【】" -> Shift + "【】" (Shift ignored for string?)
        // In current logic: 'S' is parsed in a bare chunk.
        // If "S" is before quote, it's parsed as bare.
        // parse_key_sequence_expanded("S") -> Empty (modifiers reset).
        // Then quote parsed.
        // So S is effectively ignored if not followed by a key in the same chunk.
        // This is acceptable or maybe we want "S" to apply to next key AFTER string?
        // E.g. "S" "text" "Left" -> Shift+Left?
        // No, current logic resets modifiers after each chunk in parse_key_sequence_expanded.
        // And my loop in parse_token processes chunks independently.
        // So "S" in one chunk does NOT affect next chunk.

        // Let's verify "Left" "Right" -> sequence
        assert_eq!(
            parse_token("左右"),
            Token::KeySequence(vec![
                KeyStroke {
                    key: KeySpec::Scancode(0x4B, true),
                    mods: Modifiers::none(),
                },
                KeyStroke {
                    key: KeySpec::Scancode(0x4D, true),
                    mods: Modifiers::none(),
                }
            ])
        );

        // "Left""Right" (quoted?) -> No, Left and Right are special keys, not strings.
        // "a" "b" -> a, b.
        assert_eq!(
            parse_token("'a''b'"),
            Token::KeySequence(vec![
                KeyStroke {
                    key: KeySpec::Char('a'),
                    mods: Modifiers::none(),
                },
                KeyStroke {
                    key: KeySpec::Char('b'),
                    mods: Modifiers::none(),
                }
            ])
        );
    }

    #[test]
    fn test_parse_layout_name() {
        // Case 1: Standard (First line)
        let content_with_name = "; 新下駄配列
[Main]
a,b
";
        let layout = parse_yab_content(content_with_name).expect("Failed to parse");
        assert_eq!(layout.name, Some("新下駄配列".to_string()));

        // Case 2: Skip empty lines and empty comments
        let content_skip = "

;
;
;   Real Name
[Main]
a,b
";
        let layout_skip = parse_yab_content(content_skip).expect("Failed to parse");
        assert_eq!(layout_skip.name, Some("Real Name".to_string()));

        // Case 3: No name found (starts with section)
        let content_no_name = "
;
[Main]
a,b
";
        let layout_no_name = parse_yab_content(content_no_name).expect("Failed");
        assert_eq!(layout_no_name.name, None);

        // Case 4: Name variation
        let content_name_variation = ";My Layout  ";
        let layout_var = parse_yab_content(content_name_variation).expect("Failed");
        assert_eq!(layout_var.name, Some("My Layout".to_string()));
    }

    #[test]
    fn test_parse_function_key_section() {
        let content = "
[機能キー]
 左Alt ,　拡張1
\tF13,\t 右Ctrl
Capsロック, 拡張2
左Ctrl, 右Ctrl, 余分

[Main]
a,b
";
        let layout = parse_yab_content(content).expect("Failed");
        assert_eq!(
            layout.function_key_swaps,
            vec![
                ("左Alt".to_string(), "拡張1".to_string()),
                ("F13".to_string(), "右Ctrl".to_string()),
                ("Capsロック".to_string(), "拡張2".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_sets_max_chord_size_to_two_without_double_modifier_tag() {
        let content = "
[ローマ字シフト無し]
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q>
xx,2,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(content).expect("Failed");
        assert_eq!(layout.max_chord_size, 2);
    }

    #[test]
    fn test_parse_sets_max_chord_size_to_three_with_double_modifier_tag() {
        let content = "
[ローマ字シフト無し]
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,3,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(content).expect("Failed");
        assert_eq!(layout.max_chord_size, 3);
    }

    #[test]
    fn test_decode_sjis() {
        // "テスト" in Shift_JIS
        let sjis_bytes = vec![0x83, 0x65, 0x83, 0x58, 0x83, 0x67];
        let decoded = decode_yab_bytes(&sjis_bytes);
        assert_eq!(decoded, "テスト");
    }

    #[test]
    fn test_decode_utf8() {
        let utf8_bytes = "テスト".as_bytes();
        let decoded = decode_yab_bytes(utf8_bytes);
        assert_eq!(decoded, "テスト");
    }
}
