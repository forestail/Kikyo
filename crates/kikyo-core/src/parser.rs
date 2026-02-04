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

        let tokens: Vec<String> = line.split(',').map(|s| s.trim().to_string()).collect();
        current_rows.push(tokens);
    }

    // Flush final
    if let Some(name) = current_section_name {
        flush_plane(&mut current_section, current_plane_tag, &current_rows);
        current_section.name = name.clone();
        layout.sections.insert(name, current_section);
    }

    Ok(layout)
}

fn parse_token(raw: &str) -> Token {
    if raw.is_empty() || raw == "無" {
        return Token::None;
    }

    if let Some((quote, inner)) = strip_quotes(raw) {
        let parsed = parse_quoted(inner, quote);
        return if quote == '\'' {
            Token::ImeChar(parsed)
        } else {
            Token::DirectChar(parsed)
        };
    }

    if let Some(token) = parse_modded_single_token(raw) {
        return token;
    }

    let seq = parse_key_sequence(raw);
    if seq.is_empty() {
        Token::None
    } else {
        Token::KeySequence(seq)
    }
}

fn strip_quotes(raw: &str) -> Option<(char, &str)> {
    let mut chars = raw.chars();
    let first = chars.next()?;
    let last = raw.chars().last()?;
    if (first == '\'' || first == '"') && first == last && raw.len() >= 2 {
        Some((first, &raw[first.len_utf8()..raw.len() - last.len_utf8()]))
    } else {
        None
    }
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

fn parse_modded_single_token(raw: &str) -> Option<Token> {
    let (mods, rest) = parse_modifiers(raw);
    if mods.is_empty() || rest.is_empty() {
        return None;
    }

    if let Some(key) = parse_key_spec(rest) {
        return Some(Token::KeySequence(vec![KeyStroke { key, mods }]));
    }

    None
}

fn parse_modifiers(raw: &str) -> (Modifiers, &str) {
    let mut mods = Modifiers::none();
    let mut idx = 0;
    let mut iter = raw.char_indices().peekable();
    while let Some((offset, c)) = iter.next() {
        let is_mod = matches!(c, 'C' | 'S' | 'A' | 'W');
        if !is_mod {
            break;
        }
        if iter.peek().is_none() {
            break;
        }
        match c {
            'C' => mods.ctrl = true,
            'S' => mods.shift = true,
            'A' => mods.alt = true,
            'W' => mods.win = true,
            _ => {}
        }
        idx = offset + c.len_utf8();
    }
    (mods, &raw[idx..])
}

fn parse_key_spec(raw: &str) -> Option<KeySpec> {
    if raw.is_empty() {
        return None;
    }

    if let Some(rest) = raw.strip_prefix('機') {
        let num: u8 = rest.parse().ok()?;
        let sc = function_key_scancode(num)?;
        return Some(KeySpec::Scancode(sc, false));
    }

    if let Some(rest) = raw.strip_prefix('V') {
        if rest.is_empty() {
            return None;
        }
        let value = u32::from_str_radix(rest, 16).ok()?;
        if value <= 0xFF {
            return Some(KeySpec::VirtualKey(value as u16));
        }
        return None;
    }

    let mut chars = raw.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    Some(parse_single_key_char(c))
}

fn parse_key_sequence(raw: &str) -> Vec<KeyStroke> {
    let mut seq = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(stroke) = fullwidth_shifted_keystroke(c) {
            seq.push(stroke);
            i += 1;
            continue;
        }
        if c == '機' {
            let mut j = i + 1;
            let mut digits = String::new();
            while j < chars.len() && chars[j].is_ascii_digit() {
                digits.push(chars[j]);
                j += 1;
            }
            if let Ok(num) = digits.parse::<u8>() {
                if let Some(sc) = function_key_scancode(num) {
                    seq.push(KeyStroke {
                        key: KeySpec::Scancode(sc, false),
                        mods: Modifiers::none(),
                    });
                    i = j;
                    continue;
                }
            }
        } else if c == 'V' {
            let mut j = i + 1;
            let mut digits = String::new();
            while j < chars.len() && chars[j].is_ascii_hexdigit() {
                digits.push(chars[j]);
                j += 1;
            }
            if !digits.is_empty() {
                if let Ok(value) = u32::from_str_radix(&digits, 16) {
                    if value <= 0xFF {
                        seq.push(KeyStroke {
                            key: KeySpec::VirtualKey(value as u16),
                            mods: Modifiers::none(),
                        });
                        i = j;
                        continue;
                    }
                }
            }
        }

        seq.push(KeyStroke {
            key: parse_single_key_char(c),
            mods: Modifiers::none(),
        });
        i += 1;
    }
    seq
}

fn parse_single_key_char(c: char) -> KeySpec {
    if let Some((sc, ext)) = special_key_scancode(c) {
        return KeySpec::Scancode(sc, ext);
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
        '\u{FF41}'..='\u{FF5A}' | '\u{FF21}'..='\u{FF3A}' => {
            let half = std::char::from_u32(c as u32 - 0xFEE0).unwrap_or(c);
            half.to_ascii_lowercase()
        }
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
        '逃' => Some((0x01, false)), // Esc
        '入' => Some((0x1C, false)), // Enter
        '空' => Some((0x39, false)), // Space
        '後' => Some((0x0E, false)), // Backspace
        '消' => Some((0x53, true)),  // Delete
        '挿' => Some((0x52, true)),  // Insert
        '上' => Some((0x48, true)),  // Up
        '左' => Some((0x4B, true)),  // Left
        '右' => Some((0x4D, true)),  // Right
        '下' => Some((0x50, true)),  // Down
        '家' => Some((0x47, true)),  // Home
        '終' => Some((0x4F, true)),  // End
        '前' => Some((0x49, true)),  // Page Up
        '次' => Some((0x51, true)),  // Page Down
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

    fn stroke_shift_char(c: char) -> KeyStroke {
        let mut mods = Modifiers::none();
        mods.shift = true;
        KeyStroke {
            key: KeySpec::Char(c),
            mods,
        }
    }

    #[test]
    fn test_parse_token() {
        assert_eq!(
            parse_token("ni"),
            Token::KeySequence(vec![stroke_char('n'), stroke_char('i')])
        );
        assert_eq!(parse_token("'あ'"), Token::ImeChar("あ".into()));
        assert_eq!(parse_token("\"A\""), Token::DirectChar("A".into()));
        assert_eq!(parse_token("無"), Token::None);
        assert_eq!(parse_token(""), Token::None);
        assert_eq!(parse_token("'a\\n'"), Token::ImeChar("a\n".into()));
        assert_eq!(parse_token("\"\\u0041\""), Token::DirectChar("A".into()));

        // Full-width conversion
        assert_eq!(
            parse_token("ｎｏ"),
            Token::KeySequence(vec![stroke_char('n'), stroke_char('o')])
        );
        assert_eq!(
            parse_token("ａｂｃ"),
            Token::KeySequence(vec![stroke_char('a'), stroke_char('b'), stroke_char('c')])
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
            Token::KeySequence(vec![stroke_shift_char('8'), stroke_shift_char('9')])
        );
        assert_eq!(
            parse_token("＋＊"),
            Token::KeySequence(vec![stroke_shift_char(';'), stroke_shift_char(':'),])
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
        assert_eq!(
            parse_token("CA"),
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
