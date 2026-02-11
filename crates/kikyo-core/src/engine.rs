use crate::chord_engine::{
    ChordEngine, Decision, ImeMode, KeyEdge, KeyEvent, PendingKey, Profile, EXTENDED_KEY_1_SC,
    EXTENDED_KEY_2_SC, EXTENDED_KEY_3_SC, EXTENDED_KEY_4_SC,
};
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

#[derive(Debug, Clone, Copy)]
enum FunctionKeySwapTarget {
    Key(ScKey),
    CapsLock,
    KanaLock,
}

#[derive(Debug, Clone, Copy)]
enum FunctionPseudoKey {
    CapsLock,
    KanaLock,
}

#[derive(Debug, Clone, Copy)]
enum PassThroughCurrent {
    Original,
    Inject(ScKey),
    Block,
}

#[derive(Debug, Clone, Copy)]
struct DeferredEnterRollover {
    source_key: ScKey,
    pass_through: PassThroughCurrent,
    wait_for: ScKey,
    down_emitted: bool,
    up_seen_while_waiting: bool,
}

pub struct Engine {
    chord_engine: ChordEngine,
    enabled: bool,
    layout: Option<Layout>,
    on_enabled_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    repeat_plans: HashMap<ScKey, Vec<ScKey>>,
    pending_nonshift_for_shift: HashSet<ScKey>,
    function_key_swaps: HashMap<ScKey, FunctionKeySwapTarget>,
    deferred_enter_rollover: Option<DeferredEnterRollover>,
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
            pending_nonshift_for_shift: HashSet::new(),
            function_key_swaps: HashMap::new(),
            deferred_enter_rollover: None,
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
                self.pending_nonshift_for_shift.clear();
                self.deferred_enter_rollover = None;
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

    pub fn needs_alt_handling(&self) -> bool {
        let left_alt = ScKey::new(0x38, false);
        let right_alt = ScKey::new(0x38, true);

        if self.function_key_swaps.contains_key(&left_alt)
            || self.function_key_swaps.contains_key(&right_alt)
        {
            return true;
        }

        if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
            if tk.left.contains(&left_alt)
                || tk.left.contains(&right_alt)
                || tk.right.contains(&left_alt)
                || tk.right.contains(&right_alt)
                || tk.ext1.contains(&left_alt)
                || tk.ext1.contains(&right_alt)
                || tk.ext2.contains(&left_alt)
                || tk.ext2.contains(&right_alt)
            {
                return true;
            }
        }

        if let Some(ref targets) = self.chord_engine.profile.target_keys {
            if targets.contains(&left_alt) || targets.contains(&right_alt) {
                return true;
            }
        }

        false
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
            if layout
                .sections
                .contains_key("\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}1")
                || layout
                    .sections
                    .contains_key("\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}2")
            {
                return true;
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
                targets.extend(tk.ext1.iter());
                targets.extend(tk.ext2.iter());
            }
        }

        self.chord_engine.set_profile(profile);
    }

    pub fn load_layout(&mut self, layout: Layout) {
        tracing::info!(
            "Engine: Layout loaded with {} sections.",
            layout.sections.len()
        );
        self.function_key_swaps = build_function_key_swap_map(&layout.function_key_swaps);

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
            // tracing::info!(" - Section: {}", name);
            // Parse "<A><B>" style tags
            let mut start = 0;
            while let Some(open) = name[start..].find('<') {
                if let Some(close) = name[start + open..].find('>') {
                    let inner = &name[start + open + 1..start + open + close];
                    if let Some(sc) = crate::jis_map::key_name_to_sc(inner) {
                        let key = ScKey::new(sc, false);
                        if !profile.trigger_keys.contains_key(&key) {
                            profile.trigger_keys.insert(key, name.clone());
                            tracing::info!(
                                "   -> Registered TriggerKey: {} (sc={:02X}) from {}",
                                inner,
                                sc,
                                name
                            );
                        }
                        target_keys.insert(key);
                    }
                    start += open + close + 1;
                } else {
                    break;
                }
            }

            for tag in section.sub_planes.keys() {
                let mut start = 0;
                while let Some(open) = tag[start..].find('<') {
                    if let Some(close) = tag[start + open..].find('>') {
                        let inner = &tag[start + open + 1..start + open + close];
                        if let Some(sc) = crate::jis_map::key_name_to_sc(inner) {
                            let key = ScKey::new(sc, false);
                            if !profile.trigger_keys.contains_key(&key) {
                                profile.trigger_keys.insert(key, tag.clone());
                                tracing::info!(
                                    "   -> Registered TriggerKey: {} (sc={:02X}) from subplane {}",
                                    inner,
                                    sc,
                                    tag
                                );
                            }
                            target_keys.insert(key);
                        }
                        start += open + close + 1;
                    } else {
                        break;
                    }
                }
            }
        }

        // Add thumb keys if any (currently handled via profile manually or elsewhere, but let's ensure)
        if let Some(ref tk) = profile.thumb_keys {
            target_keys.extend(tk.left.iter());
            target_keys.extend(tk.right.iter());
            target_keys.extend(tk.ext1.iter());
            target_keys.extend(tk.ext2.iter());
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

        let source_key = ScKey::new(sc, ext);
        let (key, pass_through_current, pseudo_key) = self.remap_input_key(source_key);
        if let Some(pseudo) = pseudo_key {
            return emit_pseudo_function_key(pseudo, up);
        }

        if let Some(action) =
            self.handle_deferred_enter_event(source_key, key, pass_through_current, up)
        {
            return action;
        }

        if !up && self.is_repeat_event(key) {
            return self.handle_repeat_event(key, shift, is_japanese);
        }

        self.handle_deferred_nonshift_before_event(key, up, shift, is_japanese);

        // Pre-check: Verify if the key is defined in the current section.
        // If not, we pass immediately to avoid ChordEngine buffering.
        {
            // 1. Determine local "Thumb Shift" status from ChordEngine state
            let mut has_left_thumb = false;
            let mut has_right_thumb = false;
            let mut has_ext1_thumb = false;
            let mut has_ext2_thumb = false;
            if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                let mut mark_thumb_state = |k: &ScKey| {
                    if tk.left.contains(k) {
                        has_left_thumb = true;
                    }
                    if tk.right.contains(k) {
                        has_right_thumb = true;
                    }
                    if tk.ext1.contains(k) {
                        has_ext1_thumb = true;
                    }
                    if tk.ext2.contains(k) {
                        has_ext2_thumb = true;
                    }
                };

                for k in &self.chord_engine.state.pressed {
                    mark_thumb_state(k);
                }

                // PrefixShift uses a released thumb as the next one-shot modifier.
                // Include it in section pre-check so the next key isn't passed through early.
                if let Some(prefix_thumb) = self.chord_engine.state.prefix_pending {
                    mark_thumb_state(&prefix_thumb);
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

            let section_name =
                if is_japanese && !has_left_thumb && !has_right_thumb && has_ext1_thumb {
                    "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}1".to_string()
                } else if is_japanese && !has_left_thumb && !has_right_thumb && has_ext2_thumb {
                    "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}2".to_string()
                } else {
                    section_name
                };
            // eprintln!("DEBUG: Resolve: section={} keys={:?} japanese={}", section_name, keys, is_japanese);

            // 3. Check Section Existence
            if let Some(layout) = &self.layout {
                let is_space = key.sc == 0x39;
                let key_is_managed = self.chord_engine.state.pressed.contains(&key)
                    || self.chord_engine.state.down_ts.contains_key(&key)
                    || self.chord_engine.state.pending.iter().any(|p| p.key == key);
                let mut is_thumb = false;
                if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                    if tk.left.contains(&key)
                        || tk.right.contains(&key)
                        || tk.ext1.contains(&key)
                        || tk.ext2.contains(&key)
                    {
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
                        if let Some(name) = crate::jis_map::sc_to_key_name(key.sc) {
                            let tag = format!("<{}>", name);
                            if section.sub_planes.contains_key(&tag) {
                                is_defined = true;
                            }
                            // Also check for 2-key prefix in subplanes?
                            // No, current logic only checks single key triggers here?
                            // Wait! <q><w> is a subplane key.
                            // But checking 'q' -> tag '<q>'.
                            // If section has '<q><w>', does it have '<q>'?
                            // parser.rs: '<q><w>' creates a subplane keyed by "<q><w>".
                            // It does NOT create '<q>'.
                            // So if I press 'Q', and there is only '<q><w>', then 'Q' is NOT defined as a trigger??
                            // THIS IS THE BUG!
                            // For 3-key chords to work, the first key MUST be recognized as a trigger or defined key.
                            // If 'Q' is not in base plane (it is in test).
                            // But if 'Q' was 'xx' in base plane?
                            // In test: `q` is in base plane.
                            // So `is_defined` is true via base plane.
                        }
                    }

                    if !is_defined && !is_thumb && !is_space && !(up && key_is_managed) {
                        if self.start_deferred_enter_rollover(
                            source_key,
                            key,
                            pass_through_current,
                            up,
                        ) {
                            return KeyAction::Block;
                        }
                        // Defined section, but key is not in it -> Pass
                        return passthrough_action(pass_through_current, source_key, up);
                    }
                } else {
                    // Section does NOT exist -> Pass
                    // UNLESS it is a Thumb Key
                    if !is_thumb && !is_space && !(up && key_is_managed) {
                        if self.start_deferred_enter_rollover(
                            source_key,
                            key,
                            pass_through_current,
                            up,
                        ) {
                            return KeyAction::Block;
                        }
                        return passthrough_action(pass_through_current, source_key, up);
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
                    let (token, modifier) = self.resolve_with_modifier(&keys, shift, is_japanese);
                    if let Some(token) = token {
                        if let Some(ops) = self.token_to_events(&token, shift) {
                            inject_ops.extend(ops);
                        }
                        if let Some(mod_key) = modifier {
                            self.consume_non_modifier_keys(&keys, mod_key);
                        }
                    } else {
                        // Continuous shift rollover case:
                        // if an older still-held key and a later key formed an undefined chord,
                        // emit only the later key to avoid leaking the older key's single output.
                        let undefined_rollover_pair =
                            self.chord_engine.profile.char_key_continuous && keys.len() == 2;
                        let older_pressed = undefined_rollover_pair
                            && self.chord_engine.state.pressed.contains(&keys[0]);
                        let newer_pressed = undefined_rollover_pair
                            && self.chord_engine.state.pressed.contains(&keys[1]);
                        let older_is_continuous_used_modifier = undefined_rollover_pair
                            && self.is_char_shift_key(keys[0])
                            && self.chord_engine.state.used_modifiers.contains(&keys[0]);

                        if undefined_rollover_pair && older_pressed && !newer_pressed {
                            let k = keys[1];
                            self.chord_engine.state.used_modifiers.remove(&k);
                            let mut resolved = false;
                            if let Some(token) = self.resolve(&[k], shift, is_japanese) {
                                if let Some(ops) = self.token_to_events(&token, shift) {
                                    inject_ops.extend(ops);
                                    resolved = true;
                                }
                            }
                            if !resolved {
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, false));
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, true));
                            }
                        } else if undefined_rollover_pair && !older_pressed && newer_pressed {
                            // Older key was released first during rollover.
                            // Suppress older key output and let newer key resolve on its own Up.
                            self.chord_engine.state.used_modifiers.remove(&keys[1]);
                        } else if undefined_rollover_pair
                            && !older_pressed
                            && !newer_pressed
                            && older_is_continuous_used_modifier
                        {
                            // Both keys are up and the older key is a carried-over continuous modifier.
                            // Emit only the later key to avoid leaking the older key's single output.
                            let k = keys[1];
                            let mut resolved = false;
                            if let Some(token) = self.resolve(&[k], shift, is_japanese) {
                                if let Some(ops) = self.token_to_events(&token, shift) {
                                    inject_ops.extend(ops);
                                    resolved = true;
                                }
                            }
                            if !resolved {
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, false));
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, true));
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
            inject_ops.extend(self.release_deferred_enter_on_wait_key_up(key));
            self.repeat_plans.remove(&key);
        }

        if !inject_ops.is_empty() {
            if pass_current {
                // If we also need to pass the current key, append it to the injection sequence.
                // This ensures "Flushed Keys" -> "Current Key" order.
                if let Some(ev) = passthrough_event(pass_through_current, source_key, up) {
                    inject_ops.push(ev);
                }
            }
            return KeyAction::Inject(inject_ops);
        }

        if pass_current {
            return passthrough_action(pass_through_current, source_key, up);
        }

        KeyAction::Block
    }

    fn is_enter_key(key: ScKey) -> bool {
        key.sc == 0x1C
    }

    fn latest_pressed_managed_key_except(&self, excluded: ScKey) -> Option<ScKey> {
        self.chord_engine
            .state
            .down_ts
            .iter()
            .filter_map(|(k, t)| {
                if *k == excluded || !self.chord_engine.state.pressed.contains(k) {
                    None
                } else {
                    Some((*k, *t))
                }
            })
            .max_by_key(|(_, t)| *t)
            .map(|(k, _)| k)
    }

    fn start_deferred_enter_rollover(
        &mut self,
        source_key: ScKey,
        key: ScKey,
        pass_through: PassThroughCurrent,
        up: bool,
    ) -> bool {
        if up || !Self::is_enter_key(key) || self.deferred_enter_rollover.is_some() {
            return false;
        }

        let Some(wait_for) = self.latest_pressed_managed_key_except(key) else {
            return false;
        };

        self.deferred_enter_rollover = Some(DeferredEnterRollover {
            source_key,
            pass_through,
            wait_for,
            down_emitted: false,
            up_seen_while_waiting: false,
        });
        true
    }

    fn handle_deferred_enter_event(
        &mut self,
        source_key: ScKey,
        key: ScKey,
        _pass_through: PassThroughCurrent,
        up: bool,
    ) -> Option<KeyAction> {
        if !Self::is_enter_key(key) {
            return None;
        }

        let mut deferred = self.deferred_enter_rollover?;
        if deferred.source_key != source_key {
            return None;
        }

        if up {
            if deferred.down_emitted {
                self.deferred_enter_rollover = None;
                if let Some(event) =
                    passthrough_event(deferred.pass_through, deferred.source_key, true)
                {
                    return Some(KeyAction::Inject(vec![event]));
                }
                return Some(KeyAction::Block);
            }

            deferred.up_seen_while_waiting = true;
            self.deferred_enter_rollover = Some(deferred);
            return Some(KeyAction::Block);
        }

        Some(KeyAction::Block)
    }

    fn release_deferred_enter_on_wait_key_up(&mut self, key: ScKey) -> Vec<InputEvent> {
        let Some(mut deferred) = self.deferred_enter_rollover.take() else {
            return Vec::new();
        };

        if deferred.down_emitted || deferred.wait_for != key {
            self.deferred_enter_rollover = Some(deferred);
            return Vec::new();
        }

        let mut events = Vec::new();
        if let Some(event) = passthrough_event(deferred.pass_through, deferred.source_key, false) {
            events.push(event);
        }

        deferred.down_emitted = true;

        if deferred.up_seen_while_waiting {
            if let Some(event) = passthrough_event(deferred.pass_through, deferred.source_key, true)
            {
                events.push(event);
            }
        } else {
            self.deferred_enter_rollover = Some(deferred);
        }

        events
    }

    fn remap_input_key(
        &self,
        source_key: ScKey,
    ) -> (ScKey, PassThroughCurrent, Option<FunctionPseudoKey>) {
        let mut current = source_key;
        let mut changed = false;
        let mut visited = HashSet::new();

        while let Some(target) = self.function_key_swaps.get(&current).copied() {
            if !visited.insert(current) {
                break;
            }
            changed = true;
            match target {
                FunctionKeySwapTarget::Key(next) => current = next,
                FunctionKeySwapTarget::CapsLock => {
                    return (
                        current,
                        PassThroughCurrent::Block,
                        Some(FunctionPseudoKey::CapsLock),
                    );
                }
                FunctionKeySwapTarget::KanaLock => {
                    return (
                        current,
                        PassThroughCurrent::Block,
                        Some(FunctionPseudoKey::KanaLock),
                    );
                }
            }
        }

        let pass = if !changed {
            PassThroughCurrent::Original
        } else if is_virtual_extended_key(current) {
            PassThroughCurrent::Block
        } else {
            PassThroughCurrent::Inject(current)
        };

        (current, pass, None)
    }

    fn resolve(&self, keys: &[ScKey], shift: bool, is_japanese: bool) -> Option<Token> {
        self.resolve_with_modifier(keys, shift, is_japanese).0
    }

    fn resolve_with_modifier(
        &self,
        keys: &[ScKey],
        shift: bool,
        is_japanese: bool,
    ) -> (Option<Token>, Option<ScKey>) {
        let layout = match self.layout.as_ref() {
            Some(layout) => layout,
            None => return (None, None),
        };

        // 1. Determine "Thumb Shift" status
        let mut has_left_thumb = false;
        let mut has_right_thumb = false;
        let mut has_ext1_thumb = false;
        let mut has_ext2_thumb = false;

        if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
            for k in keys {
                if tk.left.contains(k) {
                    has_left_thumb = true;
                }
                if tk.right.contains(k) {
                    has_right_thumb = true;
                }
                if tk.ext1.contains(k) {
                    has_ext1_thumb = true;
                }
                if tk.ext2.contains(k) {
                    has_ext2_thumb = true;
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
        let section_name = if is_japanese && !has_left_thumb && !has_right_thumb && has_ext1_thumb {
            "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}1".to_string()
        } else if is_japanese && !has_left_thumb && !has_right_thumb && has_ext2_thumb {
            "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}2".to_string()
        } else {
            section_name
        };
        // eprintln!("DEBUG: Resolve: section={} keys={:?} japanese={}", section_name, keys, is_japanese);

        let section = match layout.sections.get(&section_name) {
            Some(section) => section,
            None => return (None, None),
        };

        // 4. Update keys for lookup (Remove Thumb Modifiers)
        let lookup_keys: Vec<ScKey> =
            if has_left_thumb || has_right_thumb || has_ext1_thumb || has_ext2_thumb {
                if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                    keys.iter()
                        .filter(|&&k| {
                            let is_left = tk.left.contains(&k);
                            let is_right = tk.right.contains(&k);
                            let is_ext1 = tk.ext1.contains(&k);
                            let is_ext2 = tk.ext2.contains(&k);
                            if has_left_thumb && is_left {
                                return false;
                            }
                            if has_right_thumb && is_right {
                                return false;
                            }
                            if has_ext1_thumb && is_ext1 {
                                return false;
                            }
                            if has_ext2_thumb && is_ext2 {
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
            return (None, None);
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
                            return (Some(token.clone()), None);
                        }
                    }
                }
            }

            if let Some(rc) = self.key_to_rc(key) {
                return (section.base_plane.map.get(&rc).cloned(), None);
            }
        } else if lookup_keys.len() == 2 {
            let k1 = lookup_keys[0];
            let k2 = lookup_keys[1];

            if let Some(token) = self.try_resolve_modifier(section, k1, k2) {
                return (Some(token), Some(k1));
            }
            if let Some(token) = self.try_resolve_modifier(section, k2, k1) {
                return (Some(token), Some(k2));
            }
        } else if lookup_keys.len() == 3 {
            // 3-key resolution (A, B, C)
            // Check if any combination of 2 keys forms a modifier for the 3rd key
            // Permutations:
            // (A,B) -> C ?? Tag <A><B> or <B><A>
            // (A,C) -> B
            // (B,C) -> A
            let k1 = lookup_keys[0];
            let k2 = lookup_keys[1];
            let k3 = lookup_keys[2];
            // eprintln!("DEBUG: resolving 3 keys: {:?}, {:?}, {:?}", k1, k2, k3);

            // 1. Modifiers: k1, k2. Target: k3
            if let Some(token) = self.try_resolve_double_modifier(section, k1, k2, k3) {
                // eprintln!("DEBUG: Resolved (k1, k2) -> k3: {:?}", token);
                return (Some(token), Some(k1));
            }
            if let Some(token) = self.try_resolve_double_modifier(section, k2, k1, k3) {
                return (Some(token), Some(k2));
            }

            // 2. Modifiers: k1, k3. Target: k2
            if let Some(token) = self.try_resolve_double_modifier(section, k1, k3, k2) {
                return (Some(token), Some(k1));
            }
            if let Some(token) = self.try_resolve_double_modifier(section, k3, k1, k2) {
                return (Some(token), Some(k3));
            }

            // 3. Modifiers: k2, k3. Target: k1
            if let Some(token) = self.try_resolve_double_modifier(section, k2, k3, k1) {
                return (Some(token), Some(k2));
            }
            if let Some(token) = self.try_resolve_double_modifier(section, k3, k2, k1) {
                return (Some(token), Some(k3));
            }
        }

        (None, None)
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
                if let Some(token) = sub.map.get(&rc) {
                    if !matches!(token, Token::None) {
                        return Some(token.clone());
                    }
                }
            }
        }
        None
    }

    fn try_resolve_double_modifier(
        &self,
        section: &crate::types::Section,
        mod1: ScKey,
        mod2: ScKey,
        target: ScKey,
    ) -> Option<Token> {
        let name1 = crate::jis_map::sc_to_key_name(mod1.sc)?;
        let name2 = crate::jis_map::sc_to_key_name(mod2.sc)?;
        // Try <A><B>
        let tag1 = format!("<{}><{}>", name1, name2);
        // eprintln!("DEBUG: Checking tag: {}", tag1);
        if let Some(sub) = section.sub_planes.get(&tag1) {
            // eprintln!("DEBUG: Sub-plane found for {}", tag1);
            if let Some(rc) = self.key_to_rc(target) {
                // eprintln!("DEBUG: RC found for target: {:?}", rc);
                if let Some(token) = sub.map.get(&rc) {
                    // eprintln!("DEBUG: Token found: {:?}", token);
                    if !matches!(token, Token::None) {
                        return Some(token.clone());
                    }
                } // else {
                  //     eprintln!("DEBUG: No token at RC {:?}", rc);
                  // }
            } // else {
              //     eprintln!("DEBUG: No RC for target {:?}", target);
              // }
        } // else {
          //     eprintln!(
          //         "DEBUG: Sub-plane NOT found for {}. Available keys: {:?}",
          //         tag1,
          //         section.sub_planes.keys()
          //     );
          // }
        None
    }

    fn is_char_shift_key(&self, key: ScKey) -> bool {
        self.chord_engine.profile.trigger_keys.contains_key(&key)
    }

    fn deferred_key_can_form_chord_with(
        &self,
        deferred_key: ScKey,
        next_key: ScKey,
        shift: bool,
        is_japanese: bool,
    ) -> bool {
        let (token, modifier) =
            self.resolve_with_modifier(&[deferred_key, next_key], shift, is_japanese);
        token.is_some() && modifier.is_some()
    }

    fn handle_deferred_nonshift_before_event(
        &mut self,
        key: ScKey,
        up: bool,
        shift: bool,
        is_japanese: bool,
    ) {
        if self.pending_nonshift_for_shift.is_empty() {
            return;
        }

        if up {
            if self.pending_nonshift_for_shift.remove(&key) {
                let mut remove = HashSet::new();
                remove.insert(key);
                self.remove_keys_from_pending(&remove, true);
            }
            return;
        }

        let deferred_keys: Vec<ScKey> = self.pending_nonshift_for_shift.iter().copied().collect();
        let has_valid_chord = deferred_keys
            .into_iter()
            .filter(|k| self.chord_engine.state.pressed.contains(k))
            .any(|k| self.deferred_key_can_form_chord_with(k, key, shift, is_japanese));
        if has_valid_chord {
            return;
        }

        let remove: HashSet<ScKey> = self.pending_nonshift_for_shift.drain().collect();
        self.remove_keys_from_pending(&remove, true);
    }

    fn ensure_pending_key(&mut self, key: ScKey) {
        if let Some(p) = self
            .chord_engine
            .state
            .pending
            .iter_mut()
            .find(|p| p.key == key)
        {
            p.t_up = None;
            return;
        }

        let t_down = self
            .chord_engine
            .state
            .down_ts
            .get(&key)
            .copied()
            .unwrap_or_else(Instant::now);

        self.chord_engine.state.pending.push(PendingKey {
            key,
            t_down,
            t_up: None,
        });
    }

    fn remove_keys_from_pending(&mut self, remove: &HashSet<ScKey>, clear_down_ts: bool) {
        if remove.is_empty() {
            return;
        }

        let mut new_pending = Vec::new();
        for p in self.chord_engine.state.pending.iter() {
            if remove.contains(&p.key) {
                if clear_down_ts || !self.chord_engine.state.pressed.contains(&p.key) {
                    self.chord_engine.state.down_ts.remove(&p.key);
                }
                continue;
            }
            new_pending.push(p.clone());
        }
        self.chord_engine.state.pending = new_pending;
    }

    fn consume_non_modifier_keys(&mut self, keys: &[ScKey], keep: ScKey) {
        let mut remove = HashSet::new();
        let continuous = self.chord_engine.profile.char_key_continuous;

        for k in keys {
            if *k == keep {
                continue;
            }

            let is_thumb = self.is_thumb_key(*k);

            if continuous && !is_thumb && self.chord_engine.state.pressed.contains(k) {
                self.pending_nonshift_for_shift.insert(*k);
                self.ensure_pending_key(*k);
                continue;
            }

            remove.insert(*k);
        }

        if remove.is_empty() {
            return;
        }

        self.chord_engine
            .state
            .used_modifiers
            .retain(|k| !remove.contains(k));

        self.remove_keys_from_pending(&remove, false);
    }

    fn key_to_rc(&self, key: ScKey) -> Option<crate::types::Rc> {
        JIS_SC_TO_RC
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, rc)| *rc)
    }

    fn token_to_events(&self, token: &Token, shift_held: bool) -> Option<Vec<InputEvent>> {
        let is_japanese = crate::ime::is_japanese_input_active(self.chord_engine.profile.ime_mode);
        match token {
            Token::None => None,
            Token::KeySequence(seq) => {
                let mut events = Vec::new();
                for stroke in seq {
                    // Strict scancode only for KeySequence (which now comes from single-quote/bare tokens)
                    append_keystroke_events(&mut events, stroke, shift_held, false, is_japanese);
                }
                if events.is_empty() {
                    None
                } else {
                    Some(events)
                }
            }
            Token::ImeChar(text) => {
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
            Token::DirectChar(text) => {
                let mut events = Vec::new();
                // If IME is ON (Japanese Mode), we must temporarily turn it OFF to force "confirmed" input.
                // Otherwise, even Unicode events are intercepted by IME as "unconfirmed" text (e.g. Hiragana).
                let mut toggled_ime = false;
                if is_japanese {
                    if let Ok(ime_on) = crate::ime::get_ime_open_status() {
                        if ime_on {
                            events.push(InputEvent::ImeControl(false));
                            toggled_ime = true;
                        }
                    }
                }

                for c in text.chars() {
                    events.push(InputEvent::Unicode(c, false));
                    events.push(InputEvent::Unicode(c, true));
                }

                if toggled_ime {
                    events.push(InputEvent::ImeControl(true));
                }

                if events.is_empty() {
                    None
                } else {
                    Some(events)
                }
            }
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

    // ...

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
        let (mut keys, consume_pending) =
            if let Some(chord_keys) = self.detect_repeat_chord(key, now) {
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
            let left = tk.left.iter().find(|k| self.is_active_thumb_key(**k));
            let right = tk.right.iter().find(|k| self.is_active_thumb_key(**k));
            let ext1 = tk.ext1.iter().find(|k| self.is_active_thumb_key(**k));
            let ext2 = tk.ext2.iter().find(|k| self.is_active_thumb_key(**k));

            if let Some(k) = left.or(right).or(ext1).or(ext2) {
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
            return tk.left.contains(&key)
                || tk.right.contains(&key)
                || tk.ext1.contains(&key)
                || tk.ext2.contains(&key);
        }
        false
    }

    fn is_active_thumb_key(&self, key: ScKey) -> bool {
        if !self.chord_engine.state.pressed.contains(&key) {
            return false;
        }
        self.chord_engine.state.pending.iter().any(|p| p.key == key)
    }

    fn repeat_allowed_for_token(&self, token: Option<&Token>) -> bool {
        let profile = &self.chord_engine.profile;
        match token {
            Some(t) if Self::is_character_assignment(t) => profile.char_key_repeat_assigned,
            Some(_) => profile.char_key_repeat_unassigned,
            None => profile.char_key_repeat_unassigned,
        }
    }

    fn is_character_assignment(token: &Token) -> bool {
        match token {
            Token::ImeChar(_) | Token::DirectChar(_) => true,
            Token::KeySequence(seq) => {
                !seq.is_empty()
                    && seq.iter().all(|stroke| {
                        stroke.mods.is_empty() && matches!(stroke.key, KeySpec::Char(_))
                    })
            }
            Token::None => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FunctionKeySpec {
    Key(ScKey),
    CapsLock,
    KanaLock,
}

fn passthrough_event(mode: PassThroughCurrent, source_key: ScKey, up: bool) -> Option<InputEvent> {
    match mode {
        PassThroughCurrent::Original => {
            Some(InputEvent::Scancode(source_key.sc, source_key.ext, up))
        }
        PassThroughCurrent::Inject(key) => Some(InputEvent::Scancode(key.sc, key.ext, up)),
        PassThroughCurrent::Block => None,
    }
}

fn passthrough_action(mode: PassThroughCurrent, _source_key: ScKey, up: bool) -> KeyAction {
    match mode {
        PassThroughCurrent::Original => KeyAction::Pass,
        PassThroughCurrent::Inject(key) => {
            KeyAction::Inject(vec![InputEvent::Scancode(key.sc, key.ext, up)])
        }
        PassThroughCurrent::Block => KeyAction::Block,
    }
}

fn emit_pseudo_function_key(pseudo: FunctionPseudoKey, up: bool) -> KeyAction {
    if up {
        return KeyAction::Block;
    }

    let events = match pseudo {
        FunctionPseudoKey::CapsLock => vec![
            InputEvent::Scancode(0x2A, false, false),
            InputEvent::Scancode(0x3A, false, false),
            InputEvent::Scancode(0x3A, false, true),
            InputEvent::Scancode(0x2A, false, true),
        ],
        FunctionPseudoKey::KanaLock => vec![
            InputEvent::Scancode(0x1D, false, false),
            InputEvent::Scancode(0x2A, false, false),
            InputEvent::Scancode(0x70, false, false),
            InputEvent::Scancode(0x70, false, true),
            InputEvent::Scancode(0x2A, false, true),
            InputEvent::Scancode(0x1D, false, true),
        ],
    };
    KeyAction::Inject(events)
}

fn is_virtual_extended_key(key: ScKey) -> bool {
    !key.ext
        && matches!(
            key.sc,
            EXTENDED_KEY_1_SC | EXTENDED_KEY_2_SC | EXTENDED_KEY_3_SC | EXTENDED_KEY_4_SC
        )
}

fn build_function_key_swap_map(
    swaps: &[(String, String)],
) -> HashMap<ScKey, FunctionKeySwapTarget> {
    let mut map = HashMap::new();
    for (source_name, target_name) in swaps {
        let source_spec = match parse_function_key_spec(source_name) {
            Some(spec) => spec,
            None => continue,
        };
        let target_spec = match parse_function_key_spec(target_name) {
            Some(spec) => spec,
            None => continue,
        };

        let source_key = match source_spec {
            FunctionKeySpec::Key(key) => key,
            FunctionKeySpec::CapsLock | FunctionKeySpec::KanaLock => continue,
        };

        let target = match target_spec {
            FunctionKeySpec::Key(key) => FunctionKeySwapTarget::Key(key),
            FunctionKeySpec::CapsLock => FunctionKeySwapTarget::CapsLock,
            FunctionKeySpec::KanaLock => FunctionKeySwapTarget::KanaLock,
        };
        map.insert(source_key, target);
    }
    map
}

fn parse_function_key_spec(name: &str) -> Option<FunctionKeySpec> {
    let key = match name {
        "Esc" => Some(ScKey::new(0x01, false)),
        "Tab" => Some(ScKey::new(0x0F, false)),
        "無変換" => Some(ScKey::new(0x7B, false)),
        "Space" => Some(ScKey::new(0x39, false)),
        "変換" => Some(ScKey::new(0x79, false)),
        "Enter" => Some(ScKey::new(0x1C, false)),
        "BackSpace" => Some(ScKey::new(0x0E, false)),
        "Delete" => Some(ScKey::new(0x53, true)),
        "Insert" => Some(ScKey::new(0x52, true)),
        "左Shift" => Some(ScKey::new(0x2A, false)),
        "右Shift" => Some(ScKey::new(0x36, false)),
        "左Ctrl" => Some(ScKey::new(0x1D, false)),
        "右Ctrl" => Some(ScKey::new(0x1D, true)),
        "左Alt" => Some(ScKey::new(0x38, false)),
        "右Alt" => Some(ScKey::new(0x38, true)),
        "CapsLock/英数" | "CapsLock" => Some(ScKey::new(0x3A, false)),
        "半角/全角" => Some(ScKey::new(0x29, false)),
        "カタカナ/ひらがな" => Some(ScKey::new(0x70, false)),
        "左Win" => Some(ScKey::new(0x5B, true)),
        "右Win" => Some(ScKey::new(0x5C, true)),
        "Applications" => Some(ScKey::new(0x5D, true)),
        "上" => Some(ScKey::new(0x48, true)),
        "左" => Some(ScKey::new(0x4B, true)),
        "右" => Some(ScKey::new(0x4D, true)),
        "下" => Some(ScKey::new(0x50, true)),
        "Home" => Some(ScKey::new(0x47, true)),
        "End" => Some(ScKey::new(0x4F, true)),
        "PageUp" => Some(ScKey::new(0x49, true)),
        "PageDown" => Some(ScKey::new(0x51, true)),
        "拡張1" => Some(ScKey::new(EXTENDED_KEY_1_SC, false)),
        "拡張2" => Some(ScKey::new(EXTENDED_KEY_2_SC, false)),
        "拡張3" => Some(ScKey::new(EXTENDED_KEY_3_SC, false)),
        "拡張4" => Some(ScKey::new(EXTENDED_KEY_4_SC, false)),
        "Capsロック" => return Some(FunctionKeySpec::CapsLock),
        "かなロック" => return Some(FunctionKeySpec::KanaLock),
        _ => function_key_scancode_from_name(name).map(|sc| ScKey::new(sc, false)),
    }?;

    Some(FunctionKeySpec::Key(key))
}

fn function_key_scancode_from_name(name: &str) -> Option<u16> {
    let number = name.strip_prefix('F')?.parse::<u8>().ok()?;
    match number {
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
        13 => Some(0x64),
        14 => Some(0x65),
        15 => Some(0x66),
        16 => Some(0x67),
        17 => Some(0x68),
        18 => Some(0x69),
        19 => Some(0x6A),
        20 => Some(0x6B),
        21 => Some(0x6C),
        22 => Some(0x6D),
        23 => Some(0x6E),
        24 => Some(0x76),
        _ => None,
    }
}

fn append_keystroke_events(
    events: &mut Vec<InputEvent>,
    stroke: &KeyStroke,
    shift_held: bool,
    allow_unicode_fallback: bool,
    is_japanese: bool,
) {
    let key_events = match stroke.key {
        KeySpec::Scancode(sc, ext) => Some((sc, ext, false)),
        KeySpec::VirtualKey(vk) => vk_to_scancode(vk).map(|(s, e)| (s, e, false)),
        KeySpec::Char(c) => char_to_scancode(c, is_japanese),
        KeySpec::ImeOn => {
            events.push(InputEvent::ImeControl(true));
            return;
        }
        KeySpec::ImeOff => {
            events.push(InputEvent::ImeControl(false));
            return;
        }
    };

    if let Some((sc, ext, needs_shift)) = key_events {
        let mut mods = stroke.mods;
        if needs_shift {
            mods.shift = true;
        }

        if mods.shift && shift_held {
            mods.shift = false;
        }

        let mods_evs = modifier_scancodes(mods);
        for (mod_sc, mod_ext) in mods_evs.iter() {
            events.push(InputEvent::Scancode(*mod_sc, *mod_ext, false));
        }
        events.push(InputEvent::Scancode(sc, ext, false));
        events.push(InputEvent::Scancode(sc, ext, true));
        for (mod_sc, mod_ext) in mods_evs.iter().rev() {
            events.push(InputEvent::Scancode(*mod_sc, *mod_ext, true));
        }
        return;
    }

    if allow_unicode_fallback {
        if let KeySpec::Char(c) = stroke.key {
            events.push(InputEvent::Unicode(c, false));
            events.push(InputEvent::Unicode(c, true));
        }
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

fn char_to_scancode(c: char, is_japanese: bool) -> Option<(u16, bool, bool)> {
    // JP-Specific overrides
    if is_japanese {
        match c {
            '、' => return Some((0x33, false, false)), // ,
            '。' => return Some((0x34, false, false)), // .
            '・' => return Some((0x35, false, false)), // /
            '「' => return Some((0x1B, false, false)), // [
            '」' => return Some((0x2B, false, false)), // ]
            _ => {}
        }
    }

    match c {
        // Lowercase
        'a'..='z' => match c {
            'a' => Some((0x1E, false, false)),
            'b' => Some((0x30, false, false)),
            'c' => Some((0x2E, false, false)),
            'd' => Some((0x20, false, false)),
            'e' => Some((0x12, false, false)),
            'f' => Some((0x21, false, false)),
            'g' => Some((0x22, false, false)),
            'h' => Some((0x23, false, false)),
            'i' => Some((0x17, false, false)),
            'j' => Some((0x24, false, false)),
            'k' => Some((0x25, false, false)),
            'l' => Some((0x26, false, false)),
            'm' => Some((0x32, false, false)),
            'n' => Some((0x31, false, false)),
            'o' => Some((0x18, false, false)),
            'p' => Some((0x19, false, false)),
            'q' => Some((0x10, false, false)),
            'r' => Some((0x13, false, false)),
            's' => Some((0x1F, false, false)),
            't' => Some((0x14, false, false)),
            'u' => Some((0x16, false, false)),
            'v' => Some((0x2F, false, false)),
            'w' => Some((0x11, false, false)),
            'x' => Some((0x2D, false, false)),
            'y' => Some((0x15, false, false)),
            'z' => Some((0x2C, false, false)),
            _ => None,
        },
        // Uppercase
        'A'..='Z' => match c.to_ascii_lowercase() {
            'a' => Some((0x1E, false, true)),
            'b' => Some((0x30, false, true)),
            'c' => Some((0x2E, false, true)),
            'd' => Some((0x20, false, true)),
            'e' => Some((0x12, false, true)),
            'f' => Some((0x21, false, true)),
            'g' => Some((0x22, false, true)),
            'h' => Some((0x23, false, true)),
            'i' => Some((0x17, false, true)),
            'j' => Some((0x24, false, true)),
            'k' => Some((0x25, false, true)),
            'l' => Some((0x26, false, true)),
            'm' => Some((0x32, false, true)),
            'n' => Some((0x31, false, true)),
            'o' => Some((0x18, false, true)),
            'p' => Some((0x19, false, true)),
            'q' => Some((0x10, false, true)),
            'r' => Some((0x13, false, true)),
            's' => Some((0x1F, false, true)),
            't' => Some((0x14, false, true)),
            'u' => Some((0x16, false, true)),
            'v' => Some((0x2F, false, true)),
            'w' => Some((0x11, false, true)),
            'x' => Some((0x2D, false, true)),
            'y' => Some((0x15, false, true)),
            'z' => Some((0x2C, false, true)),
            _ => None,
        },
        // Numbers
        '1' => Some((0x02, false, false)),
        '2' => Some((0x03, false, false)),
        '3' => Some((0x04, false, false)),
        '4' => Some((0x05, false, false)),
        '5' => Some((0x06, false, false)),
        '6' => Some((0x07, false, false)),
        '7' => Some((0x08, false, false)),
        '8' => Some((0x09, false, false)),
        '9' => Some((0x0A, false, false)),
        '0' => Some((0x0B, false, false)),

        // Symbols (JIS Standard)
        '-' => Some((0x0C, false, false)),
        '^' => Some((0x0D, false, false)),
        '\\' | '¥' | '￥' => Some((0x7D, false, false)), // Yen (0x7D)
        '@' => Some((0x1A, false, false)),
        '[' => Some((0x1B, false, false)),
        ';' => Some((0x27, false, false)),
        ':' => Some((0x28, false, false)),
        ']' => Some((0x2B, false, false)),
        ',' => Some((0x33, false, false)),
        '.' => Some((0x34, false, false)),
        '/' => Some((0x35, false, false)),
        '_' => Some((0x73, false, true)), // JIS Backslash/Ro (0x73) Shifted

        // Shifted Symbols
        '!' => Some((0x02, false, true)),  // 1
        '"' => Some((0x03, false, true)),  // 2
        '#' => Some((0x04, false, true)),  // 3
        '$' => Some((0x05, false, true)),  // 4
        '%' => Some((0x06, false, true)),  // 5
        '&' => Some((0x07, false, true)),  // 6
        '\'' => Some((0x08, false, true)), // 7
        '(' => Some((0x09, false, true)),  // 8
        ')' => Some((0x0A, false, true)),  // 9
        // 0 -> nothing
        '=' => Some((0x0C, false, true)), // -
        '~' => Some((0x0D, false, true)), // ^
        '|' => Some((0x7D, false, true)), // Yen
        '`' => Some((0x1A, false, true)), // @
        '{' => Some((0x1B, false, true)), // [
        '+' => Some((0x27, false, true)), // ;
        '*' => Some((0x28, false, true)), // :
        '}' => Some((0x2B, false, true)), // ]
        '<' => Some((0x33, false, true)), // ,
        '>' => Some((0x34, false, true)), // .
        '?' => Some((0x35, false, true)), // /

        // Other
        ' ' => Some((0x39, false, false)),
        '\u{0008}' => Some((0x0E, false, false)),  // BS
        '\u{000D}' => Some((0x1C, false, false)),  // Enter
        '\u{F702}' => Some((0x4B, true, false)),   // Left Arrow (Extended)
        '\u{F703}' => Some((0x4D, true, false)),   // Right Arrow (Extended)
        '－' | 'ー' => Some((0x0C, false, false)), // Minus / Long Vowel (Standard Hyphen)

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_to_scancode() {
        // Updated to use 2 args (is_japanese=false) and return 3-tuple (sc, ext, shift)
        assert_eq!(char_to_scancode('－', false), Some((0x0C, false, false)));
        assert_eq!(char_to_scancode('ー', false), Some((0x0C, false, false)));
        assert_eq!(char_to_scancode('1', false), Some((0x02, false, false)));
        assert_eq!(char_to_scancode('a', false), Some((0x1E, false, false)));
        // Shifted char
        assert_eq!(char_to_scancode('!', false), Some((0x02, false, true)));
        // Japanese punctuation
        assert_eq!(char_to_scancode('。', true), Some((0x34, false, false)));
        assert_eq!(char_to_scancode('。', false), None); // Should fallback to unicode if not JP mode scancode mapping
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
        assert_eq!(
            engine.process_key(0x25, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
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
        assert_eq!(
            engine.process_key(0x21, false, false, false),
            KeyAction::Block
        );
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
        assert_eq!(
            engine.process_key(0x25, false, true, false),
            KeyAction::Block
        );
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
        assert_eq!(
            engine.process_key(0x25, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
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
        assert_eq!(
            engine.process_key(0x21, false, false, false),
            KeyAction::Block
        );
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

        assert_eq!(
            engine.process_key(0x25, false, true, false),
            KeyAction::Block
        );
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
        let token = Token::DirectChar("漢".to_string());
        let events = engine
            .token_to_events(&token, false)
            .expect("Should return events");

        assert_eq!(events.len(), 2);
        match events[0] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, '漢');
                assert_eq!(up, false);
            }
            _ => panic!("Expected Unicode down"),
        }
        match events[1] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, '漢');
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
            ext1: HashSet::new(),
            ext2: HashSet::new(),
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
    fn test_nonshift_continues_only_for_next_shift() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,xx,d,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<k>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
x,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<s>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
y,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        engine.set_profile(profile);

        // Hold A, chord with K -> expect "x"
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x25, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x25, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2D, _, _))),
                    "Expected 'x' output for A+K chord"
                );
            }
            _ => panic!("Expected Inject for A+K chord, got {:?}", res),
        }

        // Next key is shift (S) -> A should remain and chord to "y"
        assert_eq!(
            engine.process_key(0x1F, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x1F, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x15, _, _))),
                    "Expected 'y' output for A+S chord"
                );
            }
            _ => panic!("Expected Inject for A+S chord, got {:?}", res),
        }

        // Next key is non-shift (D) -> A should be flushed, only D outputs
        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x20, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x20, _, _))),
                    "Expected 'd' output after flush"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _))),
                    "Did not expect 'a' output after flush"
                );
            }
            _ => panic!("Expected Inject for D tap, got {:?}", res),
        }

        // Release A -> should not emit A
        assert_eq!(
            engine.process_key(0x1E, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_continuous_shift_case4_outputs_ab_then_bc_with_non_trigger_c() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,xx,d,f,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<a>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,x,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<d>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,z,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);

        // A_d - D_d - A_u - F_d - F_u - D_u
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        ); // A down
        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        ); // D down
        assert_eq!(
            engine.process_key(0x1E, false, true, false),
            KeyAction::Block
        ); // A up

        // F down decides A+D by case4 ratio and emits "x".
        let res = engine.process_key(0x21, false, false, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2D, _, _))),
                    "Expected 'x' output for A+D chord"
                );
            }
            _ => panic!("Expected Inject for A+D decision on F down, got {:?}", res),
        }

        // F up resolves D+F and emits "z" even though F is not a trigger key.
        let res = engine.process_key(0x21, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2C, _, _))),
                    "Expected 'z' output for D+F chord"
                );
            }
            _ => panic!("Expected Inject for D+F chord, got {:?}", res),
        }

        assert_eq!(
            engine.process_key(0x20, false, true, false),
            KeyAction::Block
        ); // D up
    }

    #[test]
    fn test_continuous_shift_keeps_non_modifier_even_if_trigger_key() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,f,xx,xx,xx,k,si,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<k>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,mo,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<l>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,ri,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<f>
; R0
無,無,無,無,無,無,無,無,無,無,無,無
; R1
無,無,無,無,無,無,無,無,無,無,無,無
; R2
無,無,無,無,無,無,無,無,無,無,無,無
; R3
無,無,無,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);
        // Hold F, then chord F+K -> "mo"
        assert_eq!(
            engine.process_key(0x21, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x25, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x25, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x32, _, _))),
                    "Expected 'mo' output for F+K chord"
                );
            }
            _ => panic!("Expected Inject for F+K chord, got {:?}", res),
        }

        // Keep holding F, then press L. F must remain pending and resolve with <l> to "ri".
        assert_eq!(
            engine.process_key(0x26, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x26, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x13, _, _))),
                    "Expected 'ri' output for F+L chord"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x1F, _, _))),
                    "Did not expect base 'si' output"
                );
            }
            _ => panic!("Expected Inject for F+L chord, got {:?}", res),
        }

        // Release F -> should not emit extra output.
        assert_eq!(
            engine.process_key(0x21, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_continuous_shift_undefined_rollover_emits_only_later_key() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,t,xx,xx,xx,g,xx,xx,xx
; R2
xx,xx,xx,xx,xx,xx,u,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<o>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,nyu,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,xx,xx,xx,無,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<j>
; R0
無,無,無,無,無,無,無,無,無,無,無,無
; R1
無,無,無,無,無,無,無,無,無,無,無,無
; R2
無,無,無,無,無,無,無,無,無,無,無,無
; R3
無,無,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);

        // T + O => "nyu" (O modifier), keep O physically held.
        assert_eq!(
            engine.process_key(0x14, false, false, false),
            KeyAction::Block
        ); // T down
        assert_eq!(
            engine.process_key(0x18, false, false, false),
            KeyAction::Block
        ); // O down
        let res = engine.process_key(0x14, false, true, false); // T up
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x31, _, _))),
                    "Expected 'nyu' output for T+O chord"
                );
            }
            _ => panic!("Expected Inject for T+O chord, got {:?}", res),
        }

        // O is still down; J rolls over, but O+J mapping is undefined.
        // Only J single output should be emitted (not O single output).
        assert_eq!(
            engine.process_key(0x24, false, false, false),
            KeyAction::Block
        ); // J down
        let res = engine.process_key(0x24, false, true, false); // J up
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x16, _, _))),
                    "Expected only later J output ('u')"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x22, _, _))),
                    "Did not expect older O output ('g')"
                );
            }
            _ => panic!("Expected Inject for J tap, got {:?}", res),
        }

        // O release should not emit base output.
        assert_eq!(
            engine.process_key(0x18, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_continuous_shift_undefined_rollover_when_older_released_first() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,t,xx,xx,xx,g,xx,xx,xx
; R2
xx,xx,xx,xx,xx,xx,u,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<o>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,nyu,xx,xx,xx,xx,xx,xx,xx
; R2
xx,xx,xx,xx,xx,xx,無,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<j>
; R0
無,無,無,無,無,無,無,無,無,無,無,無
; R1
無,無,無,無,無,無,無,無,無,無,無,無
; R2
無,無,無,無,無,無,無,無,無,無,無,無
; R3
無,無,無,無,無,無,無,無,無,無,無
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        engine.set_profile(profile);

        // T + O => "nyu"
        assert_eq!(
            engine.process_key(0x14, false, false, false),
            KeyAction::Block
        ); // T down
        assert_eq!(
            engine.process_key(0x18, false, false, false),
            KeyAction::Block
        ); // O down
        let res = engine.process_key(0x14, false, true, false); // T up
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x31, _, _))),
                    "Expected 'nyu' output for T+O chord"
                );
            }
            _ => panic!("Expected Inject for T+O chord, got {:?}", res),
        }

        // J down while O is held.
        assert_eq!(
            engine.process_key(0x24, false, false, false),
            KeyAction::Block
        );

        // O up comes before J up. This must not emit O single output.
        assert_eq!(
            engine.process_key(0x18, false, true, false),
            KeyAction::Block
        );

        // J up emits only J single output ('u').
        let res = engine.process_key(0x24, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x16, _, _))),
                    "Expected only J output ('u')"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x22, _, _))),
                    "Did not expect O output ('g')"
                );
            }
            _ => panic!("Expected Inject for J tap, got {:?}", res),
        }
    }

    #[test]
    fn test_continuous_shift_undefined_rollover_non_modifier_later_key() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,ku,xx,xx,g,xx,xx,xx
; R2
ri,xx,xx,xx,xx,ku,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<o>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,無,xx,xx,xx,xx,xx,xx
; R2
ryo,xx,xx,xx,xx,無,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);

        // A + O => "ryo"
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        ); // A down
        assert_eq!(
            engine.process_key(0x18, false, false, false),
            KeyAction::Block
        ); // O down
        let res = engine.process_key(0x1E, false, true, false); // A up
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x13, _, _))),
                    "Expected 'ryo' output for A+O chord"
                );
            }
            _ => panic!("Expected Inject for A+O chord, got {:?}", res),
        }

        // H down while O is still held.
        assert_eq!(
            engine.process_key(0x23, false, false, false),
            KeyAction::Block
        );

        // O up before H up should not emit O single output ('g').
        assert_eq!(
            engine.process_key(0x18, false, true, false),
            KeyAction::Block
        );

        // H up emits only "ku" (no leaked 'g').
        let res = engine.process_key(0x23, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x25, _, _))),
                    "Expected 'ku' output on H"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x22, _, _))),
                    "Did not expect O single output ('g')"
                );
            }
            _ => panic!("Expected Inject for H tap, got {:?}", res),
        }
    }

    #[test]
    fn test_continuous_shift_sequential_rollover_does_not_leak_old_modifier_tap() {
        let config = "
[ローマ字シフト無し]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,ku,xx,xx,g,xx,xx,xx
; R2
ri,xx,xx,xx,xx,ku,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<o>
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,無,xx,xx,xx,xx,xx,xx
; R2
ryo,xx,xx,xx,xx,無,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.2;
        engine.set_profile(profile);

        // A + O => "ryo"
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x18, false, false, false),
            KeyAction::Block
        );
        let res = engine.process_key(0x1E, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x13, _, _))),
                    "Expected 'ryo' output for A+O chord"
                );
            }
            _ => panic!("Expected Inject for A+O chord, got {:?}", res),
        }

        // H down while O is held, then quickly O up and later H up to force sequential decision.
        assert_eq!(
            engine.process_key(0x23, false, false, false),
            KeyAction::Block
        );
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(
            engine.process_key(0x18, false, true, false),
            KeyAction::Block
        );
        std::thread::sleep(Duration::from_millis(40));
        let res = engine.process_key(0x23, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x25, _, _))),
                    "Expected 'ku' output on H"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x22, _, _))),
                    "Did not expect O single output ('g')"
                );
            }
            _ => panic!("Expected Inject for H tap, got {:?}", res),
        }
    }

    #[test]
    fn test_function_key_swap_remaps_passthrough_key() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[機能キー]
左Ctrl, 右Ctrl
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        match engine.process_key(0x1D, false, false, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(evs, vec![InputEvent::Scancode(0x1D, true, false)]);
            }
            other => panic!(
                "Expected Inject for remapped LeftCtrl down, got {:?}",
                other
            ),
        }
        match engine.process_key(0x1D, false, true, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(evs, vec![InputEvent::Scancode(0x1D, true, true)]);
            }
            other => panic!("Expected Inject for remapped LeftCtrl up, got {:?}", other),
        }
    }

    #[test]
    fn test_needs_alt_handling_for_function_key_swap_source() {
        let config = "
[機能キー]
左Alt, 拡張1
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert!(
            engine.needs_alt_handling(),
            "Alt should be handled when it is used as [機能キー] swap source"
        );
    }

    #[test]
    fn test_function_key_swap_virtual_extension_without_binding_is_blocked() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[機能キー]
左Alt, 拡張1
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x38, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x38, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_function_key_swap_virtual_extension_can_drive_thumb_shift() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
x,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
z,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[機能キー]
左Alt, 拡張1
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::Extended1;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);

        assert_eq!(
            engine.process_key(0x38, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        let result = engine.process_key(0x1E, false, true, false);
        match result {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2C, _, _))),
                    "Expected 'z' output from left thumb section"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2D, _, _))),
                    "Base 'x' output should not be emitted"
                );
            }
            other => panic!("Expected Inject for mapped thumb chord, got {:?}", other),
        }
        assert_eq!(
            engine.process_key(0x38, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_function_key_swap_virtual_extension_can_drive_extended_thumb_shift_1() {
        let config = "
[拡張親指シフト1]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
z,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[機能キー]
左Alt, 拡張1
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb1.key = crate::chord_engine::ThumbKeySelect::Extended1;
        profile.extended_thumb2.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);

        assert_eq!(
            engine.process_key(0x38, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        let result = engine.process_key(0x1E, false, true, false);
        match result {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2C, _, _))),
                    "Expected 'z' output from [拡張親指シフト1]"
                );
            }
            other => panic!(
                "Expected Inject for mapped extended-thumb chord via function swap, got {:?}",
                other
            ),
        }
        assert_eq!(
            engine.process_key(0x38, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_extended_thumb_prefix_shift_via_function_swap_uses_extended_section_without_base_section(
    ) {
        let extended_thumb_section = "\u{62E1}\u{5F35}\u{89AA}\u{6307}\u{30B7}\u{30D5}\u{30C8}1"; // 拡張親指シフト1
        let function_key_section = "\u{6A5F}\u{80FD}\u{30AD}\u{30FC}"; // 機能キー
        let left_alt = "\u{5DE6}Alt"; // 左Alt
        let ext1 = "\u{62E1}\u{5F35}1"; // 拡張1
        let config = format!(
            "
[{extended_thumb_section}]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
z,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[{function_key_section}]
{left_alt}, {ext1}
"
        );
        let layout = parse_yab_content(&config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb1.key = crate::chord_engine::ThumbKeySelect::Extended1;
        profile.extended_thumb2.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb1.single_press =
            crate::chord_engine::ThumbShiftSinglePress::PrefixShift;
        engine.set_profile(profile);

        // Tap LeftAlt (mapped to virtual Extended1) to arm PrefixShift.
        assert_eq!(
            engine.process_key(0x38, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x38, false, true, false),
            KeyAction::Block
        );

        // Next key should resolve through the extended section even without a base section.
        let result = engine.process_key(0x1E, false, false, false);
        match result {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2C, _, _))),
                    "Expected prefixed output from extended thumb section"
                );
            }
            other => panic!(
                "Expected Inject for prefixed extended-thumb mapping, got {:?}",
                other
            ),
        }
        assert_eq!(
            engine.process_key(0x1E, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_extended_thumb_shift_section_1() {
        let config = "
[拡張親指シフト1]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
z,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb1.key = crate::chord_engine::ThumbKeySelect::Muhenkan;
        profile.extended_thumb2.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);
        engine.load_layout(layout);

        let profile = engine.get_profile();
        let thumbs = profile.thumb_keys.as_ref().expect("thumb keys missing");
        assert!(
            thumbs.ext1.contains(&ScKey::new(0x7B, false)),
            "Muhenkan should be registered as ext thumb 1"
        );

        assert_eq!(
            engine.process_key(0x7B, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        let result = engine.process_key(0x1E, false, true, false);
        match result {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2C, _, _))),
                    "Expected 'z' output from [拡張親指シフト1]"
                );
            }
            other => panic!("Expected Inject for extended thumb 1, got {:?}", other),
        }
        assert_eq!(
            engine.process_key(0x7B, false, true, false),
            KeyAction::Block
        );
    }

    #[test]
    fn test_extended_thumb_shift_section_2() {
        let config = "
[拡張親指シフト2]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
y,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb1.key = crate::chord_engine::ThumbKeySelect::None;
        profile.extended_thumb2.key = crate::chord_engine::ThumbKeySelect::Muhenkan;
        engine.set_profile(profile);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x7B, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        let result = engine.process_key(0x1E, false, true, false);
        match result {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x15, _, _))),
                    "Expected 'y' output from [拡張親指シフト2]"
                );
            }
            other => panic!("Expected Inject for extended thumb 2, got {:?}", other),
        }
        assert_eq!(
            engine.process_key(0x7B, false, true, false),
            KeyAction::Block
        );
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
    #[test]
    fn test_3key_chord_resolution() {
        // Define a layout where <q><w> defines 'a' (0x1E)
        // q=0x10, w=0x11, e=0x12 (target)
        let config = "
[ローマ字シフト無し]
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_overlap_ratio = 0.5; // Require 50% overlap
        engine.set_profile(profile);

        // Simulate Q, W, E down simultaneous-ish
        // T=0: Q down
        engine.process_key(0x10, false, false, false);
        // T=10: W down
        std::thread::sleep(Duration::from_millis(10));
        engine.process_key(0x11, false, false, false);
        // T=20: E down
        std::thread::sleep(Duration::from_millis(10));
        engine.process_key(0x12, false, false, false);

        // T=100: Q Up
        std::thread::sleep(Duration::from_millis(80));
        let res1 = engine.process_key(0x10, false, true, false);

        // T=110: W Up
        std::thread::sleep(Duration::from_millis(10));
        let res2 = engine.process_key(0x11, false, true, false);

        // T=120: E Up
        std::thread::sleep(Duration::from_millis(10));
        let res3 = engine.process_key(0x12, false, true, false);

        // Aggregated events from all releases
        let mut all_events = Vec::new();
        if let KeyAction::Inject(evs) = res1 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = res2 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = res3 {
            all_events.extend(evs);
        }

        assert!(
            all_events
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _))),
            "Expected 'a' output for Q+W+E chord in aggregated events"
        );
        assert!(
            !all_events
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x10, _, _))),
            "Should not output q"
        );
    }

    #[test]
    fn test_mixed_2key_and_3key_definitions() {
        // q = 0x10, w = 0x11, e = 0x12
        // Layout:
        // <q>
        // xx,2,xx... (row 0, col 1 is 'w' position -> outputs '2')
        // <q><w>
        // xx,xx,3... (row 0, col 2 is 'e' position -> outputs '3')

        let config = "
[英数シフト無し]
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字シフト無し]
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q>
xx,2,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,3,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true); // Force consistent behavior if possible, but sections cover both.
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_overlap_ratio = 0.5;
        engine.set_profile(profile);

        // Case 1: 3-key chord (q + w + e) -> '3' (0x04)
        // q down
        engine.process_key(0x10, false, false, false);
        std::thread::sleep(Duration::from_millis(10));
        // w down
        engine.process_key(0x11, false, false, false);
        std::thread::sleep(Duration::from_millis(10));
        // e down
        engine.process_key(0x12, false, false, false);

        // Release
        std::thread::sleep(Duration::from_millis(100)); // wait for overlap
        let r1 = engine.process_key(0x10, false, true, false);
        let r2 = engine.process_key(0x11, false, true, false);
        let r3 = engine.process_key(0x12, false, true, false);

        let mut events = Vec::new();
        if let KeyAction::Inject(evs) = r1 {
            events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r2 {
            events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r3 {
            events.extend(evs);
        }

        eprintln!("DEBUG: events: {:?}", events);
        if !events
            .iter()
            .any(|e| matches!(e, InputEvent::Scancode(0x04, _, _)))
        {
            panic!("DEBUG: Expected '3' (0x04) but got events: {:?}", events);
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x03, _, _))),
            "Should NOT output '2' (0x03) for q+w+e"
        );

        // Case 2: 2-key chord (q + w) -> '2' (0x03)
        std::thread::sleep(Duration::from_millis(500));

        // q down
        engine.process_key(0x10, false, false, false);
        std::thread::sleep(Duration::from_millis(10));
        // w down
        engine.process_key(0x11, false, false, false);

        std::thread::sleep(Duration::from_millis(100));
        let r1 = engine.process_key(0x10, false, true, false);
        let r2 = engine.process_key(0x11, false, true, false);

        let mut events2 = Vec::new();
        if let KeyAction::Inject(evs) = r1 {
            events2.extend(evs);
        }
        if let KeyAction::Inject(evs) = r2 {
            events2.extend(evs);
        }

        assert!(
            events2
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x03, _, _))),
            "Expected '2' (0x03) for q+w"
        );
        assert!(
            !events2
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x04, _, _))),
            "Should NOT output '3'"
        );
    }

    #[test]
    fn test_ime_control_keys() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,日,英,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_yab_content(config).expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // '日' is at row 1, col 1 -> 'w' position (0x11)
        // Down (Buffered)
        assert_eq!(
            engine.process_key(0x11, false, false, false),
            KeyAction::Block
        );
        // Up (Inject)
        let res = engine.process_key(0x11, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 1);
                assert!(matches!(evs[0], InputEvent::ImeControl(true)));
            }
            _ => panic!("Expected Inject(ImeControl(true)) on Up, got {:?}", res),
        }

        // '英' is at row 1, col 2 -> 'e' position (0x12)
        // Down (Buffered)
        assert_eq!(
            engine.process_key(0x12, false, false, false),
            KeyAction::Block
        );
        // Up (Inject)
        let res = engine.process_key(0x12, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 1);
                assert!(matches!(evs[0], InputEvent::ImeControl(false)));
            }
            _ => panic!("Expected Inject(ImeControl(false)) on Up, got {:?}", res),
        }
    }
}
