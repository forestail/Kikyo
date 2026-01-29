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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessiveCfg {
    pub enabled: bool,
    // TODO: Add details
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub chord_style: ChordStyle,
    pub chord_window_ms: u64,
    pub min_overlap_ms: u64,
    pub max_chord_size: usize,
    pub adaptive_window: AdaptiveCfg,
    pub thumb_keys: Option<ThumbKeys>,
    pub trigger_keys: HashMap<ScKey, PlaneTag>,
    pub target_keys: Option<HashSet<ScKey>>,
    pub successive: SuccessiveCfg,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            chord_style: ChordStyle::TriggerKey,
            chord_window_ms: 200,
            min_overlap_ms: 50,
            max_chord_size: 2,
            adaptive_window: AdaptiveCfg { enabled: false },
            thumb_keys: None,
            trigger_keys: HashMap::new(),
            target_keys: None,
            successive: SuccessiveCfg { enabled: false },
        }
    }
}

pub struct ChordState {
    pub enabled: bool,
    pub pressed: HashSet<ScKey>,
    pub down_ts: HashMap<ScKey, Instant>,
    pub pending: Vec<PendingKey>,
    pub latch: LatchState,
    // pub stats: Stats,
}

#[derive(Debug, Clone)]
pub struct PendingKey {
    pub key: ScKey,
    pub t_down: Instant,
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
            state: ChordState {
                enabled: true,
                pressed: HashSet::new(),
                down_ts: HashMap::new(),
                pending: Vec::new(),
                latch: LatchState::None,
            },
        }
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
    }

    pub fn on_event(&mut self, event: KeyEvent) -> Vec<Decision> {
        if event.injected {
            return vec![];
        }

        // 0. Filter non-target keys (if whitelist is active)
        if let Some(ref targets) = self.profile.target_keys {
            if !targets.contains(&event.key) {
                // Not in target list -> Pass through immediately
                return vec![Decision::Passthrough(event.key, event.edge)];
            }
        }

        let now = event.t;
        let mut output = Vec::new();

        match event.edge {
            KeyEdge::Down => {
                // 1. Update pressed state
                self.state.pressed.insert(event.key);
                self.state.down_ts.insert(event.key, now);

                // 2. Add to pending
                // Avoid duplicates (if repeat comes in)
                if !self.state.pending.iter().any(|p| p.key == event.key) {
                    self.state.pending.push(PendingKey {
                        key: event.key,
                        t_down: now,
                    });
                }

                // 3. Try to form a chord (if conditions met immediately, e.g. min_overlap=0)
                // Even if min_overlap > 0, we check.
                // If checking on Down, overlap with current key is 0.
                if self.profile.min_overlap_ms == 0 {
                    let chords = self.check_chords(now);
                    output.extend(chords);
                }
            }
            KeyEdge::Up => {
                // 1. Check for chord formation BEFORE removing from pressed
                // This allows catching the overlap at the moment of release.
                let chords = self.check_chords(now);
                output.extend(chords);

                // 2. Update state
                self.state.pressed.remove(&event.key);

                // 3. Check for Tap (Trigger/Latch) candidates
                // If the key is still pending (not consumed by chord)
                let is_pending = self.state.pending.iter().any(|p| p.key == event.key);
                if is_pending {
                    if self.is_modifier_key(event.key) {
                        // Modifier Tap -> Latch
                        // Remove from pending to consume it
                        self.state.pending.retain(|p| p.key != event.key);
                        output.push(Decision::LatchOn(LatchKind::OneShot));
                        // Clean down_ts
                        self.state.down_ts.remove(&event.key);
                    } else {
                        // Normal key tap (released without chord)
                        // It was pending (waiting for chord), but released alone.
                        // So it is a Tap.
                        self.state.pending.retain(|p| p.key != event.key);
                        output.push(Decision::KeyTap(event.key));
                        self.state.down_ts.remove(&event.key);
                    }
                }

                // Cleanup down_ts if no longer pending
                // (Already handled above for the event.key, but safety check)
                if !self.state.pending.iter().any(|p| p.key == event.key) {
                    self.state.down_ts.remove(&event.key);
                }
            }
        }

        // Cycle flush to handle timeouts
        output.extend(self.flush_expired(now));

        output
    }

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

    fn check_chords(&mut self, now: Instant) -> Vec<Decision> {
        let mut output = Vec::new();
        if self.state.pending.len() < 2 {
            return output;
        }

        // Simple MVP: Check pairs (2-key chords)
        // Iterate all pairs in pending
        let mut consumed_indices = HashSet::new();

        // Use indices to avoid cloning
        // Pending is usually small (1-3 keys).
        for i in 0..self.state.pending.len() {
            if consumed_indices.contains(&i) {
                continue;
            }

            for j in (i + 1)..self.state.pending.len() {
                if consumed_indices.contains(&j) {
                    continue;
                }

                let p1 = &self.state.pending[i];
                let p2 = &self.state.pending[j];

                // 1. Time difference check (chord window)
                let t_diff = if p1.t_down > p2.t_down {
                    p1.t_down.duration_since(p2.t_down)
                } else {
                    p2.t_down.duration_since(p1.t_down)
                };

                if t_diff.as_millis() as u64 > self.profile.chord_window_ms {
                    continue;
                }

                // 2. Overlap check
                // Interval 1: [down1, up1_or_now]
                // Interval 2: [down2, up2_or_now]
                // Up time is now if pressed, else... we don't track Up time in PendingKey!
                // We track down_ts in state.down_ts.
                // If it is in pressed, use now.
                // If NOT in pressed, we need its Up time.
                // Ah, we don't store Up time in PendingKey.
                // Issue: If A was pressed and released (pending), and B is pressed.
                // We need A's release time to calculate overlap.
                // We must store 't_up' or 't_last_associated_event' in pending?
                // Or we can't strict verify overlap if we threw away Up time.
                // REFACTOR: PendingKey should store Key state or we rely on 'state.down_ts' strictly?
                // But 'down_ts' is removed on Up in some logic?
                // No, my implementation of Up kept down_ts IF pending.
                // So down_ts has the Down time. But we need Up time.
                // If it's NOT in 'pressed', it means it is Up.
                // But WHEN did it go Up? 'now'? No, it went up earlier.
                // We missed the Up timestamp!
                // FIX: We need to store 't_up' in PendingKey if it's released but pending.

                // For MVP, if a key is released, we assume it ended "recently" or check against specific event?
                // If A Up happened 100ms ago, and we are processing B Up.
                // The overlap might be 0.
                // We need strict timestamps.

                // Let's assume we proceed for now and add 't_up' to PendingKey later?
                // Or just use 'now' if pressed, and 'p.t_down' (0 overlap) if up?
                // That would fail "Overlap required".

                // CRITICAL FIX: To support "released but pending", we need `t_up` in PendingKey.
                // I will add `t_up: Option<Instant>` to PendingKey definition later.
                // For this step, I will assume keys must be pressed to overlap (intersection with now),
                // OR purely rely on window check if overlap is disabled.
                // Since "Overlap required" is a goal, I should fix PendingKey.

                // Skip overlap check for this iteration since I can't enforce it without t_up.
                // I'll add a TODO and assume sufficient overlap for now or rely on "Both Pressed".
                let both_pressed =
                    self.state.pressed.contains(&p1.key) && self.state.pressed.contains(&p2.key);
                if self.profile.min_overlap_ms > 0 {
                    if !both_pressed {
                        // If one is released, we can't verify overlap without t_up.
                        // Fail safe: don't chord.
                        continue;
                    }
                    // If both pressed, overlap is [max(d1, d2), now].
                    let start = if p1.t_down > p2.t_down {
                        p1.t_down
                    } else {
                        p2.t_down
                    };
                    if (now.duration_since(start).as_millis() as u64) < self.profile.min_overlap_ms
                    {
                        continue;
                    }
                }

                // Match!
                consumed_indices.insert(i);
                consumed_indices.insert(j);
                output.push(Decision::Chord(vec![p1.key, p2.key]));
                break; // Move to next i
            }
        }

        // Remove consumed
        if !consumed_indices.is_empty() {
            let mut new_pending = Vec::new();
            for (i, p) in self.state.pending.iter().enumerate() {
                if !consumed_indices.contains(&i) {
                    new_pending.push(p.clone());
                } else {
                    // Consumed. Clean down_ts if not pressed?
                    // Usually yes.
                    if !self.state.pressed.contains(&p.key) {
                        self.state.down_ts.remove(&p.key);
                    }
                }
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
    fn test_basic_chord_on_release_overlap() {
        let mut profile = Profile::default();
        profile.chord_window_ms = 50;
        profile.min_overlap_ms = 5; // Require 5ms overlap

        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        // 1. Down A at t0
        let res = engine.on_event(make_event(k1, KeyEdge::Down, t0));
        assert!(res.is_empty(), "Should pending");

        // 2. Down B at t0 + 10ms (Within window)
        let t1 = t0 + Duration::from_millis(10);
        let res = engine.on_event(make_event(k2, KeyEdge::Down, t1));
        assert!(
            res.is_empty(),
            "Should pending (overlap not met on down edge)"
        );

        // 3. Up A at t0 + 20ms.
        // Overlap duration: A is [0..20], B is [10..20]. Overlap = 10ms >= 5ms.
        // Match!
        let t2 = t0 + Duration::from_millis(20);
        let res = engine.on_event(make_event(k1, KeyEdge::Up, t2));

        assert_eq!(res.len(), 1);
        if let Decision::Chord(keys) = &res[0] {
            assert_eq!(keys.len(), 2);
            assert!(keys.contains(&k1));
            assert!(keys.contains(&k2));
        } else {
            panic!("Expected Chord, got {:?}", res);
        }
    }

    #[test]
    fn test_sequential_flush() {
        let mut profile = Profile::default();
        profile.chord_window_ms = 50;
        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();
        let k1 = make_key(0x1E); // A
        let k2 = make_key(0x30); // B

        // 1. Down A
        engine.on_event(make_event(k1, KeyEdge::Down, t0));

        // 2. Down B at t0 + 100ms (Outside window)
        // This 'on_event' calls flush_expired FIRST?
        // No, current logic calls flush AT END of on_event.
        // So pending B is added. A is expired.
        // flush checks A (t0). now is t0+100. Diff=100 > 50. Expired.
        // So A becomes KeyTap. B remains pending.
        let t1 = t0 + Duration::from_millis(100);
        let res = engine.on_event(make_event(k2, KeyEdge::Down, t1));

        // Check A tap
        let has_tap_a = res
            .iter()
            .any(|d| matches!(d, Decision::KeyTap(k) if *k == k1));
        assert!(has_tap_a, "Should flush A as Tap");

        // B should NOT be output yet
        let has_b = res.iter().any(|d| match d {
            Decision::KeyTap(k) => *k == k2,
            Decision::Passthrough(k, _) => *k == k2,
            _ => false,
        });
        assert!(!has_b, "B should be pending");
    }

    #[test]
    fn test_trigger_tap() {
        let mut profile = Profile::default();
        profile.chord_window_ms = 50;
        // Make K1 a modifier (Trigger Logic via ThumbKeys or Modifier check)
        // The implementation uses `is_modifier_key`.
        // Let's set up ThumbKeys.
        let k1 = make_key(0x39); // Space
        let mut tk = std::collections::HashSet::new();
        tk.insert(k1);
        profile.thumb_keys = Some(ThumbKeys {
            left: tk.clone(),
            right: std::collections::HashSet::new(),
        });

        let mut engine = ChordEngine::new(profile);
        let t0 = Instant::now();

        // 1. Down Space
        engine.on_event(make_event(k1, KeyEdge::Down, t0));

        // 2. Up Space at t0 + 20ms (Short tap).
        // Should become LatchOn(OneShot)?
        let t1 = t0 + Duration::from_millis(20);
        let res = engine.on_event(make_event(k1, KeyEdge::Up, t1));

        assert_eq!(res.len(), 1);
        match res[0] {
            Decision::LatchOn(LatchKind::OneShot) => {}
            _ => panic!("Expected LatchOn, got {:?}", res),
        }
    }
}
