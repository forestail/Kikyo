use crate::types::{Rc, ScKey};

/// Maps Scancode to (Row, Col) for standard JIS layout.
/// Based on the request specification.
pub const JIS_SC_TO_RC: &[(ScKey, Rc)] = &[
    // Row 0: Number row (13 keys)
    (ScKey::new(0x02, false), Rc::new(0, 0)),  // 1
    (ScKey::new(0x03, false), Rc::new(0, 1)),  // 2
    (ScKey::new(0x04, false), Rc::new(0, 2)),  // 3
    (ScKey::new(0x05, false), Rc::new(0, 3)),  // 4
    (ScKey::new(0x06, false), Rc::new(0, 4)),  // 5
    (ScKey::new(0x07, false), Rc::new(0, 5)),  // 6
    (ScKey::new(0x08, false), Rc::new(0, 6)),  // 7
    (ScKey::new(0x09, false), Rc::new(0, 7)),  // 8
    (ScKey::new(0x0A, false), Rc::new(0, 8)),  // 9
    (ScKey::new(0x0B, false), Rc::new(0, 9)),  // 0
    (ScKey::new(0x0C, false), Rc::new(0, 10)), // -
    (ScKey::new(0x0D, false), Rc::new(0, 11)), // ^
    (ScKey::new(0x7D, false), Rc::new(0, 12)), // ¥ (Yen)
    // Row 1: QWERTY row (12 keys)
    (ScKey::new(0x10, false), Rc::new(1, 0)),  // Q
    (ScKey::new(0x11, false), Rc::new(1, 1)),  // W
    (ScKey::new(0x12, false), Rc::new(1, 2)),  // E
    (ScKey::new(0x13, false), Rc::new(1, 3)),  // R
    (ScKey::new(0x14, false), Rc::new(1, 4)),  // T
    (ScKey::new(0x15, false), Rc::new(1, 5)),  // Y
    (ScKey::new(0x16, false), Rc::new(1, 6)),  // U
    (ScKey::new(0x17, false), Rc::new(1, 7)),  // I
    (ScKey::new(0x18, false), Rc::new(1, 8)),  // O
    (ScKey::new(0x19, false), Rc::new(1, 9)),  // P
    (ScKey::new(0x1A, false), Rc::new(1, 10)), // @ / [ (JIS @ is at 0x1A)
    // Note: JIS '@' is key 0x1A. US '[' is 0x1A.
    // The request said "[=0x1A (1,10)".
    // Wait, JIS '@' is usually where US '[' is (next to P).
    // Let's assume 0x1A is the key right of P for now.
    (ScKey::new(0x1B, false), Rc::new(1, 11)), // [ / ] (JIS [ is at 0x1B)
    // Row 2: ASDF row (12 keys)
    (ScKey::new(0x1E, false), Rc::new(2, 0)),  // A
    (ScKey::new(0x1F, false), Rc::new(2, 1)),  // S
    (ScKey::new(0x20, false), Rc::new(2, 2)),  // D
    (ScKey::new(0x21, false), Rc::new(2, 3)),  // F
    (ScKey::new(0x22, false), Rc::new(2, 4)),  // G
    (ScKey::new(0x23, false), Rc::new(2, 5)),  // H
    (ScKey::new(0x24, false), Rc::new(2, 6)),  // J
    (ScKey::new(0x25, false), Rc::new(2, 7)),  // K
    (ScKey::new(0x26, false), Rc::new(2, 8)),  // L
    (ScKey::new(0x27, false), Rc::new(2, 9)),  // ; / +
    (ScKey::new(0x28, false), Rc::new(2, 10)), // : / *
    (ScKey::new(0x2B, false), Rc::new(2, 11)), // ] / } (JIS ] is here)
    // Row 3: ZXCV row (11 keys)
    (ScKey::new(0x2C, false), Rc::new(3, 0)),  // Z
    (ScKey::new(0x2D, false), Rc::new(3, 1)),  // X
    (ScKey::new(0x2E, false), Rc::new(3, 2)),  // C
    (ScKey::new(0x2F, false), Rc::new(3, 3)),  // V
    (ScKey::new(0x30, false), Rc::new(3, 4)),  // B
    (ScKey::new(0x31, false), Rc::new(3, 5)),  // N
    (ScKey::new(0x32, false), Rc::new(3, 6)),  // M
    (ScKey::new(0x33, false), Rc::new(3, 7)),  // , / <
    (ScKey::new(0x34, false), Rc::new(3, 8)),  // . / >
    (ScKey::new(0x35, false), Rc::new(3, 9)),  // / / ?
    (ScKey::new(0x73, false), Rc::new(3, 10)), // \ / _ (JIS Backslash/Ro, usually next to right shift)
];

pub fn sc_to_key_name(sc: u16) -> Option<&'static str> {
    match sc {
        0x02 => Some("1"),
        0x03 => Some("2"),
        0x04 => Some("3"),
        0x05 => Some("4"),
        0x06 => Some("5"),
        0x07 => Some("6"),
        0x08 => Some("7"),
        0x09 => Some("8"),
        0x0A => Some("9"),
        0x0B => Some("0"),
        0x0C => Some("-"),
        0x0D => Some("^"),
        0x7D => Some("\\"), // Yen

        0x10 => Some("q"),
        0x11 => Some("w"),
        0x12 => Some("e"),
        0x13 => Some("r"),
        0x14 => Some("t"),
        0x15 => Some("y"),
        0x16 => Some("u"),
        0x17 => Some("i"),
        0x18 => Some("o"),
        0x19 => Some("p"),
        0x1A => Some("@"),
        0x1B => Some("["),

        0x1E => Some("a"),
        0x1F => Some("s"),
        0x20 => Some("d"),
        0x21 => Some("f"),
        0x22 => Some("g"),
        0x23 => Some("h"),
        0x24 => Some("j"),
        0x25 => Some("k"),
        0x26 => Some("l"),
        0x27 => Some(";"),
        0x28 => Some(":"),
        0x2B => Some("]"),

        0x2C => Some("z"),
        0x2D => Some("x"),
        0x2E => Some("c"),
        0x2F => Some("v"),
        0x30 => Some("b"),
        0x31 => Some("n"),
        0x32 => Some("m"),
        0x33 => Some(","),
        0x34 => Some("."),
        0x35 => Some("/"),
        0x73 => Some("_"), // Backslash/Ro

        0x39 => Some("space"),
        0x79 => Some("henkan"),
        0x7B => Some("muhenkan"),
        _ => None,
    }
}
pub fn key_name_to_sc(name: &str) -> Option<u16> {
    // Brute-force reverse search for MVP (map is small)
    for sc in 0..256 {
        if let Some(n) = sc_to_key_name(sc as u16) {
            if n == name {
                return Some(sc as u16);
            }
        }
    }
    None
}
