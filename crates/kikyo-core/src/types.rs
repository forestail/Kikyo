use serde::{Deserialize, Serialize};

/// Windows Scancode + Extended flag key identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScKey {
    pub sc: u16,
    pub ext: bool,
}

impl ScKey {
    pub const fn new(sc: u16, ext: bool) -> Self {
        Self { sc, ext }
    }
}

/// Event to be injected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// Scancode injection (scancode, ext, up).
    Scancode(u16, bool, bool),
    /// Unicode character injection (char, up).
    Unicode(char, bool),
    /// IME Control (true=ON, false=OFF).
    ImeControl(bool),
    /// Wait until IME status matches the expected value (expected, timeout_ms).
    WaitUntilImeStatus(bool, u64),
    /// Delay for a specified duration (ms).
    Delay(u64),
    /// Inject a string with robust IME handling (check status -> OFF -> inject -> ON).
    DirectString(String),
}

/// Action to be taken by the hook.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    Pass,
    Block,
    /// Inject a sequence of input events.
    Inject(Vec<InputEvent>),
}

/// Row and Column in the layout matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rc {
    pub row: u8, // 0-indexed (e.g. 0=NumRow, 1=QWERTY...)
    pub col: u8, // 0-indexed
}

impl Rc {
    pub const fn new(row: u8, col: u8) -> Self {
        Self { row, col }
    }
}

/// Modifier keys applied to a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

impl Modifiers {
    pub const fn none() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
        }
    }

    pub const fn is_empty(self) -> bool {
        !(self.ctrl || self.shift || self.alt || self.win)
    }
}

/// Key specification inside a keystroke sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySpec {
    /// A character to be mapped to a scancode (fallback to Unicode if unknown).
    Char(char),
    /// Explicit scancode (scancode, ext).
    Scancode(u16, bool),
    /// Virtual key code (VK).
    VirtualKey(u16),
    /// Turn IME On (Japanese Input).
    ImeOn,
    /// Turn IME Off (Alphanumeric Input).
    ImeOff,
    /// Direct string output (IME confirmed).
    DirectString(String),
}

/// A single keystroke with optional modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStroke {
    pub key: KeySpec,
    pub mods: Modifiers,
}

/// Output token from a layout cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Character sequence to be injected via key presses (e.g. "ni", "ka").
    /// In MVP, we might treat this as a sequence of keys.
    KeySequence(Vec<KeyStroke>),

    /// Character to be injected via Unicode input (IME-like behavior).
    /// Quoted with single quotes in .yab (e.g. '－').
    ImeChar(String),

    /// Character to be injected directly (bypassing IME).
    /// Quoted with double quotes in .yab (e.g. "、").
    /// Note: MVP might treat this similarly to ImeChar or verify behavior.
    DirectChar(String),

    /// No output (empty cell).
    None,
}

/// A plane is a grid of tokens, indexed by (row, col).
/// For MVP, we use a simple Vec or HashMap.
/// Since rows are fixed (0..3) and cols are small, we can store efficiently.
/// But a HashMap<Rc, Token> is easiest for sparse/dense mix.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plane {
    pub map: std::collections::HashMap<Rc, Token>,
}

/// A section contains a base plane and optional sub-planes (chord planes).
#[derive(Debug, Clone, Default)]
pub struct Section {
    pub name: String,
    pub base_plane: Plane,
    // Map from plane tag (e.g. "<k>") to Plane
    pub sub_planes: std::collections::HashMap<String, Plane>,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub name: Option<String>,
    pub sections: std::collections::HashMap<String, Section>,
    pub function_key_swaps: Vec<(String, String)>,
    pub max_chord_size: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            name: None,
            sections: std::collections::HashMap::new(),
            function_key_swaps: Vec::new(),
            max_chord_size: 2,
        }
    }
}
