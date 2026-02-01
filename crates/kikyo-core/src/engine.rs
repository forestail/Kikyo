use crate::chord_engine::{ChordEngine, Decision, ImeMode, KeyEdge, KeyEvent, Profile};
use crate::types::{InputEvent, KeyAction, KeySpec, KeyStroke, Layout, Modifiers, ScKey, Token};
use crate::JIS_SC_TO_RC;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::debug;
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC_EX};

lazy_static::lazy_static! {
    pub static ref ENGINE: Mutex<Engine> = Mutex::new(Engine::default());
}

pub struct Engine {
    chord_engine: ChordEngine,
    enabled: bool,
    layout: Option<Layout>,
    on_enabled_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    repeat_plans: HashMap<ScKey, Vec<ScKey>>,
}

impl Default for Engine {
    fn default() -> Self {
        let mut profile = Profile::default();
        profile.update_thumb_keys();
        Self {
            chord_engine: ChordEngine::new(profile),
            enabled: true,
            layout: None,
            on_enabled_change: None,
            repeat_plans: HashMap::new(),
        }
    }
}

impl Engine {
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                // Reset state without discarding the user's profile.
                let profile = self.chord_engine.profile.clone();
                self.chord_engine = ChordEngine::new(profile);
                self.repeat_plans.clear();
            }
            if let Some(ref cb) = self.on_enabled_change {
                cb(enabled);
            }
        }
    }

    pub fn set_on_enabled_change(&mut self, cb: impl Fn(bool) + Send + Sync + 'static) {
        self.on_enabled_change = Some(Box::new(cb));
    }

    pub fn set_ignore_ime(&mut self, ignore: bool) {
        self.chord_engine.profile.ime_mode = if ignore {
            ImeMode::Ignore
        } else {
            ImeMode::Auto
        };
    }

    pub fn set_ime_mode(&mut self, mode: ImeMode) {
        self.chord_engine.profile.ime_mode = mode;
    }

    pub fn get_ime_mode(&self) -> ImeMode {
        self.chord_engine.profile.ime_mode
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn get_layout_name(&self) -> Option<String> {
        self.layout.as_ref().and_then(|l| l.name.clone())
    }

    pub fn get_profile(&self) -> Profile {
        self.chord_engine.profile.clone()
    }

    pub fn get_suspend_key(&self) -> crate::chord_engine::SuspendKey {
        self.chord_engine.profile.suspend_key
    }

    fn has_thumb_shift_sections_in_layout(&self) -> bool {
        if let Some(ref layout) = self.layout {
            let targets = [
                "ローマ字左親指シフト",
                "ローマ字右親指シフト",
                "英数左親指シフト",
                "英数右親指シフト",
            ];
            for t in &targets {
                if layout.sections.keys().any(|k| k.starts_with(t)) {
                    return true;
                }
            }
        }
        false
    }

    pub fn set_profile(&mut self, mut profile: Profile) {
        // Update thumb keys based on mode
        profile.update_thumb_keys();

        // Pattern 1: If layout does not have thumb shift sections, disable thumb keys.
        // This ensures they act as normal keys if the layout doesn't support thumb shift.
        if self.layout.is_some() && !self.has_thumb_shift_sections_in_layout() {
            profile.thumb_keys = None;
        }

        // Preserve layout-derived data if missing in new profile
        let current = &self.chord_engine.profile;
        if profile.target_keys.is_none() && current.target_keys.is_some() {
            profile.target_keys = current.target_keys.clone();
        }
        if profile.trigger_keys.is_empty() && !current.trigger_keys.is_empty() {
            profile.trigger_keys = current.trigger_keys.clone();
        }

        // Ensure new thumb keys are in target list
        if let Some(ref mut targets) = profile.target_keys {
            if let Some(ref tk) = profile.thumb_keys {
                targets.extend(tk.left.iter());
                targets.extend(tk.right.iter());
            }
        }

        self.chord_engine.set_profile(profile);
    }

    pub fn load_layout(&mut self, layout: Layout) {
        tracing::info!(
            "Engine: Layout loaded with {} sections.",
            layout.sections.len()
        );

        let mut profile = self.chord_engine.profile.clone();

        // 1. Collect all definition RCs from layout
        let mut active_rcs = HashSet::new();
        for section in layout.sections.values() {
            // Base plane
            for (rc, token) in &section.base_plane.map {
                if !matches!(token, Token::None) {
                    active_rcs.insert(rc);
                }
            }
            // Sub planes
            for sub in section.sub_planes.values() {
                for (rc, token) in &sub.map {
                    if !matches!(token, Token::None) {
                        active_rcs.insert(rc);
                    }
                }
            }
        }

        // 2. Map RCs back to ScKeys
        // Brute-force reverse mapping from JIS_SC_TO_RC
        let mut target_keys = HashSet::new();
        for (sc, rc) in JIS_SC_TO_RC.iter() {
            if active_rcs.contains(rc) {
                target_keys.insert(*sc);
            }
        }

        profile.trigger_keys.clear();

        // MVP: Detect trigger keys from "<...>" sections and sub-planes.
        for (name, section) in layout.sections.iter() {
            tracing::info!(" - Section: {}", name);
            if name.starts_with('<') && name.ends_with('>') {
                let inner = &name[1..name.len() - 1];
                if let Some(sc) = crate::jis_map::key_name_to_sc(inner) {
                    let key = ScKey::new(sc, false);
                    if !profile.trigger_keys.contains_key(&key) {
                        profile.trigger_keys.insert(key, name.clone());
                        tracing::info!("   -> Registered TriggerKey: {} (sc={:02X})", name, sc);
                    }
                    target_keys.insert(key);
                }
            }

            for tag in section.sub_planes.keys() {
                if tag.starts_with('<') && tag.ends_with('>') {
                    let inner = &tag[1..tag.len() - 1];
                    if let Some(sc) = crate::jis_map::key_name_to_sc(inner) {
                        let key = ScKey::new(sc, false);
                        if !profile.trigger_keys.contains_key(&key) {
                            profile.trigger_keys.insert(key, tag.clone());
                            tracing::info!(
                                "   -> Registered TriggerKey: {} (sc={:02X})",
                                tag,
                                sc
                            );
                        }
                        target_keys.insert(key);
                    }
                }
            }
        }

        // Add thumb keys if any (currently handled via profile manually or elsewhere, but let's ensure)
        if let Some(ref tk) = profile.thumb_keys {
            target_keys.extend(tk.left.iter());
            target_keys.extend(tk.right.iter());
        }

        profile.target_keys = Some(target_keys);

        // Update layout FIRST so set_profile can check it
        self.layout = Some(layout);
        // Then set profile (processes logic to disable thumb keys if needed)
        self.set_profile(profile);
    }

    pub fn process_key(&mut self, sc: u16, ext: bool, up: bool, shift: bool) -> KeyAction {
        if !self.enabled {
            return KeyAction::Pass;
        }

        // Check IME state
        let is_japanese = crate::ime::is_japanese_input_active(self.chord_engine.profile.ime_mode);
        // Note: previous logic had early return if !ime_on.
        // Now if !ime_on (meaning Not Japanese Input), we use is_japanese=false -> [英数...] sections.
        // However, if IME is effectively disabled/closed, logic is similar to "英数" mode.
        // But we must also ensure we don't block keys if we shouldn't hook?
        // Requirement says "relevant definition ... -> hook". If "definition missing -> no hook".
        // So checking for section existence in resolve() handles the "no hook" case.
        // But existing ime_on check also handled "Don't run ANY logic if IME off".
        // The new requirement implies we DO run logic even if IME off, specifically for [英数...] sections.
        // So we remove the early return.

        if self.layout.is_none() {
            return KeyAction::Pass;
        }

        let key = ScKey::new(sc, ext);

        if !up && self.is_repeat_event(key) {
            return self.handle_repeat_event(key, shift, is_japanese);
        }

        // Pre-check: Verify if the key is defined in the current section.
        // If not, we pass immediately to avoid ChordEngine buffering.
        {
            // 1. Determine local "Thumb Shift" status from ChordEngine state
            let mut has_left_thumb = false;
            let mut has_right_thumb = false;
            if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                for k in &self.chord_engine.state.pressed {
                    if tk.left.contains(k) {
                        has_left_thumb = true;
                    }
                    if tk.right.contains(k) {
                        has_right_thumb = true;
                    }
                }
            }

            // 2. Select PREFIX & SUFFIX
            let prefix = if is_japanese {
                "ローマ字"
            } else {
                "英数"
            };
            let suffix = if shift {
                if has_left_thumb {
                    "小指左親指シフト"
                } else if has_right_thumb {
                    "小指右親指シフト"
                } else {
                    "小指シフト"
                }
            } else {
                if has_left_thumb {
                    "左親指シフト"
                } else if has_right_thumb {
                    "右親指シフト"
                } else {
                    "シフト無し"
                }
            };

            let section_name = format!("{}{}", prefix, suffix);

            // Debug logging (temporary) - output only if not blocked to avoid spam?
            // Better to output for specific keys or always for debugging.
            if key.sc == 0x1E { // Limit to 'A' key or similar if spamming, but for now log all resolved attempts
                 // tracing::info!("Resolve: IME={} Section={} Key={}", is_japanese, section_name, sc);
            }
            // tracing::info!(
            //     "Resolve: IME={} Section={} Key={:02X}",
            //     is_japanese,
            //     section_name,
            //     sc
            // );

            // 3. Check Section Existence
            if let Some(layout) = &self.layout {
                let is_space = key.sc == 0x39;
                let mut is_thumb = false;
                if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                    if tk.left.contains(&key) || tk.right.contains(&key) {
                        is_thumb = true;
                    }
                }

                if let Some(section) = layout.sections.get(&section_name) {
                    // Section exists. Check if key is defined.
                    let mut is_defined = false;

                    // Check Base Plane
                    if let Some(rc) = self.key_to_rc(key) {
                        if let Some(token) = section.base_plane.map.get(&rc) {
                            if !matches!(token, Token::None) {
                                is_defined = true;
                            }
                        }
                    }

                    // Check Trigger Keys (Sub Planes)
                    if !is_defined {
                        if let Some(name) = crate::jis_map::sc_to_key_name(sc) {
                            let tag = format!("<{}>", name);
                            if section.sub_planes.contains_key(&tag) {
                                is_defined = true;
                            }
                        }
                    }

                    if !is_defined && !is_thumb && !is_space {
                        // Defined section, but key is not in it -> Pass
                        return KeyAction::Pass;
                    }
                } else {
                    // Section does NOT exist -> Pass
                    // UNLESS it is a Thumb Key
                    if !is_thumb && !is_space {
                        return KeyAction::Pass;
                    }
                }
            }
        }

        let event = KeyEvent {
            key,
            edge: if up { KeyEdge::Up } else { KeyEdge::Down },
            injected: false,
            t: Instant::now(),
        };

        let decisions = self.chord_engine.on_event(event);

        let mut inject_ops = Vec::new();
        let mut pass_current = false;

        for d in decisions {
            match d {
                Decision::Passthrough(k, _) => {
                    if k == key {
                        pass_current = true;
                    }
                }
                Decision::KeyTap(k) => {
                    if self.repeat_plans.contains_key(&k) {
                        continue;
                    }
                    if let Some(token) = self.resolve(&[k], shift, is_japanese) {
                        if let Some(ops) = self.token_to_events(&token, shift) {
                            inject_ops.extend(ops);
                        }
                    } else {
                        // Replay unmapped or failed resolution as original key
                        inject_ops.push(InputEvent::Scancode(k.sc, k.ext, false)); // Down
                        inject_ops.push(InputEvent::Scancode(k.sc, k.ext, true));
                        // Up
                    }
                }
                Decision::Chord(keys) => {
                    if let Some(token) = self.resolve(&keys, shift, is_japanese) {
                        if let Some(ops) = self.token_to_events(&token, shift) {
                            inject_ops.extend(ops);
                        }
                    } else {
                        // Fallback: undefined chord -> treat as sequential inputs
                        for k in keys {
                            // Try to resolve as single key (unshifted)
                            let mut resolved = false;
                            if let Some(token) = self.resolve(&[k], shift, is_japanese) {
                                if let Some(ops) = self.token_to_events(&token, shift) {
                                    inject_ops.extend(ops);
                                    resolved = true;
                                }
                            }

                            if !resolved {
                                // Ultimate fallback: raw scancode
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, false)); // Down
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, true));
                                // Up
                            }
                        }
                    }
                }
                Decision::LatchOn(kind) => {
                    debug!("LatchOn: {:?}", kind);
                }
                Decision::LatchOff => {
                    debug!("LatchOff");
                }
            }
        }

        if up {
            self.repeat_plans.remove(&key);
        }

        if !inject_ops.is_empty() {
            if pass_current {
                // If we also need to pass the current key, append it to the injection sequence.
                // This ensures "Flushed Keys" -> "Current Key" order.
                inject_ops.push(InputEvent::Scancode(sc, ext, up));
            }
            return KeyAction::Inject(inject_ops);
        }

        if pass_current {
            return KeyAction::Pass;
        }

        KeyAction::Block
    }

    fn resolve(&self, keys: &[ScKey], shift: bool, is_japanese: bool) -> Option<Token> {
        let layout = self.layout.as_ref()?;

        // 1. Determine "Thumb Shift" status
        let mut has_left_thumb = false;
        let mut has_right_thumb = false;

        if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
            for k in keys {
                if tk.left.contains(k) {
                    has_left_thumb = true;
                }
                if tk.right.contains(k) {
                    has_right_thumb = true;
                }
            }
        }

        // 2. Select PREFIX (Eng vs Roma)
        let prefix = if is_japanese {
            "ローマ字"
        } else {
            "英数"
        };

        // 3. Select SUFFIX
        let suffix = if shift {
            if has_left_thumb {
                "小指左親指シフト"
            } else if has_right_thumb {
                "小指右親指シフト"
            } else {
                "小指シフト"
            }
        } else {
            if has_left_thumb {
                "左親指シフト"
            } else if has_right_thumb {
                "右親指シフト"
            } else {
                "シフト無し"
            }
        };

        let section_name = format!("{}{}", prefix, suffix);
        // tracing::info!("Resolve: section={} keys={:?}", section_name, keys);

        let section = layout.sections.get(&section_name)?;

        // 4. Update keys for lookup (Remove Thumb Modifiers)
        let lookup_keys: Vec<ScKey> = if has_left_thumb || has_right_thumb {
            if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                keys.iter()
                    .filter(|&&k| {
                        let is_left = tk.left.contains(&k);
                        let is_right = tk.right.contains(&k);
                        if has_left_thumb && is_left {
                            return false;
                        }
                        if has_right_thumb && is_right {
                            return false;
                        }
                        true
                    })
                    .cloned()
                    .collect()
            } else {
                keys.to_vec()
            }
        } else {
            keys.to_vec()
        };

        if lookup_keys.is_empty() {
            return None;
        }

        if lookup_keys.len() == 1 {
            let key = lookup_keys[0];
            let latch = &self.chord_engine.state.latch;

            if let crate::chord_engine::LatchState::OneShot(tag)
            | crate::chord_engine::LatchState::Lock(tag) = latch
            {
                if let Some(sub) = section.sub_planes.get(tag) {
                    if let Some(rc) = self.key_to_rc(key) {
                        if let Some(token) = sub.map.get(&rc) {
                            return Some(token.clone());
                        }
                    }
                }
            }

            if let Some(rc) = self.key_to_rc(key) {
                return section.base_plane.map.get(&rc).cloned();
            }
        } else if lookup_keys.len() == 2 {
            let k1 = lookup_keys[0];
            let k2 = lookup_keys[1];

            if let Some(token) = self.try_resolve_modifier(section, k1, k2) {
                return Some(token);
            }
            if let Some(token) = self.try_resolve_modifier(section, k2, k1) {
                return Some(token);
            }
        }

        None
    }

    fn try_resolve_modifier(
        &self,
        section: &crate::types::Section,
        mod_key: ScKey,
        target_key: ScKey,
    ) -> Option<Token> {
        let mod_name = crate::jis_map::sc_to_key_name(mod_key.sc)?;
        let tag = format!("<{}>", mod_name);
        if let Some(sub) = section.sub_planes.get(&tag) {
            if let Some(rc) = self.key_to_rc(target_key) {
                return sub.map.get(&rc).cloned();
            }
        }
        None
    }

    fn key_to_rc(&self, key: ScKey) -> Option<crate::types::Rc> {
        JIS_SC_TO_RC
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, rc)| *rc)
    }

    fn token_to_events(&self, token: &Token, shift_held: bool) -> Option<Vec<InputEvent>> {
        match token {
            Token::None => None,
            Token::KeySequence(seq) => {
                let mut events = Vec::new();
                for stroke in seq {
                    append_keystroke_events(&mut events, stroke, shift_held);
                }
                if events.is_empty() {
                    None
                } else {
                    Some(events)
                }
            }
            Token::ImeChar(text) | Token::DirectChar(text) => {
                let mut events = Vec::new();
                for c in text.chars() {
                    events.push(InputEvent::Unicode(c, false));
                    events.push(InputEvent::Unicode(c, true));
                }
                if events.is_empty() {
                    None
                } else {
                    Some(events)
                }
            }
        }
    }

    fn is_repeat_event(&self, key: ScKey) -> bool {
        self.chord_engine.state.pressed.contains(&key)
    }

    fn handle_repeat_event(&mut self, key: ScKey, shift: bool, is_japanese: bool) -> KeyAction {
        let now = Instant::now();
        let (keys, consume_pending) = if let Some(keys) = self.repeat_plans.get(&key) {
            (keys.clone(), false)
        } else {
            self.compute_repeat_plan(key, now)
        };

        let token = self.resolve(&keys, shift, is_japanese);
        let allow_repeat = self.repeat_allowed_for_token(token.as_ref());
        if !allow_repeat {
            return KeyAction::Block;
        }

        let events = if let Some(token) = token {
            self.token_to_events(&token, shift)
                .unwrap_or_else(|| self.repeat_fallback_events(&keys, shift, is_japanese))
        } else {
            self.repeat_fallback_events(&keys, shift, is_japanese)
        };

        if events.is_empty() {
            return KeyAction::Block;
        }

        if consume_pending {
            self.consume_pending_for_repeat(&keys);
        }
        self.repeat_plans.entry(key).or_insert(keys);
        KeyAction::Inject(events)
    }

    fn compute_repeat_plan(&self, key: ScKey, now: Instant) -> (Vec<ScKey>, bool) {
        let (mut keys, consume_pending) = if let Some(chord_keys) = self.detect_repeat_chord(key, now)
        {
            (chord_keys, true)
        } else {
            (self.repeat_single_keys(key), false)
        };

        if keys.is_empty() {
            keys.push(key);
        }

        (keys, consume_pending)
    }

    fn repeat_single_keys(&self, key: ScKey) -> Vec<ScKey> {
        let mut keys = vec![key];
        if self.is_thumb_key(key) {
            return keys;
        }

        if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
            let left = tk
                .left
                .iter()
                .find(|k| self.is_active_thumb_key(**k));
            let right = tk
                .right
                .iter()
                .find(|k| self.is_active_thumb_key(**k));

            if let Some(k) = left.or(right) {
                keys.push(*k);
            }
        }

        keys
    }

    fn detect_repeat_chord(&self, key: ScKey, now: Instant) -> Option<Vec<ScKey>> {
        let pending = &self.chord_engine.state.pending;
        if pending.len() < 2 {
            return None;
        }

        let primary = pending.iter().find(|p| p.key == key)?;
        let mut best_ratio = 0.0;
        let mut best_key = None;
        let threshold = self.chord_engine.profile.char_key_overlap_ratio;

        for other in pending.iter() {
            if other.key == key {
                continue;
            }

            let (p1, p2) = if primary.t_down <= other.t_down {
                (primary, other)
            } else {
                (other, primary)
            };

            let ratio = Self::pending_overlap_ratio(p1, p2, now);
            if ratio >= threshold && (best_key.is_none() || ratio > best_ratio) {
                best_ratio = ratio;
                best_key = Some(other.key);
            }
        }

        best_key.map(|other_key| vec![key, other_key])
    }

    fn pending_overlap_ratio(
        p1: &crate::chord_engine::PendingKey,
        p2: &crate::chord_engine::PendingKey,
        now: Instant,
    ) -> f64 {
        let p1_end = p1.t_up.unwrap_or(now);
        let p2_end = p2.t_up.unwrap_or(now);
        if p2_end <= p2.t_down {
            return 0.0;
        }

        let overlap_start = p2.t_down;
        let overlap_end = if p1_end < p2_end { p1_end } else { p2_end };
        let overlap_dur = if overlap_end > overlap_start {
            overlap_end.duration_since(overlap_start)
        } else {
            Duration::ZERO
        };

        let p2_dur = p2_end.duration_since(p2.t_down);
        if p2_dur.as_micros() == 0 {
            return 0.0;
        }
        overlap_dur.as_secs_f64() / p2_dur.as_secs_f64()
    }

    fn consume_pending_for_repeat(&mut self, keys: &[ScKey]) {
        if keys.len() < 2 {
            return;
        }

        let mut remove = HashSet::new();
        for k in keys {
            remove.insert(*k);
        }

        let mut new_pending = Vec::new();
        for p in self.chord_engine.state.pending.iter() {
            if remove.contains(&p.key) {
                if !self.chord_engine.state.pressed.contains(&p.key) {
                    self.chord_engine.state.down_ts.remove(&p.key);
                }
                continue;
            }
            new_pending.push(p.clone());
        }
        self.chord_engine.state.pending = new_pending;
    }

    fn is_thumb_key(&self, key: ScKey) -> bool {
        if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
            return tk.left.contains(&key) || tk.right.contains(&key);
        }
        false
    }

    fn is_active_thumb_key(&self, key: ScKey) -> bool {
        if !self.chord_engine.state.pressed.contains(&key) {
            return false;
        }
        self.chord_engine
            .state
            .pending
            .iter()
            .any(|p| p.key == key)
    }

    fn repeat_allowed_for_token(&self, token: Option<&Token>) -> bool {
        let profile = &self.chord_engine.profile;
        match token {
            Some(t) if Self::is_character_assignment(t) => profile.char_key_repeat_assigned,
            Some(_) => profile.char_key_repeat_unassigned,
            None => profile.char_key_repeat_unassigned,
        }
    }

    fn repeat_fallback_events(
        &self,
        keys: &[ScKey],
        shift: bool,
        is_japanese: bool,
    ) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for k in keys {
            if let Some(token) = self.resolve(&[*k], shift, is_japanese) {
                if let Some(ops) = self.token_to_events(&token, shift) {
                    events.extend(ops);
                    continue;
                }
            }
            events.push(InputEvent::Scancode(k.sc, k.ext, false));
            events.push(InputEvent::Scancode(k.sc, k.ext, true));
        }
        events
    }

    fn is_character_assignment(token: &Token) -> bool {
        match token {
            Token::ImeChar(_) | Token::DirectChar(_) => true,
            Token::KeySequence(seq) => !seq.is_empty()
                && seq.iter().all(|stroke| {
                    stroke.mods.is_empty() && matches!(stroke.key, KeySpec::Char(_))
                }),
            Token::None => false,
        }
    }
}

fn append_keystroke_events(events: &mut Vec<InputEvent>, stroke: &KeyStroke, shift_held: bool) {
    let key_events = match stroke.key {
        KeySpec::Scancode(sc, ext) => Some((sc, ext)),
        KeySpec::VirtualKey(vk) => vk_to_scancode(vk),
        KeySpec::Char(c) => char_to_scancode(c),
    };

    if let Some((sc, ext)) = key_events {
        let mut mods = stroke.mods;
        if mods.shift && shift_held {
            mods.shift = false;
        }
        let mods = modifier_scancodes(mods);
        for (mod_sc, mod_ext) in mods.iter() {
            events.push(InputEvent::Scancode(*mod_sc, *mod_ext, false));
        }
        events.push(InputEvent::Scancode(sc, ext, false));
        events.push(InputEvent::Scancode(sc, ext, true));
        for (mod_sc, mod_ext) in mods.iter().rev() {
            events.push(InputEvent::Scancode(*mod_sc, *mod_ext, true));
        }
        return;
    }

    if let KeySpec::Char(c) = stroke.key {
        events.push(InputEvent::Unicode(c, false));
        events.push(InputEvent::Unicode(c, true));
    }
}

fn modifier_scancodes(mods: Modifiers) -> Vec<(u16, bool)> {
    let mut scancodes = Vec::new();
    if mods.ctrl {
        scancodes.push((0x1D, false));
    }
    if mods.shift {
        scancodes.push((0x2A, false));
    }
    if mods.alt {
        scancodes.push((0x38, false));
    }
    if mods.win {
        scancodes.push((0x5B, true));
    }
    scancodes
}

fn vk_to_scancode(vk: u16) -> Option<(u16, bool)> {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    if scan == 0 {
        return None;
    }
    let ext = (scan & 0xFF00) == 0xE000;
    Some(((scan & 0x00FF) as u16, ext))
}

fn char_to_scancode(c: char) -> Option<(u16, bool)> {
    match c {
        'a'..='z' | 'A'..='Z' => match c.to_ascii_lowercase() {
            'a' => Some((0x1E, false)),
            'b' => Some((0x30, false)),
            'c' => Some((0x2E, false)),
            'd' => Some((0x20, false)),
            'e' => Some((0x12, false)),
            'f' => Some((0x21, false)),
            'g' => Some((0x22, false)),
            'h' => Some((0x23, false)),
            'i' => Some((0x17, false)),
            'j' => Some((0x24, false)),
            'k' => Some((0x25, false)),
            'l' => Some((0x26, false)),
            'm' => Some((0x32, false)),
            'n' => Some((0x31, false)),
            'o' => Some((0x18, false)),
            'p' => Some((0x19, false)),
            'q' => Some((0x10, false)),
            'r' => Some((0x13, false)),
            's' => Some((0x1F, false)),
            't' => Some((0x14, false)),
            'u' => Some((0x16, false)),
            'v' => Some((0x2F, false)),
            'w' => Some((0x11, false)),
            'x' => Some((0x2D, false)),
            'y' => Some((0x15, false)),
            'z' => Some((0x2C, false)),
            _ => None,
        },
        '1' => Some((0x02, false)),
        '2' => Some((0x03, false)),
        '3' => Some((0x04, false)),
        '4' => Some((0x05, false)),
        '5' => Some((0x06, false)),
        '6' => Some((0x07, false)),
        '7' => Some((0x08, false)),
        '8' => Some((0x09, false)),
        '9' => Some((0x0A, false)),
        '0' => Some((0x0B, false)),
        '-' => Some((0x0C, false)),
        '^' => Some((0x0D, false)),
        '\\' | '¥' | '￥' => Some((0x7D, false)), // Yen
        '@' => Some((0x1A, false)),
        '[' => Some((0x1B, false)),
        ';' => Some((0x27, false)),
        ':' => Some((0x28, false)),
        ']' => Some((0x2B, false)),
        '_' => Some((0x73, false)), // JIS backslash/ro key
        ',' => Some((0x33, false)),
        '.' => Some((0x34, false)),
        '/' => Some((0x35, false)),
        ' ' => Some((0x39, false)),
        '\u{0008}' => Some((0x0E, false)), // BS
        '\u{000D}' => Some((0x1C, false)), // Enter
        '\u{F702}' => Some((0x4B, true)),  // Left Arrow (Extended)
        '\u{F703}' => Some((0x4D, true)),  // Right Arrow (Extended)
        '－' | 'ー' => Some((0x0C, false)), // Minus / Long Vowel
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_to_scancode() {
        assert_eq!(char_to_scancode('－'), Some((0x0C, false)));
        assert_eq!(char_to_scancode('ー'), Some((0x0C, false)));
        assert_eq!(char_to_scancode('1'), Some((0x02, false)));
        assert_eq!(char_to_scancode('a'), Some((0x1E, false)));
    }

    use crate::parser::parse_yab_content;

    #[test]
    fn test_chord_logic() {
        let config = "
[ローマ字シフト無し]
; Row 0
1,2,3,4,5,6,7,8,9,0,-,^,\\
; Row 1
q,w,e,r,t,y,u,i,o,p,@,[
; Row 2 (index 2)
no,to,d_base,nn,ltu,ku,u,k_base,l,;,:,]
; Row 3
z,x,c,v,b,n,m,,,.,/,\\

<k>
; Row 0
無,無,無,無,無,無,無,無,無,無,無,無,無
; Row 1
無,無,無,無,無,無,無,無,無,無,無,無
; Row 2
無,無,d_chord,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        // engine.chord_engine.profile.min_overlap_ms = 0; // Removed
        engine.load_layout(layout);

        // 1. Press K
        // Should output NOTHING now (Block)
        let res = engine.process_key(0x25, false, false, false); // Down
        assert_eq!(res, KeyAction::Block);

        // 2. Release K -> Should output "k_base" (Tap behavior)
        let res = engine.process_key(0x25, false, true, false); // Up
        match res {
            KeyAction::Inject(_events) => {
                // Good.
            }
            _ => panic!("Expected Inject on KeyUp for K, got {:?}", res),
        }
    }

    #[test]
    fn test_chord_logic_simple_chars() {
        let config = "
[ローマ字シフト無し]
; R0
1,2,3,4,5,6,7,8,9,0,-,^,\\
; R1
q,w,e,r,t,y,u,i,o,p,@,[
; R2: A S D(db) F G H J K(kb)
xx,xx,db,xx,xx,xx,xx,kb,xx,xx,xx,xx
; R3
z,x,c,v,b,n,m,,,.,/,\\

<k>
; R0
無,無,無,無,無,無,無,無,無,無,無,無,無
; R1
無,無,無,無,無,無,無,無,無,無,無,無
; R2: A S D(dc)
xx,xx,dc,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        // engine.chord_engine.profile.min_overlap_ms = 0; // Removed
        engine.load_layout(layout);

        // 1. Press K (0x25) -> Expect BLOCK (Delayed)
        let res = engine.process_key(0x25, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // 2. Press D (0x20) WHILE K is pressed -> Expect BLOCK because we need UP to calc ratio
        let res = engine.process_key(0x20, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // 3. Release D -> Now we have duration, can calc ratio. Expect "dc"
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                // Should contain c (0x2E) and d (which became c in chord)
                // Actually the chord output is "dc".
                assert_eq!(evs.len(), 4);
                // "c" -> 0x2E
                match evs[2] {
                    InputEvent::Scancode(sc, _, _) => assert_eq!(sc, 0x2E),
                    _ => panic!("Expected Scancode"),
                }
            }
            _ => panic!("Expected Inject for Chord D on Up, got {:?}", res),
        }

        // 4. Release K -> Should output NOTHING (Consumed)
        let res = engine.process_key(0x25, false, true, false);
        if res != KeyAction::Block {
            assert_eq!(res, KeyAction::Block);
        }

        // 5. Press D alone -> Expect "db"
        // Delayed Decision checks
        let res = engine.process_key(0x20, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // Release D -> output "db"
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 4);
                // "b" -> 0x30
                match evs[2] {
                    InputEvent::Scancode(sc, _, _) => assert_eq!(sc, 0x30),
                    _ => panic!("Expected Scancode"),
                }
            }
            _ => panic!("Expected Inject for Single D on Release, got {:?}", res),
        }
    }

    #[test]
    fn test_char_key_continuous_on() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,xx,d,f,xx,xx,xx,k,xx,xx,xx,xx

<k>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,x,y,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        engine.set_profile(profile);

        // Hold K as shift, then press D -> expect chord output "x".
        assert_eq!(engine.process_key(0x25, false, false, false), KeyAction::Block);
        assert_eq!(engine.process_key(0x20, false, false, false), KeyAction::Block);
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2D, _, _))),
                    "Expected 'x' output for K+D chord"
                );
            }
            _ => panic!("Expected Inject for K+D chord, got {:?}", res),
        }

        // While still holding K, press F -> expect chord output "y".
        assert_eq!(engine.process_key(0x21, false, false, false), KeyAction::Block);
        let res = engine.process_key(0x21, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x15, _, _))),
                    "Expected 'y' output for continuous K+F chord"
                );
            }
            _ => panic!("Expected Inject for K+F chord, got {:?}", res),
        }

        // Release K -> should not emit K base output.
        assert_eq!(engine.process_key(0x25, false, true, false), KeyAction::Block);
    }

    #[test]
    fn test_char_key_continuous_off() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,xx,d,f,xx,xx,xx,k,xx,xx,xx,xx

<k>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,x,y,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = false;
        engine.set_profile(profile);

        // Hold K as shift, then press D -> expect chord output "x".
        assert_eq!(engine.process_key(0x25, false, false, false), KeyAction::Block);
        assert_eq!(engine.process_key(0x20, false, false, false), KeyAction::Block);
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2D, _, _))),
                    "Expected 'x' output for K+D chord"
                );
            }
            _ => panic!("Expected Inject for K+D chord, got {:?}", res),
        }

        // K is still held, but continuous is off -> F should be a single tap ("f").
        assert_eq!(engine.process_key(0x21, false, false, false), KeyAction::Block);
        let res = engine.process_key(0x21, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x21, _, _))),
                    "Expected 'f' output for single F when continuous is off"
                );
            }
            _ => panic!("Expected Inject for single F, got {:?}", res),
        }

        assert_eq!(engine.process_key(0x25, false, true, false), KeyAction::Block);
    }

    #[test]
    fn test_shifted_layout() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,n_base,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,s_base,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        // engine.chord_engine.profile.min_overlap_ms = 0; // Removed
        engine.load_layout(layout);

        // 0x20 is 'd' key. In our dummy config, it corresponds to "n_base" (no shift) and "s_base" (shifted)

        // 1. No Shift -> press D (0x20)
        let res_down = engine.process_key(0x20, false, false, false); // Down, no shift
        assert_eq!(res_down, KeyAction::Block); // Delayed by engine logic

        let res_up = engine.process_key(0x20, false, true, false); // Up, no shift
        match res_up {
            KeyAction::Inject(evs) => {
                // n_base -> 'n' (0x31)
                assert!(
                    evs.iter()
                        .any(|e| if let InputEvent::Scancode(s, _, _) = e {
                            *s == 0x31
                        } else {
                            false
                        }),
                    "Expected 'n' in output"
                );
            }
            _ => panic!("Expected Inject for unshifted, got {:?}", res_up),
        }

        // 2. With Shift -> press D (0x20)
        // Note: engine checks shift state passed in.
        let res_down = engine.process_key(0x20, false, false, true); // Down, SHIFT=true
        assert_eq!(res_down, KeyAction::Block);

        let res_up = engine.process_key(0x20, false, true, true); // Up, SHIFT=true
        match res_up {
            KeyAction::Inject(evs) => {
                // s_base -> 's' (0x1F)
                assert!(
                    evs.iter()
                        .any(|e| if let InputEvent::Scancode(s, _, _) = e {
                            *s == 0x1F
                        } else {
                            false
                        }),
                    "Expected 's' in output"
                );
            }
            _ => panic!("Expected Inject for shifted, got {:?}", res_up),
        }
    }

    #[test]
    fn test_shift_rollover_chord_fallback_preserves_shift() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,n,m,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,s,t,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // D down (0x20), F down (0x21), F up -> chord detected but no chord mapping.
        // Fallback should preserve shift and use shifted plane for both keys.
        let res = engine.process_key(0x20, false, false, true);
        assert_eq!(res, KeyAction::Block);
        let res = engine.process_key(0x21, false, false, true);
        assert_eq!(res, KeyAction::Block);

        let res = engine.process_key(0x21, false, true, true);
        match res {
            KeyAction::Inject(evs) => {
                let has_s = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x1F, _, _)));
                let has_t = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x14, _, _)));
                assert!(
                    has_s && has_t,
                    "Expected shifted outputs (s,t) in fallback output"
                );

                let has_n = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x31, _, _)));
                let has_m = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x32, _, _)));
                assert!(
                    !has_n && !has_m,
                    "Fallback should not use base plane outputs (n,m)"
                );
            }
            _ => panic!("Expected Inject for shift rollover fallback, got {:?}", res),
        }
    }

    #[test]
    fn test_unicode_fallback() {
        let engine = Engine::default();
        let token = Token::KeySequence(vec![KeyStroke {
            key: KeySpec::Char('あ'),
            mods: Modifiers::none(),
        }]);
        // We can access private methods in tests module of the same file
        let events = engine
            .token_to_events(&token, false)
            .expect("Should return events");

        assert_eq!(events.len(), 2);
        match events[0] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, 'あ');
                assert_eq!(up, false);
            }
            _ => panic!("Expected Unicode down"),
        }
        match events[1] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, 'あ');
                assert_eq!(up, true);
            }
            _ => panic!("Expected Unicode up"),
        }
    }

    #[test]
    fn test_repeat_assigned_key_emits_repeat_and_suppresses_release() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_repeat_assigned = true;
        profile.char_key_repeat_unassigned = false;
        engine.set_profile(profile);

        let res_down = engine.process_key(0x20, false, false, false);
        assert_eq!(res_down, KeyAction::Block);

        let res_repeat = engine.process_key(0x20, false, false, false);
        match res_repeat {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 2);
                assert_eq!(evs[0], InputEvent::Scancode(0x1E, false, false));
                assert_eq!(evs[1], InputEvent::Scancode(0x1E, false, true));
            }
            _ => panic!("Expected Inject for repeat, got {:?}", res_repeat),
        }

        let res_up = engine.process_key(0x20, false, true, false);
        assert_eq!(res_up, KeyAction::Block);
    }

    #[test]
    fn test_repeat_assigned_key_disabled_allows_release_output() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_repeat_assigned = false;
        profile.char_key_repeat_unassigned = false;
        engine.set_profile(profile);

        let res_down = engine.process_key(0x20, false, false, false);
        assert_eq!(res_down, KeyAction::Block);

        let res_repeat = engine.process_key(0x20, false, false, false);
        assert_eq!(res_repeat, KeyAction::Block);

        let res_up = engine.process_key(0x20, false, true, false);
        match res_up {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 2);
                assert_eq!(evs[0], InputEvent::Scancode(0x1E, false, false));
                assert_eq!(evs[1], InputEvent::Scancode(0x1E, false, true));
            }
            _ => panic!("Expected Inject on release, got {:?}", res_up),
        }
    }

    #[test]
    fn test_repeat_start_uses_chord_definition() {
        let config = "
[ローマ字シフト無し]
; R0
無
; R1
無
; R2
a,無,無,無,無,無,無,無,無,無,無,無
; R3
無,無,無,無,b,無,無,無,無,無,無

<a>
; R0
無
; R1
無
; R2
無,無,無,無,無,無,無,無,無,無,無,無
; R3
無,無,無,無,x,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_repeat_assigned = true;
        profile.char_key_repeat_unassigned = false;
        engine.set_profile(profile);

        let res_a_down = engine.process_key(0x1E, false, false, false);
        assert_eq!(res_a_down, KeyAction::Block);

        let res_b_down = engine.process_key(0x30, false, false, false);
        assert_eq!(res_b_down, KeyAction::Block);

        let res_repeat = engine.process_key(0x1E, false, false, false);
        match res_repeat {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 2);
                assert_eq!(evs[0], InputEvent::Scancode(0x2D, false, false));
                assert_eq!(evs[1], InputEvent::Scancode(0x2D, false, true));
            }
            _ => panic!("Expected Inject for chord repeat, got {:?}", res_repeat),
        }
    }

    #[test]
    fn test_chord_logic_fallback() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,d_base,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
dummy

<k>
; R0
dummy
; R1
dummy
; R2
無,無,無,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        // engine.chord_engine.profile.min_overlap_ms = 0; // Removed
        engine.load_layout(layout);

        // 1. Press K (0x25) -> Expect BLOCK (Delayed)
        let res = engine.process_key(0x25, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // 2. Press D (0x20) WHILE K is pressed.
        // Chord K+D is detected.
        // But <k> plane has "無" (None) at D position (col 2).
        // resolve() returns None.
        // Fallback logic should trigger: Inject K, then D.
        // BUT now we check if they are resolved via layout.
        // K is at Col 7? In R2: "xx,xx,d_base,xx,xx,xx,xx,xx,..."
        // Index 7 is "xx". "xx" parses as KeySequence("xx").
        // D is at Col 2. "d_base" parses as KeySequence("d_base").

        // 2. Press D (0x20) WHILE K is pressed.
        // Expect BLOCK until D Up.
        let res = engine.process_key(0x20, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // 3. Release D -> Logic decides "Chord" (K+D). Fallback logic runs.
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                // If fallback uses raw scancode, we get K, D.
                // If fallback uses layout, we get "xx" for K (0x25), "d_base" for D.

                // Let's check for "x" scancode (0x2D) to prove resolution happened for K.
                let has_x = evs.iter().any(|e| match e {
                    InputEvent::Scancode(sc, _, _) => *sc == 0x2D,
                    _ => false,
                });
                assert!(
                    has_x,
                    "Expected 'x' (from 'xx' definition for K) in fallback output"
                );
            }
            _ => panic!("Expected Inject (Fallback) on Up, got {:?}", res),
        }
    }

    #[test]
    fn test_undefined_key_passthrough() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        // engine.chord_engine.profile.min_overlap_ms = 0; // Removed
        engine.load_layout(layout);

        // 1. Press A (0x1E) -> Defined in layout. Expect BLOCK (Wait).
        let res = engine.process_key(0x1E, false, false, false);
        assert_eq!(res, KeyAction::Block, "Defined key 'A' should wait");

        // 2. Press B (0x30) -> NOT defined. Expect PASS (Passthrough).
        // Since it's passthrough, process_key should return KeyAction::Pass
        // (because engine returns Passthrough decision and we check if k==key).
        let res = engine.process_key(0x30, false, false, false);
        assert_eq!(
            res,
            KeyAction::Pass,
            "Undefined key 'B' should pass through immediately"
        );

        // 3. Press RightArrow (0x4D extended) -> NOT defined. Expect PASS.
        let res = engine.process_key(0x4D, true, false, false);
        assert_eq!(
            res,
            KeyAction::Pass,
            "Undefined key 'RightArrow' should pass through immediately"
        );
    }

    #[test]
    fn test_space_rollover_flushes_previous_key() {
        // Space is not defined in the layout and not a thumb key.
        // When Space is pressed while a defined key is pending,
        // the pending key should flush BEFORE Space is sent.
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // 1. Press A -> Defined in layout. Expect BLOCK (Wait).
        let res = engine.process_key(0x1E, false, false, false);
        assert_eq!(res, KeyAction::Block, "Defined key 'A' should wait");

        // 2. Press Space while A is still down -> Expect Inject with A then Space.
        let res = engine.process_key(0x39, false, false, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 3, "Expected A down/up + Space down");
                assert_eq!(evs[0], InputEvent::Scancode(0x1E, false, false));
                assert_eq!(evs[1], InputEvent::Scancode(0x1E, false, true));
                assert_eq!(evs[2], InputEvent::Scancode(0x39, false, false));
            }
            _ => panic!("Expected Inject for Space rollover, got {:?}", res),
        }
    }

    #[test]
    fn test_space_rollover_preserves_chord() {
        // Space rollover should not destroy chord detection.
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2 (A,S defined)
a,s,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<s>
; R0
dummy
; R1
dummy
; R2 (A under <s> -> x)
x,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // A Down
        let res = engine.process_key(0x1E, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // S Down
        let res = engine.process_key(0x1F, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // Space Down -> expect chord output (x) then space down
        let res = engine.process_key(0x39, false, false, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 3, "Expected x down/up + Space down");
                assert_eq!(evs[0], InputEvent::Scancode(0x2D, false, false));
                assert_eq!(evs[1], InputEvent::Scancode(0x2D, false, true));
                assert_eq!(evs[2], InputEvent::Scancode(0x39, false, false));
            }
            _ => panic!("Expected Inject for Space rollover chord, got {:?}", res),
        }
    }

    #[test]
    fn test_undefined_enter_pass() {
        // Reproduce user issue: Enter key waiting for Up?
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout); // target_keys should be Some({...})

        // 1. Press Enter (0x1C) -> NOT using generic RC map. should Pass.
        let res = engine.process_key(0x1C, false, false, false);
        assert_eq!(
            res,
            KeyAction::Pass,
            "Enter key (0x1C) should pass immediately (Down)"
        );

        // 2. Up Enter
        let res = engine.process_key(0x1C, false, true, false);
        assert_eq!(
            res,
            KeyAction::Pass,
            "Enter key (0x1C) should pass immediately (Up)"
        );
    }

    #[test]
    fn test_set_profile_preserves_targets() {
        let config = "
[ローマ字シフト無し]
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        // Verify target_keys is set
        assert!(engine.get_profile().target_keys.is_some());

        // Update profile (e.g. changing timeout)
        let mut new_profile = Profile::default();
        new_profile.chord_window_ms = 999;
        // target_keys is None in default()

        engine.set_profile(new_profile);

        // Verify target_keys is PRESERVED
        assert!(
            engine.get_profile().target_keys.is_some(),
            "target_keys should be preserved"
        );

        // Verify Enter key (undefined) still Passes
        let res = engine.process_key(0x1C, false, false, false);
        assert_eq!(
            res,
            KeyAction::Pass,
            "Enter should still pass after profile update"
        );
    }

    #[test]
    fn test_ime_section_switching() {
        let config = "
[英数シフト無し]
; R0
dummy
; R1
dummy
; R2
alph_a
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
roma_a
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        // 1. Force Japanese Mode (Ignore)
        engine.set_ime_mode(ImeMode::Ignore);

        // Down
        engine.process_key(0x1E, false, false, false);
        // Up
        let res = engine.process_key(0x1E, false, true, false);

        match res {
            KeyAction::Inject(evs) => {
                // roma_a starts with 'r' (0x13)
                if let InputEvent::Scancode(sc, _, _) = evs[0] {
                    assert_eq!(sc, 0x13, "Expected 'r' from [ローマ字...], got {:02X}", sc);
                }
            }
            _ => panic!("Expected Inject in Roman mode, got {:?}", res),
        }

        // 2. Force Alpha Mode
        engine.set_ime_mode(ImeMode::ForceAlpha);

        // Down (Reset pending first? Engine state persists. Need to wait for previous key to clear?
        // Previous Up flushed pending. So safe.)
        engine.process_key(0x1E, false, false, false);
        // Up
        let res = engine.process_key(0x1E, false, true, false);

        match res {
            KeyAction::Inject(evs) => {
                // alph_a starts with 'a' (0x1E)
                // Actually alph_a -> a,l,p,h... 'a' is 0x1E.
                if let InputEvent::Scancode(sc, _, _) = evs[0] {
                    assert_eq!(sc, 0x1E, "Expected 'a' from [英数...], got {:02X}", sc);
                }
            }
            _ => panic!("Expected Inject in Alpha mode, got {:?}", res),
        }
    }

    #[test]
    fn test_missing_section_fallback() {
        // Layout: [ローマ字] defined. [英数] MISSING.
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
a,roma_a
";
        let layout = parse_yab_content(config).expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        // 1. Force Alpha Mode (Simulate IME OFF / Alpha)
        engine.set_ime_mode(ImeMode::ForceAlpha);

        // Down
        let res_down = engine.process_key(0x1E, false, false, false);
        assert_eq!(
            res_down,
            KeyAction::Pass,
            "Should PASS immediately if section is missing"
        );

        // Up
        let res_up = engine.process_key(0x1E, false, true, false);
        assert_eq!(res_up, KeyAction::Pass, "Should PASS immediately on Up too");
    }

    #[test]
    fn test_thumb_shift_filtering() {
        // Setup: Left Thumb = 0x7B (Muhenkan)
        // Layout: [ローマ字左親指シフト] -> a=thumb_a
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
roma_a

[ローマ字左親指シフト]
; R0
dummy
; R1
dummy
; R2
thumb_a
";
        let layout = parse_yab_content(config).expect("Failed to parse");
        let mut engine = Engine::default();

        let mut profile = Profile::default();
        profile.ime_mode = ImeMode::Ignore; // Force Japanese

        // Use 0x7B as thumb key
        let thumb_key = ScKey::new(0x7B, false);
        let mut left_thumbs = HashSet::new();
        left_thumbs.insert(thumb_key);

        profile.thumb_keys = Some(crate::chord_engine::ThumbKeys {
            left: left_thumbs,
            right: HashSet::new(),
        });

        // Set profile BEFORE loading layout (although load_layout merges triggers, thumb keys are separate)
        // Actually load_layout uses profile to determine Trigger Keys. Thumb Keys are manual.
        // We set profile first to ensure engine has thumb keys config.
        engine.set_profile(profile);
        engine.load_layout(layout);

        // Sequence: Thumb(Down) -> A(Down) -> A(Up) -> Thumb(Up)
        // Note: A(Up) triggers ratio check. P1(Thumb) Down, P2(A) Up.
        // This is valid overlap. Ratio check might pass if overlap is sufficient.
        // Overlap = Duration of P2(A). Ratio = 1.0.

        engine.process_key(0x7B, false, false, false); // Thumb Down
        engine.process_key(0x1E, false, false, false); // A Down

        // Release A (P2)
        let res_a = engine.process_key(0x1E, false, true, false);

        match res_a {
            KeyAction::Inject(evs) => {
                // thumb_a starts with 't' (0x14)
                if let InputEvent::Scancode(sc, _, _) = evs[0] {
                    assert_eq!(sc, 0x14, "Expected 't' from [ローマ字左親指シフト]");
                } else {
                    panic!("Expected Scancode, got {:?}", evs[0]);
                }

                // Verify Thumb Key is NOT output
                let has_thumb = evs.iter().any(|e| match e {
                    InputEvent::Scancode(s, _, _) => *s == 0x7B,
                    _ => false,
                });
                assert!(!has_thumb, "Thumb key should be consumed and filtered");
            }
            _ => panic!("Expected Inject for Chord, got {:?}", res_a),
        }

        engine.process_key(0x7B, false, true, false); // Thumb Up (Consumed)
    }
    #[test]
    fn test_thumb_shift_switching() {
        let config = r#"
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d_base,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d_left,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字右親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d_right,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"#;
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // 1. Default Mode: NonTransformTransform (Left=Muhenkan, Right=Henkan)
        let sc_d = 0x20;
        let sc_muhenkan = 0x7B;
        let sc_henkan = 0x79;
        let sc_space = 0x39;

        // Debug assertions
        let profile = engine.get_profile();
        let targets = profile.target_keys.as_ref().expect("Target keys not set");
        let thumbs = profile.thumb_keys.as_ref().expect("Thumb keys not set");

        assert!(
            targets.contains(&ScKey::new(sc_d, false)),
            "D not in targets. Targets: {:?}",
            targets
        );
        assert!(
            targets.contains(&ScKey::new(sc_muhenkan, false)),
            "Muhenkan not in targets"
        );
        assert!(
            thumbs.left.contains(&ScKey::new(sc_muhenkan, false)),
            "Muhenkan not in Left thumbs"
        );

        // Case 1-1: Muhenkan + D -> Left
        engine.process_key(sc_muhenkan, false, false, false); // Muhenkan Down
        engine.process_key(sc_d, false, false, false); // D Down
        let res = engine.process_key(sc_d, false, true, false); // D Up (Tap with Modifier)
        match res {
            KeyAction::Inject(evs) => {
                // d_left -> l (0x26)
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x26, _, _))),
                    "Expected d_left (l) output"
                );
            }
            _ => panic!("Expected Inject Left for Muhenkan+D, got {:?}", res),
        }
        engine.process_key(sc_muhenkan, false, true, false); // Muhenkan Up

        // Case 1-2: Henkan + D -> Right
        engine.process_key(sc_henkan, false, false, false); // Henkan Down
        engine.process_key(sc_d, false, false, false); // D Down
        let res = engine.process_key(sc_d, false, true, false); // D Up
        match res {
            KeyAction::Inject(evs) => {
                // d_right -> r (0x13)
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x13, _, _))),
                    "Expected d_right (r) output"
                );
            }
            _ => panic!("Expected Inject Right for Henkan+D, got {:?}", res),
        }
        engine.process_key(sc_henkan, false, true, false); // Henkan Up

        // 2. Switch Mode: NonTransformSpace (Left=Muhenkan, Right=Space)
        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::Muhenkan;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::Space;
        engine.set_profile(profile);

        // Case 2-1: Space + D -> Right
        engine.process_key(sc_space, false, false, false); // Space Down
        engine.process_key(sc_d, false, false, false); // D Down
        let res = engine.process_key(sc_d, false, true, false); // D Up
        match res {
            KeyAction::Inject(evs) => {
                // d_right -> r (0x13)
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x13, _, _))),
                    "Expected d_right (r) output with Space"
                );
            }
            _ => panic!("Expected Inject Right for Space+D, got {:?}", res),
        }
        engine.process_key(sc_space, false, true, false); // Space Up
    }

    #[test]
    fn test_suspend_key_persists_when_disabled() {
        let mut engine = Engine::default();
        let mut profile = engine.get_profile();
        profile.suspend_key = crate::chord_engine::SuspendKey::Pause;
        engine.set_profile(profile);

        engine.set_enabled(false);
        assert_eq!(
            engine.get_profile().suspend_key,
            crate::chord_engine::SuspendKey::Pause
        );
    }
}

