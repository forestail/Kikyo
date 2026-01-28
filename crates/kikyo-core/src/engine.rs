use crate::chord_engine::{ChordEngine, Decision, KeyEdge, KeyEvent, Profile};
use crate::types::{InputEvent, KeyAction, Layout, ScKey, Token};
use crate::JIS_SC_TO_RC;
use parking_lot::Mutex;
use std::time::Instant;
use tracing::debug;

lazy_static::lazy_static! {
    pub static ref ENGINE: Mutex<Engine> = Mutex::new(Engine::default());
}

pub struct Engine {
    chord_engine: ChordEngine,
    enabled: bool,
    layout: Option<Layout>,
    ignore_ime: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            chord_engine: ChordEngine::new(Profile::default()),
            enabled: true,
            layout: None,
            ignore_ime: false,
        }
    }
}

impl Engine {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.chord_engine = ChordEngine::new(Profile::default());
            if let Some(_l) = &self.layout {
                // restore profile logic if needed
            }
        }
    }

    pub fn set_ignore_ime(&mut self, ignore: bool) {
        self.ignore_ime = ignore;
    }

    pub fn get_profile(&self) -> Profile {
        self.chord_engine.profile.clone()
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.chord_engine.set_profile(profile);
    }

    pub fn load_layout(&mut self, layout: Layout) {
        tracing::info!(
            "Engine: Layout loaded with {} sections.",
            layout.sections.len()
        );

        let mut profile = self.chord_engine.profile.clone();
        // MVP: Detect trigger keys from "<...>" sections.
        for name in layout.sections.keys() {
            tracing::info!(" - Section: {}", name);
            if name.starts_with('<') && name.ends_with('>') {
                let inner = &name[1..name.len() - 1];
                if let Some(sc) = crate::jis_map::key_name_to_sc(inner) {
                    let key = ScKey::new(sc, false);
                    profile.trigger_keys.insert(key, name.clone());
                    tracing::info!("   -> Registered TriggerKey: {} (sc={:02X})", name, sc);
                }
            }
        }

        self.chord_engine.set_profile(profile);
        self.layout = Some(layout);
    }

    pub fn process_key(&mut self, sc: u16, ext: bool, up: bool, shift: bool) -> KeyAction {
        if !self.enabled {
            return KeyAction::Pass;
        }
        if !self.ignore_ime && !crate::ime::is_ime_on() {
            return KeyAction::Pass;
        }

        if self.layout.is_none() {
            return KeyAction::Pass;
        }

        let key = ScKey::new(sc, ext);
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
                    if let Some(token) = self.resolve(&[k], shift) {
                        if let Some(ops) = self.token_to_events(&token) {
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
                    if let Some(token) = self.resolve(&keys, shift) {
                        if let Some(ops) = self.token_to_events(&token) {
                            inject_ops.extend(ops);
                        }
                    } else {
                        // Fallback: undefined chord -> treat as sequential inputs
                        for k in keys {
                            // Try to resolve as single key (unshifted)
                            let mut resolved = false;
                            if let Some(token) = self.resolve(&[k], false) {
                                if let Some(ops) = self.token_to_events(&token) {
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

        if !inject_ops.is_empty() {
            return KeyAction::Inject(inject_ops);
        }

        if pass_current {
            return KeyAction::Pass;
        }

        KeyAction::Block
    }

    fn resolve(&self, keys: &[ScKey], shift: bool) -> Option<Token> {
        let layout = self.layout.as_ref()?;
        let section_name = if shift {
            "ローマ字小指シフト"
        } else {
            "ローマ字シフト無し"
        };
        // tracing::info!("Resolve: key={:?} shift={} section={}", keys, shift, section_name);
        let section = layout.sections.get(section_name)?;

        if keys.len() == 1 {
            let key = keys[0];
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
        } else if keys.len() == 2 {
            let k1 = keys[0];
            let k2 = keys[1];

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

    fn token_to_events(&self, token: &Token) -> Option<Vec<InputEvent>> {
        match token {
            Token::None => None,
            Token::KeySequence(seq) => {
                let mut events = Vec::new();
                for c in seq.chars() {
                    if let Some((sc, ext)) = char_to_scancode(c) {
                        events.push(InputEvent::Scancode(sc, ext, false));
                        events.push(InputEvent::Scancode(sc, ext, true));
                    } else {
                        // Fallback to Unicode injection
                        // warn!("Unknown char: {}", c);
                        events.push(InputEvent::Unicode(c, false)); // Down
                        events.push(InputEvent::Unicode(c, true)); // Up
                    }
                }
                if events.is_empty() {
                    None
                } else {
                    Some(events)
                }
            }
            _ => None,
        }
    }
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
        '\u{0008}' => Some((0x0E, false)), // BS
        '\u{000D}' => Some((0x1C, false)), // Enter
        '\u{F702}' => Some((0x4B, true)),  // Left Arrow (Extended)
        '\u{F703}' => Some((0x4D, true)),  // Right Arrow (Extended)
        ',' => Some((0x33, false)),
        '.' => Some((0x34, false)),
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
        engine.chord_engine.profile.min_overlap_ms = 0;
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
        engine.chord_engine.profile.min_overlap_ms = 0;
        engine.load_layout(layout);

        // 1. Press K (0x25) -> Expect BLOCK (Delayed)
        let res = engine.process_key(0x25, false, false, false);
        assert_eq!(res, KeyAction::Block);

        // 2. Press D (0x20) WHILE K is pressed -> Expect "dc" (Chord)
        let res = engine.process_key(0x20, false, false, false);
        match res {
            KeyAction::Inject(evs) => {
                assert_eq!(evs.len(), 4);
                // "c" -> 0x2E
                match evs[2] {
                    InputEvent::Scancode(sc, _, _) => assert_eq!(sc, 0x2E),
                    _ => panic!("Expected Scancode"),
                }
            }
            _ => panic!("Expected Inject for Chord D, got {:?}", res),
        }

        // 3. Release D
        engine.process_key(0x20, false, true, false);

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
        engine.chord_engine.profile.min_overlap_ms = 0;
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
    fn test_unicode_fallback() {
        let engine = Engine::default();
        let token = Token::KeySequence("→".to_string());
        // We can access private methods in tests module of the same file
        let events = engine
            .token_to_events(&token)
            .expect("Should return events");

        assert_eq!(events.len(), 2);
        match events[0] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, '→');
                assert_eq!(up, false);
            }
            _ => panic!("Expected Unicode down"),
        }
        match events[1] {
            InputEvent::Unicode(c, up) => {
                assert_eq!(c, '→');
                assert_eq!(up, true);
            }
            _ => panic!("Expected Unicode up"),
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
        engine.chord_engine.profile.min_overlap_ms = 0;
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

        let res = engine.process_key(0x20, false, false, false);
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
            _ => panic!("Expected Inject (Fallback), got {:?}", res),
        }
    }
}
