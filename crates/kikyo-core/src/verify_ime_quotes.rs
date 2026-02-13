use crate::engine::Engine;
use crate::parser::parse_yab_content;
use crate::types::{InputEvent, KeyAction};

#[test]
fn test_ime_quote_behavior_double_quotes_confirmed() {
    let config = r#"
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
"漢字",xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"#;
    let layout = parse_yab_content(config).expect("Failed to parse config");

    let mut engine = Engine::default();
    engine.set_ignore_ime(true);
    engine.load_layout(layout);

    // key 0x1E is 'a'. Mapped to "漢字" (double quotes)
    // Expect InputEvent::Unicode

    // Down
    assert_eq!(
        engine.process_key(0x1E, false, false, false),
        KeyAction::Block
    );

    // Up
    let res = engine.process_key(0x1E, false, true, false);
    match res {
        KeyAction::Inject(evs) => {
            // "漢字" -> 2 chars. Each char has down/up unicode event. Total 4 events.
            assert_eq!(evs.len(), 4);
            match evs[0] {
                InputEvent::Unicode(c, up) => {
                    assert_eq!(c, '漢');
                    assert_eq!(up, false);
                }
                _ => panic!("Expected Unicode('漢', down)"),
            }
            match evs[1] {
                InputEvent::Unicode(c, up) => {
                    assert_eq!(c, '漢');
                    assert_eq!(up, true);
                }
                _ => panic!("Expected Unicode('漢', up)"),
            }
            match evs[2] {
                InputEvent::Unicode(c, up) => {
                    assert_eq!(c, '字');
                    assert_eq!(up, false);
                }
                _ => panic!("Expected Unicode('字', down)"),
            }
            match evs[3] {
                InputEvent::Unicode(c, up) => {
                    assert_eq!(c, '字');
                    assert_eq!(up, true);
                }
                _ => panic!("Expected Unicode('字', up)"),
            }
        }
        _ => panic!("Expected Inject for double-quoted string, got {:?}", res),
    }
}

#[test]
fn test_ime_quote_behavior_single_quotes_unconfirmed() {
    let config = r#"
[ローマ字シフト無し]
; R0
dummy
; R1
dummy
; R2
'ka',xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
"#;
    let layout = parse_yab_content(config).expect("Failed to parse config");

    let mut engine = Engine::default();
    engine.set_ignore_ime(true);
    engine.load_layout(layout);

    // key 0x1E is 'a'. Mapped to 'ka' (single quotes)
    // Expect InputEvent::Scancode sequence for 'k', 'a'

    // Down
    assert_eq!(
        engine.process_key(0x1E, false, false, false),
        KeyAction::Block
    );

    // Up
    let res = engine.process_key(0x1E, false, true, false);
    match res {
        KeyAction::Inject(evs) => {
            // 'k', 'a' -> 2 keystrokes. Each has down/up scancode events. Total 4 events.
            assert_eq!(evs.len(), 4);

            // k (0x25)
            match evs[0] {
                InputEvent::Scancode(sc, _, up) => {
                    assert_eq!(sc, 0x25);
                    assert_eq!(up, false);
                }
                _ => panic!("Expected Scancode('k', down)"),
            }
            match evs[1] {
                InputEvent::Scancode(sc, _, up) => {
                    assert_eq!(sc, 0x25);
                    assert_eq!(up, true);
                }
                _ => panic!("Expected Scancode('k', up)"),
            }

            // a (0x1E)
            match evs[2] {
                InputEvent::Scancode(sc, _, up) => {
                    assert_eq!(sc, 0x1E);
                    assert_eq!(up, false);
                }
                _ => panic!("Expected Scancode('a', down)"),
            }
            match evs[3] {
                InputEvent::Scancode(sc, _, up) => {
                    assert_eq!(sc, 0x1E);
                    assert_eq!(up, true);
                }
                _ => panic!("Expected Scancode('a', up)"),
            }
        }
        _ => panic!("Expected Inject for single-quoted string, got {:?}", res),
    }
}
