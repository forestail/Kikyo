use crate::types::ScKey;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbKeys {
    pub left: HashSet<ScKey>,
    pub right: HashSet<ScKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessiveCfg {
    pub enabled: bool,
    // TODO: Add details
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub chord_style: ChordStyle,
    pub chord_window_ms: u64,
    // min_overlap_ms removed
    pub max_chord_size: usize,
    pub adaptive_window: AdaptiveCfg,
    pub thumb_keys: Option<ThumbKeys>,
    pub trigger_keys: HashMap<ScKey, PlaneTag>,
    pub target_keys: Option<HashSet<ScKey>>,
    pub successive: SuccessiveCfg,

    // New fields
    pub char_key_repeat_assigned: bool,
    pub char_key_repeat_unassigned: bool,

    pub ime_mode: ImeMode,

    pub thumb_shift_key_mode: ThumbShiftKeyMode,
    pub thumb_shift_continuous: bool,
    pub thumb_shift_single_press: ThumbShiftSinglePress,
    pub thumb_shift_repeat: bool,
    pub thumb_shift_overlap_ratio: f64,

    pub char_key_continuous: bool,
    pub char_key_overlap_ratio: f64, // Renamed from overlap_ratio_threshold
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            chord_style: ChordStyle::TriggerKey,
            chord_window_ms: 200,
            // min_overlap_ms: 50, // Removed
            max_chord_size: 2,
            adaptive_window: AdaptiveCfg { enabled: false },
            thumb_keys: None,
            trigger_keys: HashMap::new(),
            target_keys: None,
            successive: SuccessiveCfg { enabled: false },

            char_key_repeat_assigned: false,
            char_key_repeat_unassigned: true,

            ime_mode: ImeMode::Auto,

            thumb_shift_key_mode: ThumbShiftKeyMode::NonTransformTransform,
            thumb_shift_continuous: false,
            thumb_shift_single_press: ThumbShiftSinglePress::None,
            thumb_shift_repeat: false,
            thumb_shift_overlap_ratio: 0.35,

            char_key_continuous: false,
            char_key_overlap_ratio: 0.35, // Renamed from overlap_ratio_threshold
        }
    }
}

impl Profile {
    pub fn update_thumb_keys(&mut self) {
        let mut left = HashSet::new();
        let mut right = HashSet::new();

        // Scancodes
        let sc_muhenkan = ScKey::new(0x7B, false);
        let sc_henkan = ScKey::new(0x79, false);
        let sc_space = ScKey::new(0x39, false);

        match self.thumb_shift_key_mode {
            ThumbShiftKeyMode::NonTransformTransform => {
                left.insert(sc_muhenkan);
                right.insert(sc_henkan);
            }
            ThumbShiftKeyMode::NonTransformSpace => {
                left.insert(sc_muhenkan);
                right.insert(sc_space);
            }
            ThumbShiftKeyMode::SpaceTransform => {
                left.insert(sc_space);
                right.insert(sc_henkan);
            }
        }

        self.thumb_keys = Some(ThumbKeys { left, right });
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
                    output.extend(self.flush_all_pending());

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
                let chords = self.check_chords(now);
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
                let chords = self.check_chords(now);
                output.extend(chords);

                // 3. Flush Single Taps
                if self.state.pending.len() == 1 {
                    let p = &self.state.pending[0];
                    if p.t_up.is_some() {
                        // It's a lonely tap
                        let key = p.key;
                        let is_mod = self.is_modifier_key(key);

                        self.state.pending.clear();
                        self.state.down_ts.remove(&key);

                        if is_mod {
                            // Check if used
                            if self.state.used_modifiers.contains(&key) {
                                // Was used, so ignore single press
                                self.state.used_modifiers.remove(&key);
                            } else {
                                // Trigger Single Press Logic
                                match self.profile.thumb_shift_single_press {
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
                        } else {
                            output.push(Decision::KeyTap(key));
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

    fn check_chords(&mut self, now: Instant) -> Vec<Decision> {
        let mut output = Vec::new();
        if self.state.pending.len() < 2 {
            return output;
        }

        // Iterate all pairs in pending
        let mut consumed_indices = HashSet::new();
        let mut flushed_indices = HashSet::new(); // Keys decided as Sequential (Tap)

        // Use indices to avoid cloning
        for i in 0..self.state.pending.len() {
            if consumed_indices.contains(&i) || flushed_indices.contains(&i) {
                continue;
            }

            for j in (i + 1)..self.state.pending.len() {
                if consumed_indices.contains(&j) || flushed_indices.contains(&j) {
                    continue;
                }

                // Determine First (p1) and Second (p2) based on t_down
                let (idx1, idx2) = if self.state.pending[i].t_down <= self.state.pending[j].t_down {
                    (i, j)
                } else {
                    (j, i)
                };

                let p1 = &self.state.pending[idx1];
                let p2 = &self.state.pending[idx2];

                // 1. Time difference check using chord_window
                // REMOVED: chord_window check. We now only rely on overlap ratio.
                /*
                let t_diff = p2.t_down.duration_since(p1.t_down);
                if t_diff.as_millis() as u64 > self.profile.chord_window_ms {
                    continue;
                }
                */

                // 2. Overlap Ratio Check
                // We need p2 to be released (t_up known) to calculate ratio denominator.
                // Exception: if p2 is pressed, we can't determine ratio definitively generally.
                // BUT if p1 is still pressed, and p2 is pressed... Min(p1.up, p2.up) is unknown.
                // So we WAIT if p2 is pressed.

                if p2.t_up.is_none() {
                    // P2 still down. Wait.
                    continue;
                }

                // P2 is Up. P1 might be Up or Down.
                let p1_end = p1.t_up.unwrap_or(now);
                let p2_end = p2.t_up.unwrap(); // Known

                // Overlap = Intersection of [p1.down, p1_end] and [p2.down, p2_end]
                // Since p1.down <= p2.down, Intersection start is p2.down.
                // Intersection end is min(p1_end, p2_end).

                let overlap_start = p2.t_down;
                let overlap_end = if p1_end < p2_end { p1_end } else { p2_end };

                let overlap_dur = if overlap_end > overlap_start {
                    overlap_end.duration_since(overlap_start)
                } else {
                    Duration::ZERO
                };

                let p2_dur = p2_end.duration_since(p2.t_down);

                // Avoid division by zero (should be rare/impossible for real key press)
                let ratio = if p2_dur.as_micros() > 0 {
                    overlap_dur.as_secs_f64() / p2_dur.as_secs_f64()
                } else {
                    0.0
                };

                if ratio >= self.profile.char_key_overlap_ratio {
                    // CHORD!
                    let k1 = p1.key;
                    let k2 = p2.key;
                    let is_mod1 = self.is_modifier_key(k1);
                    let is_mod2 = self.is_modifier_key(k2);

                    if is_mod1 {
                        self.state.used_modifiers.insert(k1);
                    }
                    if is_mod2 {
                        self.state.used_modifiers.insert(k2);
                    }

                    // Consume keys unless continuous shift is enabled for a modifier
                    if !is_mod1 || !self.profile.thumb_shift_continuous {
                        consumed_indices.insert(idx1);
                    }
                    if !is_mod2 || !self.profile.thumb_shift_continuous {
                        consumed_indices.insert(idx2);
                    }

                    // Output Chord
                    output.push(Decision::Chord(vec![k1, k2]));

                    // If idx1 was consumed, we must move to next i.
                    // If idx1 was NOT consumed (continuous modifier), we continue checking i against other js.
                    if consumed_indices.contains(&idx1) {
                        break;
                    }
                    // If not consumed, continue inner loop to find more chords for this modifier
                } else {
                    // SEQUENTIAL!
                    // If ratio is low, it means they are effectively sequential.
                    // A(Down)->A(Up)->B(Down) or partial overlap failure.
                    // We should flush P1 as Tap.
                    // P2 remains pending (might chord with next).
                    // BUT: If P1 is flushed, we must mark it.
                    flushed_indices.insert(idx1);
                    output.push(Decision::KeyTap(p1.key));

                    // We do NOT break here, because p1 is now flushed, we continue outer loop?
                    // Actually if p1 is flushed, we shouldn't continue checking p1 against others.
                    break; // Move to next i (which will skip because p1 is flushed)
                }
            }
        }

        // Remove consumed or flushed
        if !consumed_indices.is_empty() || !flushed_indices.is_empty() {
            let mut new_pending = Vec::new();
            for (i, p) in self.state.pending.iter().enumerate() {
                if consumed_indices.contains(&i) {
                    // Consumed by chord
                    if !self.state.pressed.contains(&p.key) {
                        self.state.down_ts.remove(&p.key);
                    }
                    continue;
                }
                if flushed_indices.contains(&i) {
                    // Flushed as Tap
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

    fn is_modifier_key(&self, key: ScKey) -> bool {
        if let Some(ref tk) = self.profile.thumb_keys {
            if tk.left.contains(&key) || tk.right.contains(&key) {
                return true;
            }
        }
        false
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
}
