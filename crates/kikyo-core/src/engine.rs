use crate::analytics::AnalyticsCollector;
use crate::chord_engine::{
    ChordEngine, Decision, ImeMode, KeyEdge, KeyEvent, PendingKey, Profile, ThumbShiftSinglePress,
    EXTENDED_KEY_1_SC, EXTENDED_KEY_2_SC, EXTENDED_KEY_3_SC, EXTENDED_KEY_4_SC,
};
use crate::keyboard_map::KeyboardMap;
use crate::types::{InputEvent, KeyAction, KeySpec, KeyStroke, Layout, Modifiers, ScKey, Token};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

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
    started_at: Instant,
}

const EXTENDED_THUMB_SHIFT_1_SECTION: &str =
    "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}1";
const EXTENDED_THUMB_SHIFT_2_SECTION: &str =
    "\u{62e1}\u{5f35}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}2";
const ROMAJI_SECTION_PREFIX: &str = "\u{30ed}\u{30fc}\u{30de}\u{5b57}";
const KANA_SECTION_PREFIX: &str = "\u{304b}\u{306a}";
const ALPHA_SECTION_PREFIX: &str = "\u{82f1}\u{6570}";
const NO_SHIFT_SUFFIX: &str = "\u{30b7}\u{30d5}\u{30c8}\u{7121}\u{3057}";
const LEFT_THUMB_SHIFT_SUFFIX: &str = "\u{5de6}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}";
const RIGHT_THUMB_SHIFT_SUFFIX: &str = "\u{53f3}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}";
const PINKY_SHIFT_SUFFIX: &str = "\u{5c0f}\u{6307}\u{30b7}\u{30d5}\u{30c8}";
const PINKY_LEFT_THUMB_SHIFT_SUFFIX: &str =
    "\u{5c0f}\u{6307}\u{5de6}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}";
const PINKY_RIGHT_THUMB_SHIFT_SUFFIX: &str =
    "\u{5c0f}\u{6307}\u{53f3}\u{89aa}\u{6307}\u{30b7}\u{30d5}\u{30c8}";
const JAPANESE_PINKY_SHIFT_SUFFIXES: [&str; 3] = [
    PINKY_SHIFT_SUFFIX,
    PINKY_LEFT_THUMB_SHIFT_SUFFIX,
    PINKY_RIGHT_THUMB_SHIFT_SUFFIX,
];
const DEFERRED_ENTER_RECOVERY_TIMEOUT_MS: u64 = 1000;

thread_local! {
    static SECTION_NAME_SCRATCH: RefCell<String> = RefCell::new(String::with_capacity(64));
    static TAG_NAME_SCRATCH: RefCell<String> = RefCell::new(String::with_capacity(32));
    static DOUBLE_TAG_NAME_SCRATCH: RefCell<String> = RefCell::new(String::with_capacity(48));
}

fn with_section_name<T>(prefix: &str, suffix: &str, f: impl FnOnce(&str) -> T) -> T {
    SECTION_NAME_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.push_str(prefix);
        buf.push_str(suffix);
        f(buf.as_str())
    })
}

fn with_single_tag<T>(name: &str, f: impl FnOnce(&str) -> T) -> T {
    TAG_NAME_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.push('<');
        buf.push_str(name);
        buf.push('>');
        f(buf.as_str())
    })
}

fn with_double_tag<T>(name1: &str, name2: &str, f: impl FnOnce(&str) -> T) -> T {
    DOUBLE_TAG_NAME_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.push('<');
        buf.push_str(name1);
        buf.push('>');
        buf.push('<');
        buf.push_str(name2);
        buf.push('>');
        f(buf.as_str())
    })
}

pub struct Engine {
    chord_engine: ChordEngine,
    enabled: bool,
    layout: Option<Layout>,
    on_enabled_change: Option<Box<dyn Fn(bool) + Send + Sync>>,
    on_settings_shortcut: Option<Box<dyn Fn() + Send + Sync>>,
    on_switch_layout_shortcut: Option<Box<dyn Fn() + Send + Sync>>,
    repeat_plans: HashMap<ScKey, Vec<ScKey>>,
    pending_nonshift_for_shift: HashSet<ScKey>,
    passthrough_thumb_shift_modifiers: HashMap<ScKey, ScKey>,
    function_key_swaps: HashMap<ScKey, FunctionKeySwapTarget>,
    deferred_enter_rollover: Option<DeferredEnterRollover>,
    pub keyboard_map: Arc<KeyboardMap>,
    pub analytics: AnalyticsCollector,
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
            on_settings_shortcut: None,
            on_switch_layout_shortcut: None,
            repeat_plans: HashMap::new(),
            pending_nonshift_for_shift: HashSet::new(),
            passthrough_thumb_shift_modifiers: HashMap::new(),
            function_key_swaps: HashMap::new(),
            deferred_enter_rollover: None,
            keyboard_map: Arc::new(crate::keyboard_map::new_jis_106()),
            analytics: AnalyticsCollector::new(),
        }
    }
}

impl Engine {
    pub fn set_keyboard_map(&mut self, map: Arc<KeyboardMap>) {
        self.keyboard_map = map;
        if let Some(layout) = self.layout.clone() {
            self.load_layout(layout);
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                // Reset state without discarding the user's profile.
                let profile = self.chord_engine.profile.clone();
                self.chord_engine = ChordEngine::new(profile);
                self.repeat_plans.clear();
                self.pending_nonshift_for_shift.clear();
                self.passthrough_thumb_shift_modifiers.clear();
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

    pub fn set_on_settings_shortcut(&mut self, cb: impl Fn() + Send + Sync + 'static) {
        self.on_settings_shortcut = Some(Box::new(cb));
    }

    pub fn set_on_switch_layout_shortcut(&mut self, cb: impl Fn() + Send + Sync + 'static) {
        self.on_switch_layout_shortcut = Some(Box::new(cb));
    }

    pub fn trigger_settings_shortcut(&self) {
        if let Some(ref cb) = self.on_settings_shortcut {
            cb();
        }
    }

    pub fn trigger_switch_layout_shortcut(&self) {
        if let Some(ref cb) = self.on_switch_layout_shortcut {
            cb();
        }
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

    pub fn get_suspend_shortcut(&self) -> Option<crate::types::ShortcutKey> {
        self.chord_engine.profile.suspend_shortcut.clone()
    }

    pub fn get_settings_shortcut(&self) -> Option<crate::types::ShortcutKey> {
        self.chord_engine.profile.settings_shortcut.clone()
    }

    pub fn get_switch_layout_shortcut(&self) -> Option<crate::types::ShortcutKey> {
        self.chord_engine.profile.switch_layout_shortcut.clone()
    }

    pub fn needs_alt_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x38, false), ScKey::new(0x38, true)])
    }

    pub fn needs_left_shift_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x2A, false)])
    }

    pub fn needs_right_shift_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x36, false)])
    }

    fn has_japanese_section_with_suffix(layout: &Layout, suffix: &str) -> bool {
        with_section_name(ROMAJI_SECTION_PREFIX, suffix, |section_name| {
            layout.sections.contains_key(section_name)
        }) || with_section_name(KANA_SECTION_PREFIX, suffix, |section_name| {
            layout.sections.contains_key(section_name)
        })
    }

    fn active_section_by_suffix<'a>(
        &self,
        layout: &'a Layout,
        suffix: &str,
        is_japanese: bool,
        forced_section_name: Option<&str>,
    ) -> Option<&'a crate::types::Section> {
        if let Some(section_name) = forced_section_name {
            return layout.sections.get(section_name);
        }

        if is_japanese {
            if let Some(section) =
                with_section_name(ROMAJI_SECTION_PREFIX, suffix, |section_name| {
                    layout.sections.get(section_name)
                })
            {
                return Some(section);
            }
            return with_section_name(KANA_SECTION_PREFIX, suffix, |section_name| {
                layout.sections.get(section_name)
            });
        }

        with_section_name(ALPHA_SECTION_PREFIX, suffix, |section_name| {
            layout.sections.get(section_name)
        })
    }

    fn has_romaji_pinky_shift_section_in_layout(&self) -> bool {
        self.layout.as_ref().is_some_and(|layout| {
            JAPANESE_PINKY_SHIFT_SUFFIXES
                .iter()
                .any(|suffix| Self::has_japanese_section_with_suffix(layout, suffix))
        })
    }

    pub fn capture_left_shift_for_romaji_pinky_shift(&self) -> bool {
        if !self.enabled {
            return false;
        }
        self.has_romaji_pinky_shift_section_in_layout()
            && !self.needs_modifier_handling(&[ScKey::new(0x2A, false)])
    }

    pub fn capture_right_shift_for_romaji_pinky_shift(&self) -> bool {
        if !self.enabled {
            return false;
        }
        self.has_romaji_pinky_shift_section_in_layout()
            && !self.needs_modifier_handling(&[ScKey::new(0x36, false)])
    }

    fn needs_modifier_handling(&self, keys: &[ScKey]) -> bool {
        if !self.enabled {
            return false;
        }
        for k in keys {
            if self.function_key_swaps.contains_key(k) {
                return true;
            }
            if let Some(ref tk) = self.chord_engine.profile.thumb_keys {
                if tk.left.contains(k)
                    || tk.right.contains(k)
                    || tk.ext1.contains(k)
                    || tk.ext2.contains(k)
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn needs_left_ctrl_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x1D, false)])
    }

    pub fn needs_right_ctrl_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x1D, true)])
    }

    pub fn needs_left_win_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x5B, true)])
    }

    pub fn needs_right_win_handling(&self) -> bool {
        self.needs_modifier_handling(&[ScKey::new(0x5C, true)])
    }

    fn has_thumb_shift_sections_in_layout(&self) -> bool {
        if let Some(ref layout) = self.layout {
            let targets = [
                "ローマ字左親指シフト",
                "ローマ字右親指シフト",
                "ローマ字小指左親指シフト",
                "ローマ字小指右親指シフト",
                "かな左親指シフト",
                "かな右親指シフト",
                "かな小指左親指シフト",
                "かな小指右親指シフト",
                "英数左親指シフト",
                "英数右親指シフト",
                "英数小指左親指シフト",
                "英数小指右親指シフト",
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

        self.passthrough_thumb_shift_modifiers.clear();
        self.chord_engine.set_profile(profile);
    }

    pub fn load_layout(&mut self, layout: Layout) {
        tracing::info!(
            "Engine: Layout loaded with {} sections.",
            layout.sections.len()
        );
        self.function_key_swaps = build_function_key_swap_map(&layout.function_key_swaps);

        let mut profile = self.chord_engine.profile.clone();
        profile.max_chord_size = if layout.max_chord_size >= 3 { 3 } else { 2 };

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
        for (&sckey, &rc) in self.keyboard_map.sc_to_rc.iter() {
            let sc = sckey.sc;
            if active_rcs.contains(&rc) {
                target_keys.insert(ScKey::new(sc, false));
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
                    if let Some(sc) = self.keyboard_map.key_name_to_sc(inner) {
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
                        if let Some(sc) = self.keyboard_map.key_name_to_sc(inner) {
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

    pub fn clear_layout(&mut self) {
        tracing::info!("Engine: Layout cleared.");
        self.layout = None;
        self.function_key_swaps.clear();

        let mut profile = self.chord_engine.profile.clone();
        profile.max_chord_size = 2; // Default back to 2
        profile.trigger_keys.clear();
        profile.target_keys = None;
        self.set_profile(profile);
    }

    pub fn process_key(&mut self, sc: u16, ext: bool, up: bool, shift: bool) -> KeyAction {
        if !self.enabled {
            return KeyAction::Pass;
        }

        // Check IME state
        let ime_mode = self.chord_engine.profile.ime_mode;
        let is_japanese = crate::ime::is_japanese_input_active(ime_mode);
        let is_kana_input = crate::ime::is_kana_input_active(ime_mode);
        // ...

        if self.layout.is_none() {
            return KeyAction::Pass;
        }

        let source_key = ScKey::new(sc, ext);
        let (key, pass_through_current, pseudo_key) = self.remap_input_key(source_key);
        if let Some(pseudo) = pseudo_key {
            return emit_pseudo_function_key(pseudo, up);
        }

        self.recover_stale_deferred_enter_rollover(key, up);

        if let Some(action) =
            self.handle_deferred_enter_event(source_key, key, pass_through_current, up)
        {
            return action;
        }

        if !up && self.is_repeat_event(key) {
            return self.handle_repeat_event(key, shift, is_japanese, is_kana_input);
        }

        self.refresh_active_char_shift_keys(shift, is_japanese);
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

            // 2. Select active section suffix
            let suffix = if shift {
                if has_left_thumb {
                    PINKY_LEFT_THUMB_SHIFT_SUFFIX
                } else if has_right_thumb {
                    PINKY_RIGHT_THUMB_SHIFT_SUFFIX
                } else {
                    PINKY_SHIFT_SUFFIX
                }
            } else {
                if has_left_thumb {
                    LEFT_THUMB_SHIFT_SUFFIX
                } else if has_right_thumb {
                    RIGHT_THUMB_SHIFT_SUFFIX
                } else {
                    NO_SHIFT_SUFFIX
                }
            };

            let forced_section_name =
                if is_japanese && !has_left_thumb && !has_right_thumb && has_ext1_thumb {
                    Some(EXTENDED_THUMB_SHIFT_1_SECTION)
                } else if is_japanese && !has_left_thumb && !has_right_thumb && has_ext2_thumb {
                    Some(EXTENDED_THUMB_SHIFT_2_SECTION)
                } else {
                    None
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

                let section =
                    self.active_section_by_suffix(layout, suffix, is_japanese, forced_section_name);

                if let Some(section) = section {
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
                        if let Some(name) = self.keyboard_map.sc_to_key_name(key.sc) {
                            if with_single_tag(name, |tag| section.sub_planes.contains_key(tag)) {
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
                        return self.passthrough_with_thumb_shift_fallback(
                            source_key,
                            key,
                            pass_through_current,
                            up,
                        );
                    }

                    let is_pinky_shift = JAPANESE_PINKY_SHIFT_SUFFIXES.contains(&suffix);
                    if is_defined && !is_thumb && !is_space && !up && shift && is_pinky_shift {
                        if let Some(token) = self.resolve(&[key], shift, is_japanese) {
                            if let Some(ops) = self.token_to_events_with_ime(
                                &token,
                                shift,
                                is_japanese,
                                is_kana_input,
                            ) {
                                self.record_analytics_replacement(&[key], &ops);
                                self.chord_engine.state.pressed.insert(key);
                                self.chord_engine.state.down_ts.insert(key, Instant::now());
                                return KeyAction::Inject(ops);
                            }
                        }
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
                        return self.passthrough_with_thumb_shift_fallback(
                            source_key,
                            key,
                            pass_through_current,
                            up,
                        );
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

        let prev_require_modifier = self.chord_engine.profile.require_modifier_for_char_chord;
        let prev_max_chord_size = self.chord_engine.profile.max_chord_size;
        self.chord_engine.profile.require_modifier_for_char_chord = !is_japanese;
        if !is_japanese {
            self.chord_engine.profile.max_chord_size =
                self.chord_engine.profile.max_chord_size.min(2);
        }
        let decisions = self.chord_engine.on_event(event);
        self.chord_engine.profile.require_modifier_for_char_chord = prev_require_modifier;
        self.chord_engine.profile.max_chord_size = prev_max_chord_size;

        let mut inject_ops = Vec::new();
        let mut analytics_ops = Vec::new();
        let mut analytics_physical_keys = Vec::new();
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
                        if let Some(ops) =
                            self.token_to_events_with_ime(&token, shift, is_japanese, is_kana_input)
                        {
                            analytics_physical_keys.push(k);
                            analytics_ops.extend(ops.iter().cloned());
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
                        if let Some(ops) =
                            self.token_to_events_with_ime(&token, shift, is_japanese, is_kana_input)
                        {
                            analytics_physical_keys.extend(keys.iter().copied());
                            analytics_ops.extend(ops.iter().cloned());
                            inject_ops.extend(ops);
                        }
                        if let Some(mod_key) = modifier {
                            self.consume_non_modifier_keys(&keys, mod_key);
                        }
                    } else {
                        // 3-key undefined chord with single CharShift modifier -> split into
                        // sequential 2-key chords. Common case: a continuous-shift modifier
                        // key M held while two non-modifier keys roll (e.g. K1 then K2). With
                        // M+K1+K2 undefined but M+K1 and M+K2 each defined, emit the two
                        // resolved 2-key chords instead of falling back to the three base keys.
                        //
                        // For unresolved 2-key sub-chords, falls back to the non-modifier's
                        // base single-key resolution (preserving partial behavior).
                        let mut handled_as_3key_split = false;
                        if self.chord_engine.profile.char_key_continuous && keys.len() == 3 {
                            // Try each of the 3 keys as the candidate continuous modifier.
                            // The "best" split is the one that resolves the most 2-key chord
                            // sub-pairs. This handles the case where multiple keys are also
                            // chord trigger keys (e.g. with `<v><;>` and `<v><l>` planes,
                            // ; and L are both trigger_keys, so we can't identify the
                            // modifier by `is_char_shift_key` alone).
                            let mut best_split: Option<(
                                usize,           // m_idx
                                Vec<InputEvent>, // ops
                                Vec<ScKey>,      // emitted physical keys
                                usize,           // chord-resolved count
                            )> = None;
                            for m_idx in 0..keys.len() {
                                let modifier = keys[m_idx];
                                if !self.is_char_shift_key(modifier) {
                                    continue;
                                }
                                let others: Vec<ScKey> = keys
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != m_idx)
                                    .map(|(_, k)| *k)
                                    .collect();
                                let mut split_ops: Vec<InputEvent> = Vec::new();
                                let mut emit_keys: Vec<ScKey> = Vec::new();
                                let mut chord_resolved_count: usize = 0;
                                for &other in &others {
                                    let pair = vec![modifier, other];
                                    let (token, _) =
                                        self.resolve_with_modifier(&pair, shift, is_japanese);
                                    if let Some(token) = token {
                                        if let Some(ops) = self.token_to_events_with_ime(
                                            &token,
                                            shift,
                                            is_japanese,
                                            is_kana_input,
                                        ) {
                                            split_ops.extend(ops);
                                            emit_keys.push(other);
                                            chord_resolved_count += 1;
                                            continue;
                                        }
                                    }
                                    if let Some(token) = self.resolve(&[other], shift, is_japanese)
                                    {
                                        if let Some(ops) = self.token_to_events_with_ime(
                                            &token,
                                            shift,
                                            is_japanese,
                                            is_kana_input,
                                        ) {
                                            split_ops.extend(ops);
                                            emit_keys.push(other);
                                        }
                                    }
                                }
                                // Keep the candidate that resolves the most
                                // 2-key sub-chords. Strictly-greater means a tie
                                // keeps the first-encountered candidate (lowest
                                // key index); in practice only one of the three
                                // keys is a char-shift key, so ties are rare and
                                // this choice stays deterministic.
                                if chord_resolved_count > 0 {
                                    let take = match &best_split {
                                        None => true,
                                        Some((_, _, _, prev_count)) => {
                                            chord_resolved_count > *prev_count
                                        }
                                    };
                                    if take {
                                        best_split = Some((
                                            m_idx,
                                            split_ops,
                                            emit_keys,
                                            chord_resolved_count,
                                        ));
                                    }
                                }
                            }
                            if let Some((m_idx, split_ops, emit_keys, _)) = best_split {
                                let modifier = keys[m_idx];
                                analytics_physical_keys.extend(emit_keys.iter().copied());
                                analytics_ops.extend(split_ops.iter().cloned());
                                inject_ops.extend(split_ops);
                                // The split has already emitted output for these
                                // non-modifier keys. Unlike the normal
                                // continuous-shift consume path, they must NOT be
                                // re-queued into pending: a still-held key would
                                // otherwise re-chord on the next keypress and be
                                // emitted a second time. Remove every non-modifier
                                // key from pending unconditionally and keep only
                                // the modifier (so its used_modifiers entry is
                                // preserved).
                                //
                                // Defensive hardening: even if a 3-key
                                // undefined chord reaches this path (e.g.
                                // three overlapping char-shift keys with
                                // no 3-key mapping), duplicate consumption
                                // cannot occur. This split path is entered
                                // only when the chord resolves to no token
                                // (the `else` of the `Some(token)` arm
                                // above), which is mutually exclusive with
                                // the `consume_non_modifier_keys` call on
                                // the token-resolved arm. Removing the
                                // non-modifier keys here keeps split's
                                // consume semantics consistent with that
                                // arm and prevents a still-held key from
                                // being re-queued into pending and
                                // double-emitted on the next keypress.
                                let mut remove_set = HashSet::new();
                                for k in &keys {
                                    if *k != modifier {
                                        remove_set.insert(*k);
                                        self.pending_nonshift_for_shift.remove(k);
                                    }
                                }
                                self.chord_engine
                                    .state
                                    .used_modifiers
                                    .retain(|k| !remove_set.contains(k));
                                self.remove_keys_from_pending(&remove_set, false);
                                handled_as_3key_split = true;
                            }
                        }
                        if handled_as_3key_split {
                            continue;
                        }
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

                        if undefined_rollover_pair
                            && older_pressed
                            && !newer_pressed
                            && older_is_continuous_used_modifier
                        {
                            let k = keys[1];
                            self.chord_engine.state.used_modifiers.remove(&k);
                            let mut resolved = false;
                            if let Some(token) = self.resolve(&[k], shift, is_japanese) {
                                if let Some(ops) = self.token_to_events_with_ime(
                                    &token,
                                    shift,
                                    is_japanese,
                                    is_kana_input,
                                ) {
                                    analytics_physical_keys.push(k);
                                    analytics_ops.extend(ops.iter().cloned());
                                    inject_ops.extend(ops);
                                    resolved = true;
                                }
                            }
                            if !resolved {
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, false));
                                inject_ops.push(InputEvent::Scancode(k.sc, k.ext, true));
                            }
                        } else if undefined_rollover_pair
                            && !older_pressed
                            && newer_pressed
                            && older_is_continuous_used_modifier
                        {
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
                                if let Some(ops) = self.token_to_events_with_ime(
                                    &token,
                                    shift,
                                    is_japanese,
                                    is_kana_input,
                                ) {
                                    analytics_physical_keys.push(k);
                                    analytics_ops.extend(ops.iter().cloned());
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
                                    if let Some(ops) = self.token_to_events_with_ime(
                                        &token,
                                        shift,
                                        is_japanese,
                                        is_kana_input,
                                    ) {
                                        analytics_physical_keys.push(k);
                                        analytics_ops.extend(ops.iter().cloned());
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
            self.record_analytics_replacement(&analytics_physical_keys, &analytics_ops);

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

    fn record_analytics_replacement(
        &mut self,
        physical_keys: &[ScKey],
        output_events: &[InputEvent],
    ) {
        let output_count =
            crate::analytics::count_output_virtual_keys(output_events, &self.keyboard_map);
        if output_count == 0 {
            return;
        }

        for key in physical_keys {
            let sc_key = ScKey::new(key.sc, false);
            if self.keyboard_map.sc_to_rc.contains_key(&sc_key) {
                let key_name = self
                    .keyboard_map
                    .sc_to_key_name(key.sc)
                    .map(|s| s.to_string());
                self.analytics
                    .record_physical_keystroke(key_name.as_deref());
            }
        }
        self.analytics.record_output_virtual_keys(output_count);
    }

    fn active_shift_thumb_for_passthrough(&self) -> Option<ScKey> {
        let left_shift = ScKey::new(0x2A, false);
        if self.is_thumb_key(left_shift) && self.chord_engine.state.pressed.contains(&left_shift) {
            return Some(left_shift);
        }

        let right_shift = ScKey::new(0x36, false);
        if self.is_thumb_key(right_shift) && self.chord_engine.state.pressed.contains(&right_shift)
        {
            return Some(right_shift);
        }

        None
    }

    fn passthrough_with_thumb_shift_fallback(
        &mut self,
        source_key: ScKey,
        key: ScKey,
        pass_through_current: PassThroughCurrent,
        up: bool,
    ) -> KeyAction {
        if up {
            if let Some(shift_key) = self.passthrough_thumb_shift_modifiers.remove(&key) {
                let mut events = Vec::new();
                if let Some(event) = passthrough_event(pass_through_current, source_key, true) {
                    events.push(event);
                }
                events.push(InputEvent::Scancode(shift_key.sc, shift_key.ext, true));
                return KeyAction::Inject(events);
            }
            return passthrough_action(pass_through_current, source_key, true);
        }

        if self.key_to_rc(key).is_some() {
            return passthrough_action(pass_through_current, source_key, false);
        }

        let Some(shift_key) = self.active_shift_thumb_for_passthrough() else {
            return passthrough_action(pass_through_current, source_key, false);
        };

        let Some(current_event) = passthrough_event(pass_through_current, source_key, false) else {
            return KeyAction::Block;
        };

        if self.passthrough_thumb_shift_modifiers.contains_key(&key) {
            return KeyAction::Inject(vec![current_event]);
        }

        self.passthrough_thumb_shift_modifiers
            .insert(key, shift_key);
        KeyAction::Inject(vec![
            InputEvent::Scancode(shift_key.sc, shift_key.ext, false),
            current_event,
        ])
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

    fn recover_stale_deferred_enter_rollover(&mut self, current_key: ScKey, up: bool) {
        let Some(deferred) = self.deferred_enter_rollover else {
            return;
        };

        if deferred.down_emitted {
            return;
        }

        // The waited key's own KeyUp is the normal release path for the deferred Enter.
        // Recovering here would purge the pending tap before it can resolve.
        if deferred.wait_for == current_key && up {
            return;
        }

        if !self.should_recover_deferred_enter_rollover(deferred) {
            return;
        }

        self.purge_stale_key_from_state(deferred.wait_for);
        self.deferred_enter_rollover = None;
    }

    fn should_recover_deferred_enter_rollover(&self, deferred: DeferredEnterRollover) -> bool {
        if let Some(wait_for_down) = Self::is_sc_key_physically_down(deferred.wait_for) {
            return !wait_for_down;
        }

        deferred.started_at.elapsed().as_millis() as u64 >= DEFERRED_ENTER_RECOVERY_TIMEOUT_MS
    }

    fn is_sc_key_physically_down(key: ScKey) -> Option<bool> {
        crate::keyboard_hook::is_sc_key_physically_down(key)
    }

    fn purge_stale_key_from_state(&mut self, key: ScKey) {
        self.chord_engine.state.pressed.remove(&key);
        self.chord_engine.state.down_ts.remove(&key);
        self.chord_engine.state.pending.retain(|p| p.key != key);
        self.chord_engine.state.passed_keys.remove(&key);
        self.chord_engine.state.used_modifiers.remove(&key);
        self.pending_nonshift_for_shift.remove(&key);
        self.repeat_plans.remove(&key);
        self.passthrough_thumb_shift_modifiers
            .retain(|k, v| *k != key && *v != key);
        if self.chord_engine.state.prefix_pending == Some(key) {
            self.chord_engine.state.prefix_pending = None;
        }
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
            started_at: Instant::now(),
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

        // 2. Select SUFFIX
        let suffix = if shift {
            if has_left_thumb {
                PINKY_LEFT_THUMB_SHIFT_SUFFIX
            } else if has_right_thumb {
                PINKY_RIGHT_THUMB_SHIFT_SUFFIX
            } else {
                PINKY_SHIFT_SUFFIX
            }
        } else {
            if has_left_thumb {
                LEFT_THUMB_SHIFT_SUFFIX
            } else if has_right_thumb {
                RIGHT_THUMB_SHIFT_SUFFIX
            } else {
                NO_SHIFT_SUFFIX
            }
        };

        let forced_section_name =
            if is_japanese && !has_left_thumb && !has_right_thumb && has_ext1_thumb {
                Some(EXTENDED_THUMB_SHIFT_1_SECTION)
            } else if is_japanese && !has_left_thumb && !has_right_thumb && has_ext2_thumb {
                Some(EXTENDED_THUMB_SHIFT_2_SECTION)
            } else {
                None
            };
        // eprintln!("DEBUG: Resolve: section={} keys={:?} japanese={}", section_name, keys, is_japanese);

        let section =
            match self.active_section_by_suffix(layout, suffix, is_japanese, forced_section_name) {
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
        let mod_name = self.keyboard_map.sc_to_key_name(mod_key.sc)?;
        with_single_tag(mod_name, |tag| {
            if let Some(sub) = section.sub_planes.get(tag) {
                if let Some(rc) = self.key_to_rc(target_key) {
                    if let Some(token) = sub.map.get(&rc) {
                        if !matches!(token, Token::None) {
                            return Some(token.clone());
                        }
                    }
                }
            }
            None
        })
    }

    fn try_resolve_double_modifier(
        &self,
        section: &crate::types::Section,
        mod1: ScKey,
        mod2: ScKey,
        target: ScKey,
    ) -> Option<Token> {
        let name1 = self.keyboard_map.sc_to_key_name(mod1.sc)?;
        let name2 = self.keyboard_map.sc_to_key_name(mod2.sc)?;
        with_double_tag(name1, name2, |tag1| {
            // eprintln!("DEBUG: Checking tag: {}", tag1);
            if let Some(sub) = section.sub_planes.get(tag1) {
                // eprintln!("DEBUG: Sub-plane found for {}", tag1);
                if let Some(rc) = self.key_to_rc(target) {
                    // eprintln!("DEBUG: RC found for target: {:?}", rc);
                    if let Some(token) = sub.map.get(&rc) {
                        // eprintln!("DEBUG: Token found: {:?}", token);
                        if !matches!(token, Token::None) {
                            return Some(token.clone());
                        }
                    }
                } // else {
                  //     eprintln!("DEBUG: No token at RC {:?}", rc);
                  // }
            } // else {
              //     eprintln!("DEBUG: No RC for target {:?}", target);
              // }
              // } else {
              //     eprintln!(
              //         "DEBUG: Sub-plane NOT found for {}. Available keys: {:?}",
              //         tag1,
              //         section.sub_planes.keys()
              //     );
              // }
            None
        })
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
        let mut keep_keys = HashSet::new();

        for k in deferred_keys {
            // If the deferred key is no longer pressed, we can't use it for chords regardless.
            // (It will be drained/removed at the end if not in keep_keys)
            if !self.chord_engine.state.pressed.contains(&k) {
                continue;
            }

            // 1. Check if [k, key] forms a valid 2-key chord
            if self.deferred_key_can_form_chord_with(k, key, shift, is_japanese) {
                keep_keys.insert(k);
                continue;
            }

            // 2. Check if [other, k, key] forms a valid 3-key chord using any other pressed key
            for other in self.chord_engine.state.pressed.iter() {
                if *other == k {
                    continue;
                }
                // Check if [*other, k, key] forms a valid chord.
                // resolve_with_modifier handles permutations for 3-key lookups.
                let (token, _) = self.resolve_with_modifier(&[*other, k, key], shift, is_japanese);
                if token.is_some() {
                    keep_keys.insert(k);
                    break;
                }
            }
        }

        if !keep_keys.is_empty() {
            // Keep only keys that can form a valid chord.
            // Remove others.
            let remove: HashSet<ScKey> = self
                .pending_nonshift_for_shift
                .difference(&keep_keys)
                .copied()
                .collect();
            self.remove_keys_from_pending(&remove, true);
            return;
        }

        // If no keys can form a valid chord, release all pending deferred keys.
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
            used: false,
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

            // If the key is currently pressed with an active pending stroke, preserve it.
            // This prevents deleting a newly re-pressed stroke when an older stroke of the
            // same physical key has just been resolved.
            let has_active_pending_stroke = self
                .chord_engine
                .state
                .pending
                .iter()
                .any(|p| p.key == *k && p.t_up.is_none());
            if self.chord_engine.state.pressed.contains(k) && has_active_pending_stroke {
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

    fn refresh_active_char_shift_keys(&mut self, shift: bool, is_japanese: bool) {
        let active_trigger_keys = {
            let Some(layout) = self.layout.as_ref() else {
                return;
            };

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

                if let Some(prefix_thumb) = self.chord_engine.state.prefix_pending {
                    mark_thumb_state(&prefix_thumb);
                }
            }

            let suffix = if shift {
                if has_left_thumb {
                    PINKY_LEFT_THUMB_SHIFT_SUFFIX
                } else if has_right_thumb {
                    PINKY_RIGHT_THUMB_SHIFT_SUFFIX
                } else {
                    PINKY_SHIFT_SUFFIX
                }
            } else if has_left_thumb {
                LEFT_THUMB_SHIFT_SUFFIX
            } else if has_right_thumb {
                RIGHT_THUMB_SHIFT_SUFFIX
            } else {
                NO_SHIFT_SUFFIX
            };

            let forced_section_name =
                if is_japanese && !has_left_thumb && !has_right_thumb && has_ext1_thumb {
                    Some(EXTENDED_THUMB_SHIFT_1_SECTION)
                } else if is_japanese && !has_left_thumb && !has_right_thumb && has_ext2_thumb {
                    Some(EXTENDED_THUMB_SHIFT_2_SECTION)
                } else {
                    None
                };

            let section =
                self.active_section_by_suffix(layout, suffix, is_japanese, forced_section_name);

            let mut active = HashMap::new();
            if let Some(section) = section {
                Self::register_trigger_keys_from_tag(
                    &self.keyboard_map,
                    &section.name,
                    &mut active,
                );
                for tag in section.sub_planes.keys() {
                    Self::register_trigger_keys_from_tag(&self.keyboard_map, tag, &mut active);
                }
            }

            active
        };

        self.chord_engine.profile.trigger_keys = active_trigger_keys;
    }

    fn register_trigger_keys_from_tag(
        keyboard_map: &KeyboardMap,
        tag: &str,
        trigger_keys: &mut HashMap<ScKey, String>,
    ) {
        let mut start = 0;
        while let Some(open) = tag[start..].find('<') {
            if let Some(close) = tag[start + open..].find('>') {
                let inner = &tag[start + open + 1..start + open + close];
                if let Some(sc) = keyboard_map.key_name_to_sc(inner) {
                    let key = ScKey::new(sc, false);
                    trigger_keys.entry(key).or_insert_with(|| tag.to_string());
                }
                start += open + close + 1;
            } else {
                break;
            }
        }
    }

    fn key_to_rc(&self, key: ScKey) -> Option<crate::types::Rc> {
        self.keyboard_map.key_to_rc(key)
    }

    fn is_romaji_pinky_shift_section_active(&self, shift_held: bool, is_japanese: bool) -> bool {
        if !shift_held || !is_japanese {
            return false;
        }

        let Some(layout) = self.layout.as_ref() else {
            return false;
        };

        let Some(tk) = self.chord_engine.profile.thumb_keys.as_ref() else {
            return Self::has_japanese_section_with_suffix(layout, PINKY_SHIFT_SUFFIX);
        };

        let mut has_left_thumb = false;
        let mut has_right_thumb = false;
        let mut has_ext1_thumb = false;
        let mut has_ext2_thumb = false;
        for k in &self.chord_engine.state.pressed {
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
        if !has_left_thumb && !has_right_thumb && !has_ext1_thumb && !has_ext2_thumb {
            if let Some(prefix_thumb) = self.chord_engine.state.prefix_pending {
                has_left_thumb = tk.left.contains(&prefix_thumb);
                has_right_thumb = tk.right.contains(&prefix_thumb);
                has_ext1_thumb = tk.ext1.contains(&prefix_thumb);
                has_ext2_thumb = tk.ext2.contains(&prefix_thumb);
            }
        }

        if !has_left_thumb && !has_right_thumb && (has_ext1_thumb || has_ext2_thumb) {
            return false;
        }

        let suffix = if has_left_thumb {
            PINKY_LEFT_THUMB_SHIFT_SUFFIX
        } else if has_right_thumb {
            PINKY_RIGHT_THUMB_SHIFT_SUFFIX
        } else {
            PINKY_SHIFT_SUFFIX
        };

        Self::has_japanese_section_with_suffix(layout, suffix)
    }

    fn token_to_events_with_ime(
        &self,
        token: &Token,
        shift_held: bool,
        is_japanese: bool,
        is_kana_input: bool,
    ) -> Option<Vec<InputEvent>> {
        match token {
            Token::None => None,
            Token::KeySequence(seq) => {
                let mut events = Vec::new();
                let mut committed_for_uppercase = false;
                let effective_shift_held =
                    if self.is_romaji_pinky_shift_section_active(shift_held, is_japanese) {
                        false
                    } else {
                        shift_held
                    };
                for stroke in seq {
                    if is_kana_input
                        && !committed_for_uppercase
                        && stroke.mods.is_empty()
                        && matches!(stroke.key, KeySpec::Char(c) if c.is_ascii_uppercase())
                    {
                        events.push(InputEvent::CommitImeComposition);
                        committed_for_uppercase = true;
                    }
                    // Strict scancode only for KeySequence (which now comes from single-quote/bare tokens)
                    append_keystroke_events(
                        &mut events,
                        stroke,
                        effective_shift_held,
                        false,
                        is_japanese,
                        is_kana_input,
                    );
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
        is_kana_input: bool,
    ) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for k in keys {
            if let Some(token) = self.resolve(&[*k], shift, is_japanese) {
                if let Some(ops) =
                    self.token_to_events_with_ime(&token, shift, is_japanese, is_kana_input)
                {
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

    fn handle_repeat_event(
        &mut self,
        key: ScKey,
        shift: bool,
        is_japanese: bool,
        is_kana_input: bool,
    ) -> KeyAction {
        let now = Instant::now();
        let (keys, consume_pending) = if let Some(keys) = self.repeat_plans.get(&key) {
            (keys.clone(), false)
        } else {
            self.compute_repeat_plan(key, now)
        };

        // Check if the repeating key is a thumb key and handle specifically
        let thumb_config = {
            let p = &self.chord_engine.profile;
            if p.thumb_left.key.to_sckey() == Some(key) {
                Some(&p.thumb_left)
            } else if p.thumb_right.key.to_sckey() == Some(key) {
                Some(&p.thumb_right)
            } else if p.extended_thumb1.key.to_sckey() == Some(key) {
                Some(&p.extended_thumb1)
            } else if p.extended_thumb2.key.to_sckey() == Some(key) {
                Some(&p.extended_thumb2)
            } else {
                None
            }
        };

        if let Some(config) = thumb_config {
            if !config.repeat {
                return KeyAction::Block;
            }
            match config.single_press {
                ThumbShiftSinglePress::SpaceKey => {
                    if consume_pending {
                        self.consume_pending_for_repeat(&keys);
                    }
                    self.repeat_plans.entry(key).or_insert(keys);
                    return KeyAction::Inject(vec![
                        InputEvent::Scancode(0x39, false, false),
                        InputEvent::Scancode(0x39, false, true),
                    ]);
                }
                ThumbShiftSinglePress::Enable => {
                    // Force repeat of the key itself
                    if consume_pending {
                        self.consume_pending_for_repeat(&keys);
                    }
                    self.repeat_plans.entry(key).or_insert(keys);
                    return KeyAction::Inject(vec![
                        InputEvent::Scancode(key.sc, key.ext, false),
                        InputEvent::Scancode(key.sc, key.ext, true),
                    ]);
                }
                _ => {
                    // None, PrefixShift -> Do not repeat
                    return KeyAction::Block;
                }
            }
        }

        let token = self.resolve(&keys, shift, is_japanese);
        let allow_repeat = self.repeat_allowed_for_token(token.as_ref());
        if !allow_repeat {
            return KeyAction::Block;
        }

        let mut resolved_from_layout = false;
        let events = if let Some(token) = token {
            if let Some(events) =
                self.token_to_events_with_ime(&token, shift, is_japanese, is_kana_input)
            {
                resolved_from_layout = true;
                events
            } else {
                self.repeat_fallback_events(&keys, shift, is_japanese, is_kana_input)
            }
        } else {
            self.repeat_fallback_events(&keys, shift, is_japanese, is_kana_input)
        };

        if events.is_empty() {
            return KeyAction::Block;
        }

        if consume_pending {
            self.consume_pending_for_repeat(&keys);
        }
        self.repeat_plans.entry(key).or_insert(keys);
        if resolved_from_layout {
            let keys = self.repeat_plans.get(&key).cloned().unwrap_or_default();
            self.record_analytics_replacement(&keys, &events);
        }
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
        if p2_dur == Duration::ZERO {
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
    _is_kana_input: bool,
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
        KeySpec::DirectString(ref s) => {
            // Hand off the complex IME handling logic to the hook (outside the lock).
            // This avoids deadlock when calling IME APIs while holding the Engine lock.
            events.push(InputEvent::DirectString(s.clone()));
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
    crate::keyboard_hook::vk_to_scancode(vk)
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

    use crate::parser::parse_layout_content;

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
    fn test_analytics_records_only_layout_replacements_in_alpha_mode() {
        let config = "
[英数シフト無し]
無,無,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無,無
b,無,無,無,無,無,無,無,無,無,無,無
無,無,無,無,無,無,無,無,無,無,無
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::ForceAlpha);
        engine.load_layout(layout);
        engine.analytics.set_enabled(true);
        engine
            .analytics
            .set_layout_name("analytics-test".to_string());

        assert_eq!(
            engine.process_key(0x30, false, false, false),
            KeyAction::Pass
        );
        assert_eq!(
            engine.process_key(0x30, false, true, false),
            KeyAction::Pass
        );
        assert!(engine.analytics.to_data().records.is_empty());

        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        match engine.process_key(0x1E, false, true, false) {
            KeyAction::Inject(events) => assert!(
                events
                    .iter()
                    .any(|event| matches!(event, InputEvent::Scancode(0x30, false, false))),
                "expected defined a -> b replacement, got {:?}",
                events
            ),
            other => panic!("expected Inject for defined replacement, got {:?}", other),
        }

        let data = engine.analytics.to_data();
        assert_eq!(data.records.len(), 1);
        let record = &data.records[0];
        assert_eq!(record.physical_keystrokes, 1);
        assert_eq!(record.output_virtual_keys, 1);
        assert_eq!(record.key_counts.get("a"), Some(&1));
        assert_eq!(record.key_counts.get("b"), None);
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
    fn test_romaji_pinky_shift_kana_sends_romaji_scancodes() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,の,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x20, false, false, true),
            KeyAction::Block
        );
        let res_up = engine.process_key(0x20, false, true, true);
        match res_up {
            KeyAction::Inject(evs) => {
                let has_n = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x31, _, _)));
                let has_o = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x18, _, _)));
                let has_shift = evs.iter().any(|e| {
                    matches!(
                        e,
                        InputEvent::Scancode(0x2A, _, _) | InputEvent::Scancode(0x36, _, _)
                    )
                });
                assert!(has_n && has_o, "Expected romaji 'n' and 'o' scancodes");
                assert!(
                    !has_shift,
                    "No Shift scancode should be injected for pinky-shift romaji output"
                );
            }
            _ => panic!("Expected Inject for shifted kana mapping, got {:?}", res_up),
        }
    }

    #[test]
    fn test_romaji_pinky_shift_fullwidth_uppercase_emits_uppercase_keystroke() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,Ａ,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x20, false, false, true),
            KeyAction::Block
        );
        let res_up = engine.process_key(0x20, false, true, true);
        match res_up {
            KeyAction::Inject(evs) => {
                let has_a = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _)));
                let has_shift = evs.iter().any(|e| {
                    matches!(
                        e,
                        InputEvent::Scancode(0x2A, _, _) | InputEvent::Scancode(0x36, _, _)
                    )
                });
                assert!(has_a, "Expected 'A' scancode output");
                assert!(
                    has_shift,
                    "Uppercase token should inject Shift modifier for uppercase key output"
                );
            }
            _ => panic!(
                "Expected Inject for shifted fullwidth uppercase mapping, got {:?}",
                res_up
            ),
        }
    }

    #[test]
    fn test_romaji_pinky_shift_undefined_key_passthrough_keeps_shift_modifier() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,b,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // 0x21 ('f') is undefined in both base and shifted sections.
        // It should pass through, so the OS can apply normal Shift behavior.
        assert_eq!(
            engine.process_key(0x21, false, false, true),
            KeyAction::Pass
        );
        assert_eq!(engine.process_key(0x21, false, true, true), KeyAction::Pass);
    }

    #[test]
    fn test_romaji_pinky_shift_keeps_shift_key_as_plain_modifier_key() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指シフト]
; R0
dummy
; R1
dummy
; R2
xx,xx,の,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert!(
            !engine.needs_left_shift_handling(),
            "Left Shift should remain passthrough so standalone Shift is recognized by other apps"
        );
        assert!(
            !engine.needs_right_shift_handling(),
            "Right Shift should remain passthrough so standalone Shift is recognized by other apps"
        );
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
            .token_to_events_with_ime(&token, false, false, false)
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
    fn test_kana_mode_uppercase_commits_then_emits_unconfirmed_scancode() {
        let token = Token::KeySequence(vec![KeyStroke {
            key: KeySpec::Char('A'),
            mods: Modifiers::none(),
        }]);
        let engine = Engine::default();
        let events = engine
            .token_to_events_with_ime(&token, false, true, true)
            .expect("Should return events");

        assert_eq!(
            events,
            vec![
                InputEvent::CommitImeComposition,
                InputEvent::Scancode(0x2A, false, false),
                InputEvent::Scancode(0x1E, false, false),
                InputEvent::Scancode(0x1E, false, true),
                InputEvent::Scancode(0x2A, false, true),
            ]
        );
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
xx,xx,d_base,xx,xx,xx,xx,x,xx,xx,xx,xx
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        // K is at Col 7? In R2: "xx,xx,d_base,xx,xx,xx,xx,x,..."
        // Index 7 is "x".
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
                // If fallback uses layout, we get "x" for K (0x2D), "d_base" for D.

                // Let's check for "x" scancode (0x2D) to prove resolution happened for K.
                let has_x = evs.iter().any(|e| match e {
                    InputEvent::Scancode(sc, _, _) => *sc == 0x2D,
                    _ => false,
                });
                assert!(
                    has_x,
                    "Expected 'x' (from 'x' definition for K) in fallback output"
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
    fn test_wait_key_up_skips_deferred_enter_recovery() {
        let config = "
[ローマ字シフト無し]
dummy
dummy
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let wait_key = ScKey::new(0x13, false);
        let now = Instant::now();
        engine.chord_engine.state.pressed.insert(wait_key);
        engine.chord_engine.state.down_ts.insert(wait_key, now);
        engine.chord_engine.state.pending.push(PendingKey {
            key: wait_key,
            t_down: now,
            t_up: None,
            used: false,
        });
        engine.deferred_enter_rollover = Some(DeferredEnterRollover {
            source_key: ScKey::new(0x1C, false),
            pass_through: PassThroughCurrent::Original,
            wait_for: wait_key,
            down_emitted: false,
            up_seen_while_waiting: false,
            started_at: Instant::now(),
        });

        engine.recover_stale_deferred_enter_rollover(wait_key, true);

        assert!(engine.deferred_enter_rollover.is_some());
        assert!(engine.chord_engine.state.pressed.contains(&wait_key));
        assert!(engine
            .chord_engine
            .state
            .pending
            .iter()
            .any(|pending| pending.key == wait_key));
    }

    #[test]
    fn test_enter_rollover_preserves_preceding_mapped_tap() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,x,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x13, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1C, false, false, false),
            KeyAction::Block
        );

        let res = engine.process_key(0x13, false, true, false);
        assert_eq!(
            res,
            KeyAction::Inject(vec![
                InputEvent::Scancode(0x2D, false, false),
                InputEvent::Scancode(0x2D, false, true),
                InputEvent::Scancode(0x1C, false, false),
            ])
        );

        let res = engine.process_key(0x1C, false, true, false);
        assert_eq!(
            res,
            KeyAction::Inject(vec![InputEvent::Scancode(0x1C, false, true)])
        );
    }

    #[test]
    fn test_stale_deferred_enter_rollover_is_recovered() {
        let config = "
[繝ｭ繝ｼ繝槫ｭ励す繝輔ヨ辟｡縺余
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let stale_wait_key = ScKey::new(0xFF, false);
        let now = Instant::now();
        engine.chord_engine.state.pressed.insert(stale_wait_key);
        engine
            .chord_engine
            .state
            .down_ts
            .insert(stale_wait_key, now);
        engine.chord_engine.state.pending.push(PendingKey {
            key: stale_wait_key,
            t_down: now,
            t_up: None,
            used: false,
        });
        engine.chord_engine.state.passed_keys.insert(stale_wait_key);
        engine
            .chord_engine
            .state
            .used_modifiers
            .insert(stale_wait_key);
        engine.pending_nonshift_for_shift.insert(stale_wait_key);
        engine
            .repeat_plans
            .insert(stale_wait_key, vec![stale_wait_key]);
        engine
            .passthrough_thumb_shift_modifiers
            .insert(stale_wait_key, ScKey::new(0x2A, false));
        engine.chord_engine.state.prefix_pending = Some(stale_wait_key);
        engine.deferred_enter_rollover = Some(DeferredEnterRollover {
            source_key: ScKey::new(0x1C, false),
            pass_through: PassThroughCurrent::Original,
            wait_for: stale_wait_key,
            down_emitted: false,
            up_seen_while_waiting: false,
            started_at: Instant::now()
                - Duration::from_millis(DEFERRED_ENTER_RECOVERY_TIMEOUT_MS + 1),
        });

        // Should recover stale deferred state and pass Enter immediately.
        let res = engine.process_key(0x1C, false, false, false);
        assert_eq!(res, KeyAction::Pass);
        assert!(engine.deferred_enter_rollover.is_none());
        assert!(!engine.chord_engine.state.pressed.contains(&stale_wait_key));
        assert!(!engine
            .chord_engine
            .state
            .down_ts
            .contains_key(&stale_wait_key));
        assert!(!engine
            .chord_engine
            .state
            .pending
            .iter()
            .any(|p| p.key == stale_wait_key));
        assert!(!engine
            .chord_engine
            .state
            .passed_keys
            .contains(&stale_wait_key));
        assert!(!engine
            .chord_engine
            .state
            .used_modifiers
            .contains(&stale_wait_key));
        assert!(!engine.pending_nonshift_for_shift.contains(&stale_wait_key));
        assert!(!engine.repeat_plans.contains_key(&stale_wait_key));
        assert!(!engine
            .passthrough_thumb_shift_modifiers
            .contains_key(&stale_wait_key));
        assert_ne!(
            engine.chord_engine.state.prefix_pending,
            Some(stale_wait_key)
        );
    }

    #[test]
    fn test_set_profile_preserves_targets() {
        let config = "
[ローマ字シフト無し]
; R2 (A only defined)
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
    fn test_load_layout_sets_max_chord_size_to_two_without_double_modifier_tag() {
        let config = "
[Main]
a

<q>
xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        assert_eq!(engine.get_profile().max_chord_size, 2);
    }

    #[test]
    fn test_load_layout_sets_max_chord_size_to_three_with_double_modifier_tag() {
        let config = "
[Main]
a

<q><w>
xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        assert_eq!(engine.get_profile().max_chord_size, 3);
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
    fn test_kana_section_is_used_when_romaji_section_is_missing() {
        let config = "
[かなシフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,の,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::Ignore);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
        match engine.process_key(0x20, false, true, false) {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x25, _, _))),
                    "Expected kana keycode 'k' from [かなシフト無し]"
                );
            }
            other => panic!("Expected Inject from [かなシフト無し], got {other:?}"),
        }
    }

    #[test]
    fn test_romaji_section_has_priority_over_kana_section() {
        let config = "
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx

[かなシフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,の,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::Ignore);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
        match engine.process_key(0x20, false, true, false) {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _))),
                    "Expected romaji section output to be selected"
                );
                assert!(
                    !evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x25, _, _))),
                    "Kana section output should be ignored when romaji section exists"
                );
            }
            other => panic!("Expected Inject for mixed romaji/kana sections, got {other:?}"),
        }
    }

    #[test]
    fn test_kana_section_dakuten_definition_emits_two_kana_keycodes() {
        let config = "
[かなシフト無し]
; R0
dummy
; R1
dummy
; R2
xx,xx,ど,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::Ignore);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x20, false, false, false),
            KeyAction::Block
        );
        match engine.process_key(0x20, false, true, false) {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x1F, _, _))),
                    "Expected base kana keycode 's' for 'ど'"
                );
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x1A, _, _))),
                    "Expected dakuten keycode '@' for 'ど'"
                );
            }
            other => panic!("Expected Inject for kana dakuten definition, got {other:?}"),
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse");
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
    fn test_continuous_shift_plain_alpha_rollover_emits_both_keys_when_newer_released_first() {
        let alpha_section = "\u{82f1}\u{6570}\u{30b7}\u{30d5}\u{30c8}\u{7121}\u{3057}";
        let config = format!(
            "
[{alpha_section}]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,s,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"
        );
        let layout = parse_layout_content(&config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::ForceAlpha);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);

        // A down -> S down -> S up -> A up
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1F, false, false, false),
            KeyAction::Block
        );

        let mut downs = Vec::new();
        for action in [
            engine.process_key(0x1F, false, true, false),
            engine.process_key(0x1E, false, true, false),
        ] {
            if let KeyAction::Inject(evs) = action {
                downs.extend(evs.into_iter().filter_map(|e| match e {
                    InputEvent::Scancode(sc, _, false) => Some(sc),
                    _ => None,
                }));
            }
        }

        assert_eq!(
            downs,
            vec![0x1E, 0x1F],
            "Expected rollover to emit A then S"
        );
    }

    #[test]
    fn test_continuous_shift_plain_alpha_rollover_emits_both_keys_when_older_released_first() {
        let alpha_section = "\u{82f1}\u{6570}\u{30b7}\u{30d5}\u{30c8}\u{7121}\u{3057}";
        let config = format!(
            "
[{alpha_section}]
; R0
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R1
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R2
a,s,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
; R3
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"
        );
        let layout = parse_layout_content(&config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::ForceAlpha);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.0;
        engine.set_profile(profile);

        // A down -> S down -> A up -> S up
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x1F, false, false, false),
            KeyAction::Block
        );

        let mut downs = Vec::new();
        for action in [
            engine.process_key(0x1E, false, true, false),
            engine.process_key(0x1F, false, true, false),
        ] {
            if let KeyAction::Inject(evs) = action {
                downs.extend(evs.into_iter().filter_map(|e| match e {
                    InputEvent::Scancode(sc, _, false) => Some(sc),
                    _ => None,
                }));
            }
        }

        assert_eq!(
            downs,
            vec![0x1E, 0x1F],
            "Expected rollover to emit A then S"
        );
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert!(
            engine.needs_alt_handling(),
            "Alt should be handled when it is used as [機能キー] swap source"
        );
    }

    #[test]
    fn test_needs_shift_handling_for_thumb_shift_assignment() {
        let mut engine = Engine::default();
        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::LeftShift;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);

        assert!(
            engine.needs_left_shift_handling(),
            "LeftShift should be handled when it is assigned as thumb shift"
        );
        assert!(
            !engine.needs_right_shift_handling(),
            "RightShift should not be handled when it is not assigned"
        );
    }

    #[test]
    fn test_needs_modifier_handling_is_disabled_when_engine_disabled() {
        let mut engine = Engine::default();
        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::LeftShift;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::RightCtrl;
        engine.set_profile(profile);
        engine.set_enabled(false);

        assert!(
            !engine.needs_left_shift_handling(),
            "Modifier handling should be off when engine is disabled"
        );
        assert!(
            !engine.needs_right_ctrl_handling(),
            "Modifier handling should be off when engine is disabled"
        );
    }

    #[test]
    fn test_needs_ctrl_handling_for_thumb_shift_assignment() {
        let mut engine = Engine::default();
        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::LeftCtrl;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);

        assert!(
            engine.needs_left_ctrl_handling(),
            "LeftCtrl should be handled when it is assigned as thumb shift"
        );
        assert!(
            !engine.needs_right_ctrl_handling(),
            "RightCtrl should not be handled when it is not assigned"
        );

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::RightCtrl;
        engine.set_profile(profile);

        assert!(
            !engine.needs_left_ctrl_handling(),
            "LeftCtrl should not be handled when it is not assigned"
        );
        assert!(
            engine.needs_right_ctrl_handling(),
            "RightCtrl should be handled when it is assigned as thumb shift"
        );
    }

    #[test]
    fn test_pinky_shift_rollover_bypass_keeps_shifted_state() {
        let config = "
[ローマ字小指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        let sc_char = ScKey::new(0x20, false); // 'D' key (Rc: 2, 2)
        let sc_shift = 0x2A; // Left Shift

        // Simulate Shift Down
        let action1 = engine.process_key(sc_shift, false, false, true);
        assert_eq!(action1, KeyAction::Pass);

        // Simulate 'D' Down with Shift
        let action2 = engine.process_key(sc_char.sc, sc_char.ext, false, true);
        if let KeyAction::Inject(ops) = action2 {
            // Should be 'a' token mapped to OS output immediately, bypassing ChordEngine
            assert!(
                ops.iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _))), // 0x1E is 'A' key
                "Expected Inject output for 'A' directly from bypass"
            );
        } else {
            panic!(
                "Expected Inject output directly from bypass, got {:?}",
                action2
            );
        }

        assert!(
            engine.chord_engine.state.pressed.contains(&sc_char),
            "Character key should be tracked as pressed to absorb UP event"
        );
        assert!(
            !engine
                .chord_engine
                .state
                .pending
                .iter()
                .any(|p| p.key == sc_char),
            "Character key should NOT be pending in ChordEngine"
        );

        // Simulate Shift Up
        let action3 = engine.process_key(sc_shift, false, true, false);
        assert_eq!(action3, KeyAction::Pass);

        // Simulate 'D' Up (after Shift is released)
        let action4 = engine.process_key(sc_char.sc, sc_char.ext, true, false);
        // Because it was registered as pressed but not pending, UP event should just be absorbed.
        assert_eq!(
            action4,
            KeyAction::Block,
            "The unmatched UP event should be safely absorbed"
        );
    }

    #[test]
    fn test_capture_shift_for_romaji_pinky_shift_section() {
        let config = "
[ローマ字小指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        assert!(
            engine.capture_left_shift_for_romaji_pinky_shift(),
            "Left Shift should be capturable for romaji pinky shift when not otherwise assigned"
        );
        assert!(
            engine.capture_right_shift_for_romaji_pinky_shift(),
            "Right Shift should be capturable for romaji pinky shift when not otherwise assigned"
        );
    }

    #[test]
    fn test_capture_shift_for_kana_pinky_shift_section() {
        let config = "
[かな小指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        assert!(
            engine.capture_left_shift_for_romaji_pinky_shift(),
            "Left Shift should be capturable for kana pinky shift as well"
        );
        assert!(
            engine.capture_right_shift_for_romaji_pinky_shift(),
            "Right Shift should be capturable for kana pinky shift as well"
        );
    }

    #[test]
    fn test_capture_shift_for_pinky_shift_is_disabled_when_engine_disabled() {
        let config = "
[ローマ字小指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);
        engine.set_enabled(false);

        assert!(
            !engine.capture_left_shift_for_romaji_pinky_shift(),
            "Shift capture should be disabled when engine is disabled"
        );
        assert!(
            !engine.capture_right_shift_for_romaji_pinky_shift(),
            "Shift capture should be disabled when engine is disabled"
        );
    }

    #[test]
    fn test_capture_shift_for_romaji_pinky_thumb_shift_sections() {
        let config = "
[ローマ字小指左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,b,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指右親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,c,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        assert!(
            engine.capture_left_shift_for_romaji_pinky_shift(),
            "Left Shift should be capturable when romaji pinky+left-thumb section exists"
        );
        assert!(
            engine.capture_right_shift_for_romaji_pinky_shift(),
            "Right Shift should be capturable when romaji pinky+right-thumb section exists"
        );
    }

    #[test]
    fn test_romaji_pinky_thumb_shift_sections_are_treated_as_thumb_shift_sections() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,b,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指右親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,c,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let profile = engine.get_profile();
        assert!(
            profile.thumb_keys.is_some(),
            "Thumb keys should remain enabled when romaji pinky+thumb sections exist"
        );

        assert_eq!(
            engine.process_key(0x7B, false, false, true),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x20, false, false, true),
            KeyAction::Block
        );
        match engine.process_key(0x20, false, true, true) {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x30, _, _))),
                    "Expected 'b' output from [ローマ字小指左親指シフト]"
                );
            }
            other => panic!("Expected Inject for left pinky+thumb shifted output, got {other:?}"),
        }
        assert_eq!(
            engine.process_key(0x7B, false, true, true),
            KeyAction::Block
        );

        assert_eq!(
            engine.process_key(0x79, false, false, true),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x20, false, false, true),
            KeyAction::Block
        );
        match engine.process_key(0x20, false, true, true) {
            KeyAction::Inject(evs) => {
                assert!(
                    evs.iter()
                        .any(|e| matches!(e, InputEvent::Scancode(0x2E, _, _))),
                    "Expected 'c' output from [ローマ字小指右親指シフト]"
                );
            }
            other => panic!("Expected Inject for right pinky+thumb shifted output, got {other:?}"),
        }
        assert_eq!(
            engine.process_key(0x79, false, true, true),
            KeyAction::Block
        );
    }

    #[test]
    fn test_kana_pinky_thumb_shift_sections_are_treated_as_thumb_shift_sections() {
        let config = "
[かなシフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[かな小指左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,b,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[かな小指右親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,c,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let profile = engine.get_profile();
        assert!(
            profile.thumb_keys.is_some(),
            "Thumb keys should remain enabled when kana pinky+thumb sections exist"
        );
    }

    #[test]
    fn test_romaji_pinky_left_thumb_shift_fullwidth_uppercase_emits_uppercase_keystroke() {
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字小指左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,Ａ,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        assert_eq!(
            engine.process_key(0x7B, false, false, true),
            KeyAction::Block
        );
        assert_eq!(
            engine.process_key(0x20, false, false, true),
            KeyAction::Block
        );
        let res_up = engine.process_key(0x20, false, true, true);
        match res_up {
            KeyAction::Inject(evs) => {
                let has_a = evs
                    .iter()
                    .any(|e| matches!(e, InputEvent::Scancode(0x1E, _, _)));
                let has_shift = evs.iter().any(|e| {
                    matches!(
                        e,
                        InputEvent::Scancode(0x2A, _, _) | InputEvent::Scancode(0x36, _, _)
                    )
                });
                assert!(has_a, "Expected 'A' scancode output");
                assert!(
                    has_shift,
                    "Uppercase token should inject Shift modifier for pinky+thumb shifted output"
                );
            }
            _ => panic!(
                "Expected Inject for pinky+thumb shifted fullwidth uppercase mapping, got {:?}",
                res_up
            ),
        }
        assert_eq!(
            engine.process_key(0x7B, false, true, true),
            KeyAction::Block
        );
    }

    #[test]
    fn test_capture_shift_for_romaji_pinky_shift_is_disabled_when_shift_is_assigned() {
        let config = "
[ローマ字小指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字右親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");
        let mut engine = Engine::default();
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::LeftShift;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::RightShift;
        engine.set_profile(profile);

        assert!(
            !engine.capture_left_shift_for_romaji_pinky_shift(),
            "Left Shift capture should be off when LeftShift is explicitly assigned"
        );
        assert!(
            !engine.capture_right_shift_for_romaji_pinky_shift(),
            "Right Shift capture should be off when RightShift is explicitly assigned"
        );
    }

    #[test]
    fn test_thumb_shift_key_falls_back_to_normal_shift_for_non_character_keys() {
        let config = r#"
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
a,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字左親指シフト]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
b,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"#;
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::LeftShift;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::None;
        engine.set_profile(profile);

        assert_eq!(
            engine.process_key(0x2A, false, false, false),
            KeyAction::Block
        );

        match engine.process_key(0x47, true, false, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(
                    evs,
                    vec![
                        InputEvent::Scancode(0x2A, false, false),
                        InputEvent::Scancode(0x47, true, false),
                    ],
                    "Home down should be emitted with synthetic Shift down"
                );
            }
            other => panic!(
                "Expected Inject for Shift+Home down fallback, got {:?}",
                other
            ),
        }

        match engine.process_key(0x47, true, true, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(
                    evs,
                    vec![
                        InputEvent::Scancode(0x47, true, true),
                        InputEvent::Scancode(0x2A, false, true),
                    ],
                    "Home up should be emitted with synthetic Shift up"
                );
            }
            other => panic!(
                "Expected Inject for Shift+Home up fallback, got {:?}",
                other
            ),
        }

        assert_eq!(
            engine.process_key(0x2A, false, true, false),
            KeyAction::Block
        );

        let mut profile = engine.get_profile();
        profile.thumb_left.key = crate::chord_engine::ThumbKeySelect::None;
        profile.thumb_right.key = crate::chord_engine::ThumbKeySelect::RightShift;
        engine.set_profile(profile);

        assert_eq!(
            engine.process_key(0x36, false, false, false),
            KeyAction::Block
        );

        match engine.process_key(0x4F, true, false, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(
                    evs,
                    vec![
                        InputEvent::Scancode(0x36, false, false),
                        InputEvent::Scancode(0x4F, true, false),
                    ],
                    "End down should be emitted with synthetic RightShift down"
                );
            }
            other => panic!(
                "Expected Inject for RightShift+End down fallback, got {:?}",
                other
            ),
        }

        match engine.process_key(0x4F, true, true, false) {
            KeyAction::Inject(evs) => {
                assert_eq!(
                    evs,
                    vec![
                        InputEvent::Scancode(0x4F, true, true),
                        InputEvent::Scancode(0x36, false, true),
                    ],
                    "End up should be emitted with synthetic RightShift up"
                );
            }
            other => panic!(
                "Expected Inject for RightShift+End up fallback, got {:?}",
                other
            ),
        }

        assert_eq!(
            engine.process_key(0x36, false, true, false),
            KeyAction::Block
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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(&config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
    fn test_suspend_shortcut_persists_when_disabled() {
        let mut engine = Engine::default();
        let mut profile = engine.get_profile();
        let test_shortcut = crate::types::ShortcutKey {
            vkey: 0x13, // VK_PAUSE
            code: "Pause".to_string(),
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
        };
        profile.suspend_shortcut = Some(test_shortcut.clone());
        engine.set_profile(profile);

        engine.set_enabled(false);
        assert_eq!(engine.get_profile().suspend_shortcut, Some(test_shortcut));
    }
    #[test]
    fn test_3key_chord_resolution() {
        // Define a layout where <q><w> defines 'a' (0x1E)
        // q=0x10, w=0x11, e=0x12 (target)
        let config = "
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        // xx,2,xx... (row 1, col 1 is 'w' position -> outputs '2')
        // <q><w>
        // xx,xx,3... (row 1, col 2 is 'e' position -> outputs '3')

        let config = "
[英数シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,2,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,3,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

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

    #[test]
    fn test_3key_continuous_shift_with_non_trigger_target() {
        // Simulates D+F held, N tapped repeatedly.
        // D and F are both trigger keys (<d>, <f>, <d><f> sections exist).
        // N has a mapping in <f> section (similar to 薙刀式 where <f><n> = だ).
        //
        // Key codes: d=0x20, f=0x21, n=0x31
        // n is at bottom row position 5 (0-indexed)
        //
        // Note: <f> section is intentionally omitted (or empty) to verify that
        // D+F+N chord works even when F+N is not defined in <f> section.
        // Previously, F was released immediately because F+N formed no chord,
        // breaking the D+F+N chord.
        let config = "
[英数シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,f,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,n,xx,xx,xx,xx,xx

[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,f,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,n,xx,xx,xx,xx,xx

<d>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,2,xx,xx,xx,xx,xx

<f>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx


<d><f>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,4,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.35;
        engine.set_profile(profile);

        // ==== First chord: D+F+N → should output '4' ====
        engine.process_key(0x20, false, false, false); // D down
        std::thread::sleep(Duration::from_millis(5));
        engine.process_key(0x21, false, false, false); // F down
        std::thread::sleep(Duration::from_millis(15));
        engine.process_key(0x31, false, false, false); // N down
        std::thread::sleep(Duration::from_millis(30));
        let r1 = engine.process_key(0x31, false, true, false); // N up

        let events1 = match r1 {
            KeyAction::Inject(evs) => evs,
            other => panic!("First N(Up): expected Inject, got {:?}", other),
        };
        assert!(
            events1
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x05, _, false))),
            "First chord: expected '4' (sc=0x05) but got {:?}",
            events1
        );

        // ==== Second chord: D+F still held, N tapped again → should also output '4' ====
        std::thread::sleep(Duration::from_millis(10));
        engine.process_key(0x31, false, false, false); // N down
        std::thread::sleep(Duration::from_millis(30));
        let r2 = engine.process_key(0x31, false, true, false); // N up

        let events2 = match r2 {
            KeyAction::Inject(evs) => evs,
            other => panic!("Second N(Up): expected Inject, got {:?}", other),
        };
        assert!(
            events2
                .iter()
                .any(|e| matches!(e, InputEvent::Scancode(0x05, _, false))),
            "Second chord: expected '4' (sc=0x05) but got {:?}. \
             If '3' (0x03) was output, F-only shift was used instead of D+F.",
            events2
        );
    }

    #[test]
    fn test_3key_undefined_chord_with_continuous_modifier_splits() {
        // Reproduces the rolling bug: D held + ; rolled + L rolled, with
        // <d>+;+L undefined but <d>+;='1' and <d>+L='2' defined -> expect
        // '1','2' (split into two 2-key chords) instead of falling back to
        // base D/;/L scancodes.
        //
        // Scancodes: d=0x20, l=0x26, ;=0x27. '1'=sc 0x02, '2'=sc 0x03.
        let config = "
[英数シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,xx,xx,xx,xx,xx,l,;,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,xx,xx,xx,xx,xx,l,;,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<d>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,2,1,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.35;
        engine.set_profile(profile);

        // Roll: D down -> ; down -> L down -> ; up -> L up -> D up
        engine.process_key(0x20, false, false, false); // D down
        std::thread::sleep(Duration::from_millis(5));
        engine.process_key(0x27, false, false, false); // ; down
        std::thread::sleep(Duration::from_millis(5));
        engine.process_key(0x26, false, false, false); // L down
        std::thread::sleep(Duration::from_millis(5));
        let r1 = engine.process_key(0x27, false, true, false); // ; up
        std::thread::sleep(Duration::from_millis(5));
        let r2 = engine.process_key(0x26, false, true, false); // L up
        std::thread::sleep(Duration::from_millis(5));
        let r3 = engine.process_key(0x20, false, true, false); // D up

        let mut all_events = Vec::new();
        if let KeyAction::Inject(evs) = r1 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r2 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r3 {
            all_events.extend(evs);
        }

        let down_scancodes: Vec<u16> = all_events
            .iter()
            .filter_map(|e| match e {
                InputEvent::Scancode(sc, _, false) => Some(*sc),
                _ => None,
            })
            .collect();

        assert!(
            down_scancodes.contains(&0x02),
            "Expected '1' (D+; chord, sc=0x02) in {:?}",
            down_scancodes
        );
        assert!(
            down_scancodes.contains(&0x03),
            "Expected '2' (D+L chord, sc=0x03) in {:?}",
            down_scancodes
        );
        assert!(
            !down_scancodes.contains(&0x20),
            "Should NOT emit D base (sc=0x20) -- would mean fell back to base"
        );
        assert!(
            !down_scancodes.contains(&0x27),
            "Should NOT emit ; base (sc=0x27)"
        );
        assert!(
            !down_scancodes.contains(&0x26),
            "Should NOT emit L base (sc=0x26)"
        );
    }

    #[test]
    fn test_3key_split_when_other_keys_are_also_trigger_keys() {
        // Realistic scenario where ; and L are ALSO trigger_keys because
        // `<v><;>` and `<v><l>` planes exist. The modifier identification can't
        // rely on is_char_shift_key alone -- must try each key as candidate
        // and pick the best resolution.
        //
        // Layout: <d>+;='1', <d>+L='2', plus dummy <v><;> and <v><l> sections
        // to register ; and L as trigger_keys (with no useful values for D pairs).
        let config = "
[英数シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,xx,xx,xx,xx,v,l,;,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,d,xx,xx,xx,xx,v,l,;,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<d>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,2,1,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<v><;>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<v><l>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
";
        let layout = parse_layout_content(config, &crate::keyboard_map::new_jis_106())
            .expect("Failed to parse config");

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.35;
        engine.set_profile(profile);

        engine.process_key(0x20, false, false, false); // D down
        std::thread::sleep(Duration::from_millis(5));
        engine.process_key(0x27, false, false, false); // ; down
        std::thread::sleep(Duration::from_millis(5));
        engine.process_key(0x26, false, false, false); // L down
        std::thread::sleep(Duration::from_millis(5));
        let r1 = engine.process_key(0x27, false, true, false); // ; up
        std::thread::sleep(Duration::from_millis(5));
        let r2 = engine.process_key(0x26, false, true, false); // L up
        std::thread::sleep(Duration::from_millis(5));
        let r3 = engine.process_key(0x20, false, true, false); // D up

        let mut all_events = Vec::new();
        if let KeyAction::Inject(evs) = r1 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r2 {
            all_events.extend(evs);
        }
        if let KeyAction::Inject(evs) = r3 {
            all_events.extend(evs);
        }

        let down_scancodes: Vec<u16> = all_events
            .iter()
            .filter_map(|e| match e {
                InputEvent::Scancode(sc, _, false) => Some(*sc),
                _ => None,
            })
            .collect();

        assert!(
            down_scancodes.contains(&0x02),
            "Expected '1' (D+; chord, sc=0x02) in {:?} (modifier identification \
             must pick D, not ; or L)",
            down_scancodes
        );
        assert!(
            down_scancodes.contains(&0x03),
            "Expected '2' (D+L chord, sc=0x03) in {:?}",
            down_scancodes
        );
        assert!(
            !down_scancodes.contains(&0x20),
            "Should NOT emit D base (sc=0x20)"
        );
        assert!(
            !down_scancodes.contains(&0x27),
            "Should NOT emit ; base (sc=0x27)"
        );
        assert!(
            !down_scancodes.contains(&0x26),
            "Should NOT emit L base (sc=0x26)"
        );
    }

    #[test]
    fn test_mixed_string_and_keys() {
        let config = "
[ローマ字シフト無し]
xx
xx
\"test\"b
";
        let layout =
            crate::parser::parse_layout_content(config, &crate::keyboard_map::new_jis_106())
                .expect("Failed to parse config");

        if let Some(sec) = layout.sections.get("ローマ字シフト無し") {
            println!("Section found. Map keys: {:?}", sec.base_plane.map.keys());
            let rc = crate::types::Rc::new(2, 0);
            println!("Looking for (2,0): {:?}", sec.base_plane.map.get(&rc));
        } else {
            println!(
                "Section 'ローマ字シフト無し' NOT found. Available: {:?}",
                layout.sections.keys()
            );
        }

        let mut engine = Engine::default();
        engine.set_ignore_ime(true);
        engine.load_layout(layout);

        // 'a' (0x1E) -> "test" then 'b' (0x30)
        // Down
        assert_eq!(
            engine.process_key(0x1E, false, false, false),
            KeyAction::Block
        );
        // Up
        let res = engine.process_key(0x1E, false, true, false);
        match res {
            KeyAction::Inject(evs) => {
                // Now we expect a DirectString event, followed by 'b' key events.
                // The complex IME handling is offloaded to the hook.

                // Expected events:
                // 1. DirectString("test")
                // 2. Scancode(0x30, false, false) (b down)
                // 3. Scancode(0x30, false, true) (b up)

                assert_eq!(evs.len(), 3);

                match &evs[0] {
                    InputEvent::DirectString(s) => assert_eq!(s, "test"),
                    _ => panic!("Expected DirectString, got {:?}", evs[0]),
                }

                match &evs[1] {
                    InputEvent::Scancode(sc, _, false) => assert_eq!(*sc, 0x30),
                    _ => panic!("Expected Scancode down, got {:?}", evs[1]),
                }

                match &evs[2] {
                    InputEvent::Scancode(sc, _, true) => assert_eq!(*sc, 0x30),
                    _ => panic!("Expected Scancode up, got {:?}", evs[2]),
                }
            }
            other => panic!("Expected Inject for direct string + key, got {:?}", other),
        }
    }
}
