use crate::types::ScKey;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const EXTENDED_KEY_1_SC: u16 = 0x0201;
pub const EXTENDED_KEY_2_SC: u16 = 0x0202;
pub const EXTENDED_KEY_3_SC: u16 = 0x0203;
pub const EXTENDED_KEY_4_SC: u16 = 0x0204;

/// Internal event type for the engine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdge {
    Down,
    Up,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: ScKey,
    pub edge: KeyEdge,
    pub injected: bool,
    pub t: Instant,
}

/// Output decision from the engine
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// No special handling (or pass through for non-target keys)
    Passthrough(ScKey, KeyEdge),
    /// Determined as a single tap
    KeyTap(ScKey),
    /// Determined as a chord
    Chord(Vec<ScKey>),
    /// Start a latch (continuous shift)
    LatchOn(LatchKind),
    /// End a latch
    LatchOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchKind {
    OneShot,
    Lock,
}

/// Abstract representation of a plane or modifier identity
pub type PlaneTag = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChordStyle {
    ThumbShift,
    TriggerKey,
    NNumberKey,
}

impl Default for ChordStyle {
    fn default() -> Self {
        Self::TriggerKey
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThumbKeys {
    pub left: HashSet<ScKey>,
    pub right: HashSet<ScKey>,
    pub ext1: HashSet<ScKey>,
    pub ext2: HashSet<ScKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AdaptiveCfg {
    pub enabled: bool,
    // Add parameters for adaptive window here later
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThumbShiftKeyMode {
    NonTransformTransform, // 無変換 - 変換
    NonTransformSpace,     // 無変換 - スペース
    SpaceTransform,        // スペース - 変換
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThumbShiftSinglePress {
    None,        // 無効
    Enable,      // 有効
    PrefixShift, // 前置シフト
    SpaceKey,    // Spaceキー
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImeMode {
    Auto,
    Imm,
    Tsf,
    Ignore,     // Force Japanese (Roman)
    ForceAlpha, // Force Alphanumeric
}

impl Default for ImeMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuspendKey {
    None,
    ScrollLock,
    Pause,
    Insert,
    RightShift,
    RightControl,
    RightAlt,
}

impl Default for SuspendKey {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SuccessiveCfg {
    pub enabled: bool,
    // TODO: Add details
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThumbKeySelect {
    None,
    Esc,
    Tab,
    Muhenkan,
    Space,
    Henkan,
    Enter,
    BackSpace,
    Delete,
    Insert,
    Up,
    Left,
    Right,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    Extended1,
    Extended2,
    Extended3,
    Extended4,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThumbSideConfig {
    pub key: ThumbKeySelect,
    pub continuous: bool,
    pub single_press: ThumbShiftSinglePress,
    pub repeat: bool,
}

impl Default for ThumbSideConfig {
    fn default() -> Self {
        Self {
            key: ThumbKeySelect::None,
            continuous: false,
            single_press: ThumbShiftSinglePress::None,
            repeat: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    #[serde(default)]
    pub chord_style: ChordStyle,
    #[serde(default = "default_chord_window_ms")]
    pub chord_window_ms: u64,
    #[serde(default = "default_max_chord_size")]
    pub max_chord_size: usize,
    #[serde(default)]
    pub adaptive_window: AdaptiveCfg,
    #[serde(default)]
    pub thumb_keys: Option<ThumbKeys>,
    #[serde(default)]
    pub trigger_keys: HashMap<ScKey, PlaneTag>,
    #[serde(default)]
    pub target_keys: Option<HashSet<ScKey>>,
    #[serde(default)]
    pub successive: SuccessiveCfg,

    #[serde(default)]
    pub char_key_repeat_assigned: bool,
    #[serde(default = "default_char_key_repeat_unassigned")]
    pub char_key_repeat_unassigned: bool,

    #[serde(default)]
    pub ime_mode: ImeMode,
    #[serde(default)]
    pub suspend_key: SuspendKey,

    // New separate configurations
    #[serde(default)]
    pub thumb_left: ThumbSideConfig,
    #[serde(default)]
    pub thumb_right: ThumbSideConfig,
    #[serde(default)]
    pub extended_thumb1: ThumbSideConfig,
    #[serde(default)]
    pub extended_thumb2: ThumbSideConfig,
    #[serde(default = "default_thumb_shift_overlap_ratio")]
    pub thumb_shift_overlap_ratio: f64, // Kept global as per implementation plan but not strictly required to be split by user yet

    #[serde(default)]
    pub char_key_continuous: bool,
    #[serde(default = "default_char_key_overlap_ratio")]
    pub char_key_overlap_ratio: f64,
}

fn default_chord_window_ms() -> u64 {
    200
}

fn default_max_chord_size() -> usize {
    2
}

fn default_char_key_repeat_unassigned() -> bool {
    true
}

fn default_thumb_shift_overlap_ratio() -> f64 {
    0.35
}

fn default_char_key_overlap_ratio() -> f64 {
    0.35
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            chord_style: ChordStyle::TriggerKey,
            chord_window_ms: 200,
            max_chord_size: 2,
            adaptive_window: AdaptiveCfg { enabled: false },
            thumb_keys: None,
            trigger_keys: HashMap::new(),
            target_keys: None,
            successive: SuccessiveCfg { enabled: false },

            char_key_repeat_assigned: false,
            char_key_repeat_unassigned: true,

            ime_mode: ImeMode::Auto,
            suspend_key: SuspendKey::None,

            thumb_left: ThumbSideConfig {
                key: ThumbKeySelect::Muhenkan,
                continuous: false,
                single_press: ThumbShiftSinglePress::None,
                repeat: false,
            },
            thumb_right: ThumbSideConfig {
                key: ThumbKeySelect::Henkan,
                continuous: false,
                single_press: ThumbShiftSinglePress::None,
                repeat: false,
            },
            extended_thumb1: ThumbSideConfig {
                key: ThumbKeySelect::Extended1,
                continuous: false,
                single_press: ThumbShiftSinglePress::None,
                repeat: false,
            },
            extended_thumb2: ThumbSideConfig {
                key: ThumbKeySelect::Extended2,
                continuous: false,
                single_press: ThumbShiftSinglePress::None,
                repeat: false,
            },
            thumb_shift_overlap_ratio: 0.35,

            char_key_continuous: false,
            char_key_overlap_ratio: 0.35,
        }
    }
}

impl ThumbKeySelect {
    pub fn to_sckey(&self) -> Option<ScKey> {
        match self {
            ThumbKeySelect::None => None,
            ThumbKeySelect::Esc => Some(ScKey::new(0x01, false)),
            ThumbKeySelect::Tab => Some(ScKey::new(0x0F, false)),
            ThumbKeySelect::Muhenkan => Some(ScKey::new(0x7B, false)),
            ThumbKeySelect::Space => Some(ScKey::new(0x39, false)),
            ThumbKeySelect::Henkan => Some(ScKey::new(0x79, false)),
            ThumbKeySelect::Enter => Some(ScKey::new(0x1C, false)),
            ThumbKeySelect::BackSpace => Some(ScKey::new(0x0E, false)),
            ThumbKeySelect::Delete => Some(ScKey::new(0x53, true)),
            ThumbKeySelect::Insert => Some(ScKey::new(0x52, true)),
            ThumbKeySelect::Up => Some(ScKey::new(0x48, true)),
            ThumbKeySelect::Left => Some(ScKey::new(0x4B, true)),
            ThumbKeySelect::Right => Some(ScKey::new(0x4D, true)),
            ThumbKeySelect::Down => Some(ScKey::new(0x50, true)),
            ThumbKeySelect::Home => Some(ScKey::new(0x47, true)),
            ThumbKeySelect::End => Some(ScKey::new(0x4F, true)),
            ThumbKeySelect::PageUp => Some(ScKey::new(0x49, true)),
            ThumbKeySelect::PageDown => Some(ScKey::new(0x51, true)),
            ThumbKeySelect::LeftShift => Some(ScKey::new(0x2A, false)),
            ThumbKeySelect::RightShift => Some(ScKey::new(0x36, false)),
            ThumbKeySelect::LeftCtrl => Some(ScKey::new(0x1D, false)),
            ThumbKeySelect::RightCtrl => Some(ScKey::new(0x1D, true)),
            ThumbKeySelect::Extended1 => Some(ScKey::new(EXTENDED_KEY_1_SC, false)),
            ThumbKeySelect::Extended2 => Some(ScKey::new(EXTENDED_KEY_2_SC, false)),
            ThumbKeySelect::Extended3 => Some(ScKey::new(EXTENDED_KEY_3_SC, false)),
            ThumbKeySelect::Extended4 => Some(ScKey::new(EXTENDED_KEY_4_SC, false)),
        }
    }
}

impl Profile {
    pub fn update_thumb_keys(&mut self) {
        let mut left = HashSet::new();
        let mut right = HashSet::new();
        let mut ext1 = HashSet::new();
        let mut ext2 = HashSet::new();

        if let Some(sck) = self.thumb_left.key.to_sckey() {
            left.insert(sck);
        }
        if let Some(sck) = self.thumb_right.key.to_sckey() {
            right.insert(sck);
        }
        if let Some(sck) = self.extended_thumb1.key.to_sckey() {
            ext1.insert(sck);
        }
        if let Some(sck) = self.extended_thumb2.key.to_sckey() {
            ext2.insert(sck);
        }

        self.thumb_keys = Some(ThumbKeys {
            left,
            right,
            ext1,
            ext2,
        });
    }
}

pub struct ChordState {
    pub enabled: bool,
    pub pressed: HashSet<ScKey>,
    pub down_ts: HashMap<ScKey, Instant>,
    pub pending: Vec<PendingKey>,
    pub latch: LatchState,
    pub passed_keys: HashSet<ScKey>,
    // Track modifiers used in generated chords to detect single-press vs used-as-modifier
    pub used_modifiers: HashSet<ScKey>,
    // For Prefix Shift mode
    pub prefix_pending: Option<ScKey>,
}

impl Default for ChordState {
    fn default() -> Self {
        Self {
            enabled: true,
            pressed: HashSet::new(),
            down_ts: HashMap::new(),
            pending: Vec::new(),
            latch: LatchState::None,
            passed_keys: HashSet::new(),
            used_modifiers: HashSet::new(),
            prefix_pending: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifierKind {
    None,
    ThumbLeft,
    ThumbRight,
    ThumbExt1,
    ThumbExt2,
    CharShift,
}

impl ModifierKind {
    fn is_modifier(self) -> bool {
        !matches!(self, ModifierKind::None)
    }
}
#[derive(Debug, Clone)]
pub struct PendingKey {
    pub key: ScKey,
    pub t_down: Instant,
    pub t_up: Option<Instant>,
    // kind_hint: PendingKindHint
}

#[derive(Debug, Clone, PartialEq)]
pub enum LatchState {
    None,
    OneShot(PlaneTag),
    Lock(PlaneTag),
    // Deadline(PlaneTag, Instant),
}

pub struct ChordEngine {
    pub profile: Profile, // Make profile public too if needed, or just state
    pub state: ChordState,
}

impl ChordEngine {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            state: ChordState::default(),
        }
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
    }

    pub fn on_event(&mut self, event: KeyEvent) -> Vec<Decision> {
        if event.injected {
            return vec![];
        }

        let now = event.t;
        let mut output = Vec::new();

        // 0. Priority Handling Checklist
        match event.edge {
            KeyEdge::Up => {
                // Check passed keys (Always pass through regardless of target list)
                if self.state.passed_keys.contains(&event.key) {
                    self.state.passed_keys.remove(&event.key);
                    self.state.pressed.remove(&event.key);
                    self.state.down_ts.remove(&event.key);
                    return vec![Decision::Passthrough(event.key, KeyEdge::Up)];
                }
            }
            KeyEdge::Down => {
                // Special Handling for Space Key (0x39) - Always check first
                // BUT only if it is NOT a modifier (thumb key)
                if event.key.sc == 0x39 && !self.is_modifier_key(event.key) {
                    // Flush existing pending keys FIRST.
                    output.extend(self.flush_pending_with_cutoff(now));

                    // Output Space as Passthrough (Down) immediately.
                    self.state.passed_keys.insert(event.key);
                    output.push(Decision::Passthrough(event.key, KeyEdge::Down));

                    // Since we handled Space, we return immediately with the sequence
                    return output;
                }
            }
        }

        // 1. Filter non-target keys (if whitelist is active)
        if let Some(ref targets) = self.profile.target_keys {
            if !targets.contains(&event.key) {
                // Not in target list -> Pass through immediately
                return vec![Decision::Passthrough(event.key, event.edge)];
            }
        }

        match event.edge {
            KeyEdge::Down => {
                // 1. Update pressed state
                self.state.pressed.insert(event.key);
                self.state.down_ts.insert(event.key, now);
                if matches!(self.modifier_kind(event.key), ModifierKind::CharShift) {
                    self.state.used_modifiers.remove(&event.key);
                }

                // Handle Prefix Shift Logic
                if let Some(prefix_thumb) = self.state.prefix_pending {
                    // If a valid key comes in (and it's not the thumb itself, though ScKey check assumes unique)
                    // We assume the prefix thumb is applied to this key.
                    // Note: If the new key is also a modifier, we might chain?
                    // For now, assume applying to any new key.
                    self.state.prefix_pending = None;
                    self.state.used_modifiers.insert(prefix_thumb); // Mark as used
                                                                    // Return Chord immediately
                    output.push(Decision::Chord(vec![prefix_thumb, event.key]));

                    // We consume this key event immediately.
                    // But we also need to consider if this key is a modifier?
                    // If the pressed key is "A", fine.
                    // We don't add to pending.
                    return output;
                }

                // 2. Add to pending
                // Avoid duplicates (if repeat comes in)
                if !self.state.pending.iter().any(|p| p.key == event.key) {
                    self.state.pending.push(PendingKey {
                        key: event.key,
                        t_down: now,
                        t_up: None,
                    });
                }

                // 3. Check chords
                let chords = self.check_chords(now, Some((event.key, event.edge)));
                output.extend(chords);
            }
            KeyEdge::Up => {
                // 1. Update state
                self.state.pressed.remove(&event.key);
                // Mark t_up in pending
                if let Some(p) = self.state.pending.iter_mut().find(|p| p.key == event.key) {
                    p.t_up = Some(now);
                }

                // 2. Check for chord formation
                let chords = self.check_chords(now, Some((event.key, event.edge)));
                output.extend(chords);

                // 3. Flush Single Taps
                if self.state.pending.len() == 1 {
                    let p = &self.state.pending[0];
                    if p.t_up.is_some() {
                        // It's a lonely tap
                        let key = p.key;
                        let mod_kind = self.modifier_kind(key);

                        self.state.pending.clear();
                        self.state.down_ts.remove(&key);

                        match mod_kind {
                            ModifierKind::ThumbLeft
                            | ModifierKind::ThumbRight
                            | ModifierKind::ThumbExt1
                            | ModifierKind::ThumbExt2 => {
                                if self.state.used_modifiers.contains(&key) {
                                    // Was used, so ignore single press
                                    self.state.used_modifiers.remove(&key);
                                } else {
                                    let sp_setting = match mod_kind {
                                        ModifierKind::ThumbLeft => {
                                            self.profile.thumb_left.single_press
                                        }
                                        ModifierKind::ThumbRight => {
                                            self.profile.thumb_right.single_press
                                        }
                                        ModifierKind::ThumbExt1 => {
                                            self.profile.extended_thumb1.single_press
                                        }
                                        ModifierKind::ThumbExt2 => {
                                            self.profile.extended_thumb2.single_press
                                        }
                                        _ => ThumbShiftSinglePress::None,
                                    };

                                    match sp_setting {
                                        ThumbShiftSinglePress::None => {
                                            // Disable single press (swallow)
                                        }
                                        ThumbShiftSinglePress::Enable => {
                                            output.push(Decision::KeyTap(key));
                                        }
                                        ThumbShiftSinglePress::PrefixShift => {
                                            self.state.prefix_pending = Some(key);
                                        }
                                        ThumbShiftSinglePress::SpaceKey => {
                                            output.push(Decision::KeyTap(ScKey::new(0x39, false)));
                                        }
                                    }
                                }
                            }
                            ModifierKind::CharShift => {
                                if self.state.used_modifiers.contains(&key) {
                                    self.state.used_modifiers.remove(&key);
                                } else {
                                    output.push(Decision::KeyTap(key));
                                }
                            }
                            ModifierKind::None => {
                                output.push(Decision::KeyTap(key));
                            }
                        }
                    }
                }
            }
        }

        // Cycle flush to handle timeouts
        // REMOVED: flush_expired logic to allow infinite window (overlap based only)
        // output.extend(self.flush_expired(now));

        output
    }

    /* REMOVED flush_expired to disable time window
    pub fn flush_expired(&mut self, now: Instant) -> Vec<Decision> {
        // Simple flush: if > window, force KeyTap.
        let window = Duration::from_millis(self.profile.chord_window_ms);
        let mut output = Vec::new();

        // We use retain logic but we need to extract items.
        // Identify expired keys first.
        let mut expired_indices = Vec::new();
        for (i, p) in self.state.pending.iter().enumerate() {
            if now.duration_since(p.t_down) > window {
                expired_indices.push(i);
            }
        }

        if expired_indices.is_empty() {
            return output;
        }

        // Process in reverse to keep indices valid
        for &i in expired_indices.iter().rev() {
            let p = self.state.pending.remove(i);
            // It expired, so it's a Tap.
            output.push(Decision::KeyTap(p.key));

            // Clean up down_ts if it's not pressed
            if !self.state.pressed.contains(&p.key) {
                self.state.down_ts.remove(&p.key);
            }
        }

        // Restore order? Removing from Vec in reverse means we process B then A if both expired.
        // This reverses output order relative to input order.
        // Ideally we should process oldest first.
        output.reverse();

        output
    }
    */

    pub fn flush_pending_with_cutoff(&mut self, now: Instant) -> Vec<Decision> {
        // Force-release pending keys at 'now' so chord ratio can be evaluated.
        for p in self.state.pending.iter_mut() {
            if p.t_up.is_none() {
                p.t_up = Some(now);
            }
        }

        let mut output = self.check_chords(now, None);

        if !self.state.pending.is_empty() {
            let pending = std::mem::take(&mut self.state.pending);
            for p in pending {
                output.push(Decision::KeyTap(p.key));
                // Clean up down_ts if it's not pressed
                if !self.state.pressed.contains(&p.key) {
                    self.state.down_ts.remove(&p.key);
                }
            }
        }

        output
    }

    pub fn flush_all_pending(&mut self) -> Vec<Decision> {
        let mut output = Vec::new();
        // Drain all pending keys and output them as KeyTap
        let pending = std::mem::take(&mut self.state.pending);

        for p in pending {
            output.push(Decision::KeyTap(p.key));
            // Clean up down_ts if it's not pressed
            if !self.state.pressed.contains(&p.key) {
                self.state.down_ts.remove(&p.key);
            }
        }

        output
    }

    fn check_chords(&mut self, now: Instant, trigger: Option<(ScKey, KeyEdge)>) -> Vec<Decision> {
        let mut output = Vec::new();
        if self.state.pending.len() < 2 {
            return output;
        }

        let mut consumed_indices = HashSet::new();
        let mut flushed_indices = HashSet::new();

        let mut ordered_indices: Vec<usize> = (0..self.state.pending.len()).collect();
        ordered_indices.sort_by(|a, b| {
            self.state.pending[*a]
                .t_down
                .cmp(&self.state.pending[*b].t_down)
        });

        for oi in 0..ordered_indices.len() {
            let idx1 = ordered_indices[oi];
            if consumed_indices.contains(&idx1) || flushed_indices.contains(&idx1) {
                continue;
            }

            for oj in (oi + 1)..ordered_indices.len() {
                let idx2 = ordered_indices[oj];
                if consumed_indices.contains(&idx2) || flushed_indices.contains(&idx2) {
                    continue;
                }

                let p1 = &self.state.pending[idx1];
                let p2 = &self.state.pending[idx2];

                let ratio = match self.pair_overlap_ratio(p1, p2, now, trigger) {
                    Some(ratio) => ratio,
                    None => {
                        // Wait for the first unresolved newer key (time-order preserving).
                        break;
                    }
                };

                if ratio >= self.profile.char_key_overlap_ratio {
                    let k1 = p1.key;
                    let k2 = p2.key;
                    let kind1 = self.modifier_kind(k1);
                    let kind2 = self.modifier_kind(k2);

                    if kind1.is_modifier() {
                        self.state.used_modifiers.insert(k1);
                    }
                    if kind2.is_modifier() {
                        self.state.used_modifiers.insert(k2);
                    }

                    let continuous1 = self.modifier_is_continuous(kind1);
                    let continuous2 = self.modifier_is_continuous(kind2);

                    let keep1 =
                        kind1.is_modifier() && continuous1 && self.state.pressed.contains(&k1);
                    let keep2 =
                        kind2.is_modifier() && continuous2 && self.state.pressed.contains(&k2);

                    if !keep1 {
                        consumed_indices.insert(idx1);
                    }
                    if !keep2 {
                        consumed_indices.insert(idx2);
                    }

                    output.push(Decision::Chord(vec![k1, k2]));

                    if consumed_indices.contains(&idx1) {
                        break;
                    }
                } else {
                    flushed_indices.insert(idx1);

                    let kind1 = self.modifier_kind(p1.key);
                    let suppress_p1_tap = kind1.is_modifier()
                        && self.modifier_is_continuous(kind1)
                        && self.state.used_modifiers.contains(&p1.key);

                    if !suppress_p1_tap {
                        output.push(Decision::KeyTap(p1.key));
                    }

                    break;
                }
            }
        }

        if !consumed_indices.is_empty() || !flushed_indices.is_empty() {
            let mut new_pending = Vec::new();
            for (i, p) in self.state.pending.iter().enumerate() {
                if consumed_indices.contains(&i) {
                    if !self.state.pressed.contains(&p.key) {
                        self.state.down_ts.remove(&p.key);
                    }
                    continue;
                }
                if flushed_indices.contains(&i) {
                    if !self.state.pressed.contains(&p.key) {
                        self.state.down_ts.remove(&p.key);
                    }
                    continue;
                }
                new_pending.push(p.clone());
            }
            self.state.pending = new_pending;
        }

        output
    }

    fn pair_overlap_ratio(
        &self,
        p1: &PendingKey,
        p2: &PendingKey,
        now: Instant,
        trigger: Option<(ScKey, KeyEdge)>,
    ) -> Option<f64> {
        let p1_end = p1.t_up.unwrap_or(now);

        let (p2_end, ratio_den) = if let Some(p2_up) = p2.t_up {
            let p2_dur = p2_up.duration_since(p2.t_down);
            if p2_dur.as_micros() > 0 {
                (p2_up, p2_dur.as_secs_f64())
            } else {
                (p2_up, 0.0)
            }
        } else {
            if p1.t_up.is_none() {
                return None;
            }

            let kind1 = self.modifier_kind(p1.key);
            let kind2 = self.modifier_kind(p2.key);
            let immediate_continuous_modifier =
                kind2.is_modifier() && self.modifier_is_continuous(kind2) && !kind1.is_modifier();

            if immediate_continuous_modifier {
                let p1_dur = p1_end.duration_since(p1.t_down);
                if p1_dur.as_micros() > 0 {
                    (p1_end, p1_dur.as_secs_f64())
                } else {
                    (p1_end, 0.0)
                }
            } else {
                let is_char_pair = !matches!(
                    kind1,
                    ModifierKind::ThumbLeft
                        | ModifierKind::ThumbRight
                        | ModifierKind::ThumbExt1
                        | ModifierKind::ThumbExt2
                ) && !matches!(
                    kind2,
                    ModifierKind::ThumbLeft
                        | ModifierKind::ThumbRight
                        | ModifierKind::ThumbExt1
                        | ModifierKind::ThumbExt2
                );
                let third_key_down = matches!(
                    trigger,
                    Some((k, KeyEdge::Down)) if k != p1.key && k != p2.key
                ) && now > p2.t_down;

                if !(self.profile.char_key_continuous && is_char_pair && third_key_down) {
                    return None;
                }

                let p2_dur = now.duration_since(p2.t_down);
                if p2_dur.as_micros() == 0 {
                    return None;
                }
                (now, p2_dur.as_secs_f64())
            }
        };

        let overlap_start = p2.t_down;
        let overlap_end = if p1_end < p2_end { p1_end } else { p2_end };
        let overlap_dur = if overlap_end > overlap_start {
            overlap_end.duration_since(overlap_start)
        } else {
            Duration::ZERO
        };

        if ratio_den > 0.0 {
            Some(overlap_dur.as_secs_f64() / ratio_den)
        } else {
            Some(0.0)
        }
    }

    fn modifier_kind(&self, key: ScKey) -> ModifierKind {
        if let Some(ref tk) = self.profile.thumb_keys {
            if tk.left.contains(&key) {
                return ModifierKind::ThumbLeft;
            }
            if tk.right.contains(&key) {
                return ModifierKind::ThumbRight;
            }
            if tk.ext1.contains(&key) {
                return ModifierKind::ThumbExt1;
            }
            if tk.ext2.contains(&key) {
                return ModifierKind::ThumbExt2;
            }
        }

        if self.profile.trigger_keys.contains_key(&key) {
            return ModifierKind::CharShift;
        }

        ModifierKind::None
    }

    fn modifier_is_continuous(&self, kind: ModifierKind) -> bool {
        match kind {
            ModifierKind::ThumbLeft => self.profile.thumb_left.continuous,
            ModifierKind::ThumbRight => self.profile.thumb_right.continuous,
            ModifierKind::ThumbExt1 => self.profile.extended_thumb1.continuous,
            ModifierKind::ThumbExt2 => self.profile.extended_thumb2.continuous,
            ModifierKind::CharShift => self.profile.char_key_continuous,
            ModifierKind::None => false,
        }
    }

    fn is_modifier_key(&self, key: ScKey) -> bool {
        self.modifier_kind(key).is_modifier()
    }

    // Tests will be added later
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_key(sc: u16) -> ScKey {
        ScKey { sc, ext: false }
    }

    fn make_event(key: ScKey, edge: KeyEdge, t: Instant) -> KeyEvent {
        KeyEvent {
            key,
            edge,
            injected: false,
            t,
        }
    }

    fn continuous_char_profile(threshold: f64, modifiers: &[ScKey]) -> Profile {
        let mut profile = Profile::default();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = threshold;
        for key in modifiers {
            profile
                .trigger_keys
                .insert(*key, format!("<{:02X}>", key.sc));
        }
        profile
    }

    fn assert_single_chord(res: &[Decision], k1: ScKey, k2: ScKey) {
        assert_eq!(res.len(), 1, "Expected single decision, got {:?}", res);
        match &res[0] {
            Decision::Chord(keys) => assert_eq!(keys, &vec![k1, k2]),
            _ => panic!("Expected Chord({:?}, {:?}), got {:?}", k1, k2, res),
        }
    }

    #[test]
    fn test_basic_chord_nested_overlap() {
        // A(Down) -> B(Down) -> B(Up) -> A(Up)
        // Ratio should be 1.0 (100%)
        let mut profile = Profile::default();
        profile.chord_window_ms = 200;
        profile.char_key_overlap_ratio = 0.35;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        // 1. Down A
        engine.on_event(make_event(k1, KeyEdge::Down, t0));
        // 2. Down B at +10
        engine.on_event(make_event(
            k2,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        // 3. Up B at +60 (Duration 50)
        let res = engine.on_event(make_event(k2, KeyEdge::Up, t0 + Duration::from_millis(60)));

        // At this point:
        // P1(A) Down t0, Up None.
        // P2(B) Down t0+10, Up t0+60.
        // A is still down, so A "covers" B strictly.
        // Overlap = Duration of B = 50. Ratio = 1.0.
        // Should produce Chord(A, B).

        assert_eq!(res.len(), 1);
        if let Decision::Chord(keys) = &res[0] {
            assert!(keys.contains(&k1));
            assert!(keys.contains(&k2));
        } else {
            panic!("Expected Chord, got {:?}", res);
        }
    }

    #[test]
    fn test_ratio_sequential() {
        // A(Down) -> A(Up) -> B(Down) -> B(Up)
        // Ratio 0.
        let mut profile = Profile::default();
        profile.chord_window_ms = 200;
        profile.char_key_overlap_ratio = 0.35;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        engine.on_event(make_event(k1, KeyEdge::Down, t0));

        let res1 = engine.on_event(make_event(k1, KeyEdge::Up, t0 + Duration::from_millis(50)));
        assert_eq!(res1.len(), 1);
        assert_eq!(res1[0], Decision::KeyTap(k1));

        engine.on_event(make_event(
            k2,
            KeyEdge::Down,
            t0 + Duration::from_millis(60),
        ));

        let res2 = engine.on_event(make_event(k2, KeyEdge::Up, t0 + Duration::from_millis(110)));

        // A tap, B tap.
        // BUp should flush A (Tap) and also flush B (Tap) because B is lonely.
        // But let's check content.
        let taps: Vec<ScKey> = res2
            .iter()
            .filter_map(|d| match d {
                Decision::KeyTap(k) => Some(*k),
                _ => None,
            })
            .collect();

        // assert!(taps.contains(&k1), "Should contain Tap A"); // A is already flushed
        assert!(taps.contains(&k2), "Should contain Tap B");
    }

    #[test]
    fn test_ratio_chord_pass() {
        // A(Down) -> B(Down) -> A(Up) -> B(Up)
        // A: [0, 100], B: [50, 150]. B Dur = 100.
        // Overlap: [50, 100] = 50ms.
        // Ratio: 0.5 >= 0.35. -> Chord.
        let mut profile = Profile::default();
        profile.chord_window_ms = 200;
        profile.char_key_overlap_ratio = 0.35;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        engine.on_event(make_event(k1, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k2,
            KeyEdge::Down,
            t0 + Duration::from_millis(50),
        ));

        // A Up at 100.
        // At this point B is Down but not Up. Wait.
        let res1 = engine.on_event(make_event(k1, KeyEdge::Up, t0 + Duration::from_millis(100)));
        assert!(res1.is_empty(), "Should wait for B release");

        // B Up at 150.
        let res2 = engine.on_event(make_event(k2, KeyEdge::Up, t0 + Duration::from_millis(150)));

        assert_eq!(res2.len(), 1);
        match &res2[0] {
            Decision::Chord(keys) => {
                assert!(keys.contains(&k1));
                assert!(keys.contains(&k2));
            }
            _ => panic!("Expected Chord, got {:?}", res2),
        }
    }

    #[test]
    fn test_ratio_chord_fail() {
        // A(Down) -> B(Down) -> A(Up) -> B(Up)
        // A: [0, 60], B: [50, 150]. B Dur = 100.
        // Overlap: [50, 60] = 10ms.
        // Ratio: 0.1 < 0.35. -> Tap A, Tap B.
        let mut profile = Profile::default();
        profile.chord_window_ms = 200;
        profile.char_key_overlap_ratio = 0.35;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        engine.on_event(make_event(k1, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k2,
            KeyEdge::Down,
            t0 + Duration::from_millis(50),
        ));
        engine.on_event(make_event(k1, KeyEdge::Up, t0 + Duration::from_millis(60)));

        let res = engine.on_event(make_event(k2, KeyEdge::Up, t0 + Duration::from_millis(150)));

        let taps: Vec<ScKey> = res
            .iter()
            .filter_map(|d| match d {
                Decision::KeyTap(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert!(taps.contains(&k1));
        assert!(taps.contains(&k2));
    }

    #[test]
    fn test_chord_long_delay() {
        // A(Down) --- wait 500ms --- B(Down) -> B(Up) -> A(Up)
        // Even with long specific wait, if overlap is good, it should chord.
        // A Down at 0.
        // B Down at 500. B Up at 600. (Dur 100).
        // Overlap [500, 600] = 100. Ratio = 1.0.
        // Old logic would timeout A at 200ms. New logic should find Chord.
        let mut profile = Profile::default();
        profile.chord_window_ms = 200; // should be ignored
        profile.char_key_overlap_ratio = 0.35;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        // 1. Down A
        engine.on_event(make_event(k1, KeyEdge::Down, t0));

        // 2. Down B at +500ms
        let t_b_down = t0 + Duration::from_millis(500);
        engine.on_event(make_event(k2, KeyEdge::Down, t_b_down));

        // 3. Up B at +600ms
        let t_b_up = t_b_down + Duration::from_millis(100);
        let res = engine.on_event(make_event(k2, KeyEdge::Up, t_b_up));

        // Should be Chord
        assert_eq!(res.len(), 1);
        if let Decision::Chord(keys) = &res[0] {
            assert!(keys.contains(&k1));
            assert!(keys.contains(&k2));
        } else {
            // It might fail if we didn't remove the window check
            panic!("Expected Chord (long delay), got {:?}", res);
        }

        // Cleanup A
        engine.on_event(make_event(
            k1,
            KeyEdge::Up,
            t_b_up + Duration::from_millis(100),
        ));
    }

    #[test]
    fn test_char_continuous_case1_ab_then_ac() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.35, &[k_a]));

        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Down, t0))
            .is_empty());
        assert!(engine
            .on_event(make_event(
                k_b,
                KeyEdge::Down,
                t0 + Duration::from_millis(10)
            ))
            .is_empty());
        let res = engine.on_event(make_event(k_b, KeyEdge::Up, t0 + Duration::from_millis(40)));
        assert_single_chord(&res, k_a, k_b);

        assert!(engine
            .on_event(make_event(
                k_c,
                KeyEdge::Down,
                t0 + Duration::from_millis(50)
            ))
            .is_empty());
        let res = engine.on_event(make_event(k_c, KeyEdge::Up, t0 + Duration::from_millis(90)));
        assert_single_chord(&res, k_a, k_c);
    }

    #[test]
    fn test_char_continuous_case2_rollover_ab_then_ac() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.35, &[k_a]));

        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Down, t0))
            .is_empty());
        assert!(engine
            .on_event(make_event(
                k_b,
                KeyEdge::Down,
                t0 + Duration::from_millis(10)
            ))
            .is_empty());
        assert!(engine
            .on_event(make_event(
                k_c,
                KeyEdge::Down,
                t0 + Duration::from_millis(20)
            ))
            .is_empty());

        let res = engine.on_event(make_event(k_b, KeyEdge::Up, t0 + Duration::from_millis(50)));
        assert_single_chord(&res, k_a, k_b);

        let res = engine.on_event(make_event(k_c, KeyEdge::Up, t0 + Duration::from_millis(90)));
        assert_single_chord(&res, k_a, k_c);
    }

    #[test]
    fn test_char_continuous_case3_a_up_before_c_up_ratio_pass() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.5, &[k_a]));

        engine.on_event(make_event(k_a, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k_b,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        let res = engine.on_event(make_event(k_b, KeyEdge::Up, t0 + Duration::from_millis(40)));
        assert_single_chord(&res, k_a, k_b);

        assert!(engine
            .on_event(make_event(
                k_c,
                KeyEdge::Down,
                t0 + Duration::from_millis(50)
            ))
            .is_empty());
        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Up, t0 + Duration::from_millis(70)))
            .is_empty());

        let res = engine.on_event(make_event(k_c, KeyEdge::Up, t0 + Duration::from_millis(85)));
        assert_single_chord(&res, k_a, k_c);
    }

    #[test]
    fn test_char_continuous_case3_a_up_before_c_up_ratio_fail() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.5, &[k_a]));

        engine.on_event(make_event(k_a, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k_b,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        let res = engine.on_event(make_event(k_b, KeyEdge::Up, t0 + Duration::from_millis(40)));
        assert_single_chord(&res, k_a, k_b);

        assert!(engine
            .on_event(make_event(
                k_c,
                KeyEdge::Down,
                t0 + Duration::from_millis(50)
            ))
            .is_empty());
        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Up, t0 + Duration::from_millis(70)))
            .is_empty());

        let res = engine.on_event(make_event(
            k_c,
            KeyEdge::Up,
            t0 + Duration::from_millis(130),
        ));
        assert_eq!(res, vec![Decision::KeyTap(k_c)]);
    }

    #[test]
    fn test_char_continuous_case4_ab_judged_on_third_down_then_bc() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.6, &[k_a, k_b]));

        engine.on_event(make_event(k_a, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k_b,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Up, t0 + Duration::from_millis(40)))
            .is_empty());

        let res = engine.on_event(make_event(
            k_c,
            KeyEdge::Down,
            t0 + Duration::from_millis(50),
        ));
        assert_single_chord(&res, k_a, k_b);

        let res = engine.on_event(make_event(k_c, KeyEdge::Up, t0 + Duration::from_millis(90)));
        assert_single_chord(&res, k_b, k_c);
    }

    #[test]
    fn test_char_continuous_case4_ab_fail_then_a_tap_and_bc() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.8, &[k_a, k_b]));

        engine.on_event(make_event(k_a, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k_b,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Up, t0 + Duration::from_millis(40)))
            .is_empty());

        let res = engine.on_event(make_event(
            k_c,
            KeyEdge::Down,
            t0 + Duration::from_millis(50),
        ));
        assert_eq!(res, vec![Decision::KeyTap(k_a)]);

        let res = engine.on_event(make_event(k_c, KeyEdge::Up, t0 + Duration::from_millis(90)));
        assert_single_chord(&res, k_b, k_c);
    }

    #[test]
    fn test_char_continuous_case4_b_up_before_c_up_ratio_fail_for_bc() {
        let t0 = Instant::now();
        let k_a = make_key(0x1E);
        let k_b = make_key(0x20);
        let k_c = make_key(0x21);

        let mut engine = ChordEngine::new(continuous_char_profile(0.5, &[k_a, k_b]));

        engine.on_event(make_event(k_a, KeyEdge::Down, t0));
        engine.on_event(make_event(
            k_b,
            KeyEdge::Down,
            t0 + Duration::from_millis(10),
        ));
        assert!(engine
            .on_event(make_event(k_a, KeyEdge::Up, t0 + Duration::from_millis(40)))
            .is_empty());

        let res = engine.on_event(make_event(
            k_c,
            KeyEdge::Down,
            t0 + Duration::from_millis(50),
        ));
        assert_single_chord(&res, k_a, k_b);

        let res = engine.on_event(make_event(k_b, KeyEdge::Up, t0 + Duration::from_millis(60)));
        assert!(res.is_empty());

        let res = engine.on_event(make_event(
            k_c,
            KeyEdge::Up,
            t0 + Duration::from_millis(130),
        ));
        assert_eq!(res, vec![Decision::KeyTap(k_c)]);
    }
}
