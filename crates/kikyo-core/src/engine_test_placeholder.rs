#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_yab_content;
    use crate::types::Token;

    // Helper to parse config directly
    fn load_test_layout() -> Layout {
        let config = r#"
[ローマ字シフト無し]
無,無,無,無,無,無,無,k_base,無,無,無,無,無
無,無,d_base,無,無,無,無,無,無,無,無,無

<k>
無,無,d_chord,無,無,無,無,無,無,無,無,無
"#;
        // Note: The parser implementation is private "parse_yab_content" in parser.rs?
        // Let's check parser.rs visibility.
        // It says "fn parse_yab_content" is private (not pub).
        // But "load_yab" is pub.
        // We might need to expose parse_string or similar, or just make parse_yab_content pub.
        // For now, I'll assume I need to make it pub or use a temporary file.
        // Actually, let's just make parse_yab_content pub in parser.rs first.
        Layout::default()
    }
}
