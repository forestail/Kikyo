use crate::types::{Rc, ScKey};
use std::collections::HashMap;

/// Provides dynamic keyboard layout mapping (Scancode <-> Row/Col, Scancode <-> KeyName)
#[derive(Debug, Clone)]
pub struct KeyboardMap {
    pub name: String,
    pub rc_to_sc: HashMap<Rc, ScKey>,
    pub sc_to_rc: HashMap<ScKey, Rc>,
    pub sc_to_name: HashMap<u16, String>,
    pub name_to_sc: HashMap<String, u16>,
}

impl KeyboardMap {
    /// Creates a new empty KeyboardMap
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rc_to_sc: HashMap::new(),
            sc_to_rc: HashMap::new(),
            sc_to_name: HashMap::new(),
            name_to_sc: HashMap::new(),
        }
    }

    /// Adds a key mapping entry
    pub fn add_key(&mut self, row: u8, col: u8, scancode: u16, name: impl Into<String>) {
        let rc = Rc::new(row, col);
        let sckey = ScKey::new(scancode, false);
        let name_str = name.into();

        self.rc_to_sc.insert(rc, sckey);
        self.sc_to_rc.insert(sckey, rc);
        // Only track base scancode name (extended key nuances are handled elsewhere if needed, but for MVP standard scancode is fine)
        self.sc_to_name.insert(scancode, name_str.clone());
        self.name_to_sc.insert(name_str, scancode);
    }

    // Adds a mapping for modifiers or other keys that don't have a Row/Col position in the standard layout matrix
    pub fn add_free_key(&mut self, scancode: u16, name: impl Into<String>) {
        let name_str = name.into();
        self.sc_to_name.insert(scancode, name_str.clone());
        self.name_to_sc.insert(name_str, scancode);
    }

    pub fn key_to_rc(&self, key: ScKey) -> Option<Rc> {
        if key.ext {
            return None; // Mapping doesn't currently use ext flags for matrix positions
        }
        self.sc_to_rc.get(&key).copied()
    }

    pub fn sc_to_key_name(&self, sc: u16) -> Option<&str> {
        self.sc_to_name.get(&sc).map(|s| s.as_str())
    }

    pub fn key_name_to_sc(&self, name: &str) -> Option<u16> {
        self.name_to_sc.get(name).copied()
    }

    pub fn load_from_file(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    pub fn load_from_str(content: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut map = Self::new("Custom");
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("Name:") {
                map.name = rest.trim().to_string();
                continue;
            }

            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() < 4 {
                // Ignore lines that don't match the row,col,scancode,name format
                continue;
            }

            let row = parts[0].parse::<u8>();
            let col = parts[1].parse::<u8>();
            let scancode_str = parts[2];
            let name = parts[3];

            if name.is_empty() {
                continue;
            }

            let scancode = if let Some(hex) = scancode_str
                .strip_prefix("0x")
                .or_else(|| scancode_str.strip_prefix("0X"))
            {
                u16::from_str_radix(hex, 16)
            } else {
                scancode_str.parse::<u16>()
            };

            if let (Ok(r), Ok(c), Ok(sc)) = (row, col, scancode.clone()) {
                map.add_key(r, c, sc, name);
            } else if let Ok(sc) = scancode {
                // if row/col is not parseable but scancode is, maybe it's a free key
                map.add_free_key(sc, name);
            }
        }
        Ok(map)
    }
}

impl Default for KeyboardMap {
    fn default() -> Self {
        Self::new("Default")
    }
}

/// Returns a pre-configured JIS 106/109 KeyboardMap
pub fn new_jis_106() -> KeyboardMap {
    let mut map = KeyboardMap::new("JIS106/109");

    // Row 0: Number row (13 keys)
    map.add_key(0, 0, 0x02, "1");
    map.add_key(0, 1, 0x03, "2");
    map.add_key(0, 2, 0x04, "3");
    map.add_key(0, 3, 0x05, "4");
    map.add_key(0, 4, 0x06, "5");
    map.add_key(0, 5, 0x07, "6");
    map.add_key(0, 6, 0x08, "7");
    map.add_key(0, 7, 0x09, "8");
    map.add_key(0, 8, 0x0A, "9");
    map.add_key(0, 9, 0x0B, "0");
    map.add_key(0, 10, 0x0C, "-");
    map.add_key(0, 11, 0x0D, "^");
    map.add_key(0, 12, 0x7D, "\\"); // Yen

    // Row 1: QWERTY row (12 keys)
    map.add_key(1, 0, 0x10, "q");
    map.add_key(1, 1, 0x11, "w");
    map.add_key(1, 2, 0x12, "e");
    map.add_key(1, 3, 0x13, "r");
    map.add_key(1, 4, 0x14, "t");
    map.add_key(1, 5, 0x15, "y");
    map.add_key(1, 6, 0x16, "u");
    map.add_key(1, 7, 0x17, "i");
    map.add_key(1, 8, 0x18, "o");
    map.add_key(1, 9, 0x19, "p");
    map.add_key(1, 10, 0x1A, "@"); // JIS @
    map.add_key(1, 11, 0x1B, "[");

    // Row 2: ASDF row (12 keys)
    map.add_key(2, 0, 0x1E, "a");
    map.add_key(2, 1, 0x1F, "s");
    map.add_key(2, 2, 0x20, "d");
    map.add_key(2, 3, 0x21, "f");
    map.add_key(2, 4, 0x22, "g");
    map.add_key(2, 5, 0x23, "h");
    map.add_key(2, 6, 0x24, "j");
    map.add_key(2, 7, 0x25, "k");
    map.add_key(2, 8, 0x26, "l");
    map.add_key(2, 9, 0x27, ";");
    map.add_key(2, 10, 0x28, ":");
    map.add_key(2, 11, 0x2B, "]");

    // Row 3: ZXCV row (11 keys)
    map.add_key(3, 0, 0x2C, "z");
    map.add_key(3, 1, 0x2D, "x");
    map.add_key(3, 2, 0x2E, "c");
    map.add_key(3, 3, 0x2F, "v");
    map.add_key(3, 4, 0x30, "b");
    map.add_key(3, 5, 0x31, "n");
    map.add_key(3, 6, 0x32, "m");
    map.add_key(3, 7, 0x33, ",");
    map.add_key(3, 8, 0x34, ".");
    map.add_key(3, 9, 0x35, "/");
    map.add_key(3, 10, 0x73, "_"); // Backslash/Ro

    map.add_free_key(0x39, "space");
    map.add_free_key(0x79, "henkan");
    map.add_free_key(0x7B, "muhenkan");

    map
}

/// Returns a pre-configured US 101/104 KeyboardMap
pub fn new_us_101() -> KeyboardMap {
    let mut map = KeyboardMap::new("US101/104");

    // Row 0: Number row (13 keys) US has ` at left, then 1-0, -, =
    map.add_key(0, 0, 0x29, "`"); // Backtick/Tilde
    map.add_key(0, 1, 0x02, "1");
    map.add_key(0, 2, 0x03, "2");
    map.add_key(0, 3, 0x04, "3");
    map.add_key(0, 4, 0x05, "4");
    map.add_key(0, 5, 0x06, "5");
    map.add_key(0, 6, 0x07, "6");
    map.add_key(0, 7, 0x08, "7");
    map.add_key(0, 8, 0x09, "8");
    map.add_key(0, 9, 0x0A, "9");
    map.add_key(0, 10, 0x0B, "0");
    map.add_key(0, 11, 0x0C, "-");
    map.add_key(0, 12, 0x0D, "="); // Equal

    // Row 1: QWERTY row (13 keys in US layout, up to backslash)
    map.add_key(1, 0, 0x10, "q");
    map.add_key(1, 1, 0x11, "w");
    map.add_key(1, 2, 0x12, "e");
    map.add_key(1, 3, 0x13, "r");
    map.add_key(1, 4, 0x14, "t");
    map.add_key(1, 5, 0x15, "y");
    map.add_key(1, 6, 0x16, "u");
    map.add_key(1, 7, 0x17, "i");
    map.add_key(1, 8, 0x18, "o");
    map.add_key(1, 9, 0x19, "p");
    map.add_key(1, 10, 0x1A, "[");
    map.add_key(1, 11, 0x1B, "]");
    map.add_key(1, 12, 0x2B, "\\");

    // Row 2: ASDF row (11 keys)
    map.add_key(2, 0, 0x1E, "a");
    map.add_key(2, 1, 0x1F, "s");
    map.add_key(2, 2, 0x20, "d");
    map.add_key(2, 3, 0x21, "f");
    map.add_key(2, 4, 0x22, "g");
    map.add_key(2, 5, 0x23, "h");
    map.add_key(2, 6, 0x24, "j");
    map.add_key(2, 7, 0x25, "k");
    map.add_key(2, 8, 0x26, "l");
    map.add_key(2, 9, 0x27, ";");
    map.add_key(2, 10, 0x28, "'"); // Single Quote

    // Row 3: ZXCV row (10 keys)
    map.add_key(3, 0, 0x2C, "z");
    map.add_key(3, 1, 0x2D, "x");
    map.add_key(3, 2, 0x2E, "c");
    map.add_key(3, 3, 0x2F, "v");
    map.add_key(3, 4, 0x30, "b");
    map.add_key(3, 5, 0x31, "n");
    map.add_key(3, 6, 0x32, "m");
    map.add_key(3, 7, 0x33, ",");
    map.add_key(3, 8, 0x34, ".");
    map.add_key(3, 9, 0x35, "/");

    map.add_free_key(0x39, "space");

    map
}

/// Returns a pre-configured AX KeyboardMap
pub fn new_ax() -> KeyboardMap {
    let mut map = KeyboardMap::new("AX");

    // AX is similar to US, but sometimes differs. For MVP we will approximate to standard AX.
    // Assuming mostly US standard + specific keys.
    map.add_key(0, 0, 0x29, "`");
    map.add_key(0, 1, 0x02, "1");
    map.add_key(0, 2, 0x03, "2");
    map.add_key(0, 3, 0x04, "3");
    map.add_key(0, 4, 0x05, "4");
    map.add_key(0, 5, 0x06, "5");
    map.add_key(0, 6, 0x07, "6");
    map.add_key(0, 7, 0x08, "7");
    map.add_key(0, 8, 0x09, "8");
    map.add_key(0, 9, 0x0A, "9");
    map.add_key(0, 10, 0x0B, "0");
    map.add_key(0, 11, 0x0C, "-");
    map.add_key(0, 12, 0x0D, "=");

    map.add_key(1, 0, 0x10, "q");
    map.add_key(1, 1, 0x11, "w");
    map.add_key(1, 2, 0x12, "e");
    map.add_key(1, 3, 0x13, "r");
    map.add_key(1, 4, 0x14, "t");
    map.add_key(1, 5, 0x15, "y");
    map.add_key(1, 6, 0x16, "u");
    map.add_key(1, 7, 0x17, "i");
    map.add_key(1, 8, 0x18, "o");
    map.add_key(1, 9, 0x19, "p");
    map.add_key(1, 10, 0x1A, "[");
    map.add_key(1, 11, 0x1B, "]");
    map.add_key(1, 12, 0x2B, "\\");

    map.add_key(2, 0, 0x1E, "a");
    map.add_key(2, 1, 0x1F, "s");
    map.add_key(2, 2, 0x20, "d");
    map.add_key(2, 3, 0x21, "f");
    map.add_key(2, 4, 0x22, "g");
    map.add_key(2, 5, 0x23, "h");
    map.add_key(2, 6, 0x24, "j");
    map.add_key(2, 7, 0x25, "k");
    map.add_key(2, 8, 0x26, "l");
    map.add_key(2, 9, 0x27, ";");
    map.add_key(2, 10, 0x28, "'");

    map.add_key(3, 0, 0x2C, "z");
    map.add_key(3, 1, 0x2D, "x");
    map.add_key(3, 2, 0x2E, "c");
    map.add_key(3, 3, 0x2F, "v");
    map.add_key(3, 4, 0x30, "b");
    map.add_key(3, 5, 0x31, "n");
    map.add_key(3, 6, 0x32, "m");
    map.add_key(3, 7, 0x33, ",");
    map.add_key(3, 8, 0x34, ".");
    map.add_key(3, 9, 0x35, "/");

    // AX specific modifiers for thumb operations usually map to Space/Convert etc, or right Alt.
    map.add_free_key(0x39, "space");
    // AX Right Alt is typically used as KANJI toggle, assume standard scan codes for now
    map.add_free_key(0x79, "henkan");
    map.add_free_key(0x7B, "muhenkan");

    map
}
