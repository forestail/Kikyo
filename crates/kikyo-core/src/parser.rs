use crate::types::{Layout, Plane, Rc, Section, Token};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, warn};

pub fn load_yab<P: AsRef<Path>>(path: P) -> Result<Layout> {
    let raw = std::fs::read(path)?;
    // Detect encoding. Simple check for BOM.
    let (cow, encoding_used, had_errors) = encoding_rs::UTF_16LE.decode(&raw);
    if had_errors {
        warn!("UTF-16 decode had errors (replacement characters used)");
    }
    debug!("Decoded using: {}", encoding_used.name());

    let text = cow.as_ref();
    parse_yab_content(text)
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

    let mut iter = content.lines();

    // Check first line for layout name (starts with ;)
    if let Some(first_line) = iter.next() {
        let first_line_trim = first_line.trim();
        if first_line_trim.starts_with(';') {
            // remove leading ';'
            let name = first_line_trim.trim_start_matches(';').trim().to_string();
            if !name.is_empty() {
                layout.name = Some(name);
            }
        } else {
            // If not a comment line, treat it as normal content?
            // Actually, usually headers are optional. If it's not a name, we should process it.
            // But for simplicity in this iterator approach, let's just re-process it if not handled?
            // Or better: Re-create iterator or handle the first line specially.
            // Given the requirements: "{読み込んでいる配列定義の名称}は読み込んだyabファイルの先頭行を表示する。ただし１文字目は";"のため非表示にする。"
            // It implies the name IS on the first line.
            // However, we should be robust.

            // To properly handle the "peek" or "process", let's rewrite the loop slightly.
            // But actually, `iter` is stateful.

            // If the first line was NOT a name comment, we need to process it as normal line.
            // Just duplicate the line processing logic or use a helper is tedious.
            // Let's iterate `content.lines()` again but skip 0 if we consumed it as name?
            // No, `content.lines()` creates a new iterator.
        }
    }

    // Re-iterate from start for full parsing, but this time we already have the name if found.
    // Wait, simpler approach:

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if i == 0 && line.starts_with(';') {
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

    // Check quotes
    if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
        return Token::ImeChar(raw[1..raw.len() - 1].to_string());
    }
    if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
        return Token::DirectChar(raw[1..raw.len() - 1].to_string());
    }

    // Convert full-width to half-width and handle special tokens
    let mut seq = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            // Full-width a-z (U+FF41 to U+FF5A)
            '\u{FF41}'..='\u{FF5A}' => {
                let half = std::char::from_u32(c as u32 - 0xFEE0).unwrap();
                seq.push(half);
            }
            // Full-width A-Z (U+FF21 to U+FF3A)
            '\u{FF21}'..='\u{FF3A}' => {
                let half = std::char::from_u32(c as u32 - 0xFEE0).unwrap();
                seq.push(half);
            }
            // Special tokens handling
            // Since we iterate chars, we need to handle multi-char strings like "無" (handled above),
            // but for "後", "左" etc which might be mixed?
            // The request implies these are standalone or part of a sequence?
            // "aキーに"n""o"のシーケンスを割り当てるのであれば、"ｎｏ"のように記述される" -> Sequence of chars.
            // "後" etc are likely single tokens standing for a key press.
            // If "後" is mixed like "a後", it's probably "a" + "BackSpace".
            // Let's assume we replace them in the sequence.
            '後' => seq.push('\u{0008}'), // BS
            '入' => seq.push('\u{000D}'), // CR (Enter)
            '左' => seq.push('\u{F702}'), // Left Arrow (PUA)
            '右' => seq.push('\u{F703}'), // Right Arrow (PUA)

            // Punctuation
            '，' => seq.push(','),
            '．' => seq.push('.'),

            // Pass through others (number, half-width etc)
            _ => seq.push(c),
        }
    }

    Token::KeySequence(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_token() {
        assert_eq!(parse_token("ni"), Token::KeySequence("ni".into()));
        assert_eq!(parse_token("'あ'"), Token::ImeChar("あ".into()));
        assert_eq!(parse_token("\"A\""), Token::DirectChar("A".into()));
        assert_eq!(parse_token("無"), Token::None);
        assert_eq!(parse_token(""), Token::None);

        // Full-width conversion
        assert_eq!(parse_token("ｎｏ"), Token::KeySequence("no".into()));
        assert_eq!(parse_token("ａｂｃ"), Token::KeySequence("abc".into()));

        // Special tokens
        assert_eq!(parse_token("後"), Token::KeySequence("\u{0008}".into()));
        assert_eq!(parse_token("入"), Token::KeySequence("\u{000D}".into()));
        assert_eq!(parse_token("左"), Token::KeySequence("\u{F702}".into()));
        assert_eq!(parse_token("右"), Token::KeySequence("\u{F703}".into()));

        // Mixed
        assert_eq!(parse_token("a後b"), Token::KeySequence("a\u{0008}b".into()));

        // Uppercase conversion
        assert_eq!(parse_token("ＡＢＣ"), Token::KeySequence("ABC".into()));

        // Punctuation
        assert_eq!(parse_token("，．"), Token::KeySequence(",.".into()));
    }
    #[test]
    fn test_parse_layout_name() {
        let content_with_name = "; 新下駄配列
[Main]
a,b
";
        let layout = parse_yab_content(content_with_name).expect("Failed to parse");
        assert_eq!(layout.name, Some("新下駄配列".to_string()));

        let content_without_name = "
[Main]
a,b
";
        // Note: Our current logic checks the VERY FIRST line of the string.
        // In the string literal above, the first line is empty (newline).
        // Let's adjust the test string to be precise.
        let layout_no_name = parse_yab_content(content_without_name.trim_start()).expect("Failed");
        // If it starts with [Main], it's not a comment, so name should be None.
        assert_eq!(layout_no_name.name, None);

        let content_name_variation = ";My Layout  ";
        let layout_var = parse_yab_content(content_name_variation).expect("Failed");
        assert_eq!(layout_var.name, Some("My Layout".to_string()));
    }
}
