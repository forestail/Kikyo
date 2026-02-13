use kikyo_core::engine::Engine;
use kikyo_core::parser;
use kikyo_core::types::{InputEvent, KeyAction};
use std::path::PathBuf;

fn collect_down_sc(events: &[InputEvent]) -> Vec<u16> {
    events
        .iter()
        .filter_map(|e| match e {
            InputEvent::Scancode(sc, _, false) => Some(*sc),
            _ => None,
        })
        .collect()
}

fn run_and_collect(engine: &mut Engine, sc: u16, up: bool, out: &mut Vec<InputEvent>) {
    if let KeyAction::Inject(evs) = engine.process_key(sc, false, up, false) {
        out.extend(evs);
    }
}

#[test]
fn delayed_release_of_first_k_does_not_drop_second_f() {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.push("..");
    p.push("layout");
    p.push("sin-geta.yab");

    let layout = parser::load_yab(&p).expect("load sin-geta.yab");

    let mut engine = Engine::default();
    engine.set_ignore_ime(true);
    engine.load_layout(layout);

    let mut profile = engine.get_profile();
    profile.char_key_continuous = false;
    profile.char_key_overlap_ratio = 0.35;
    engine.set_profile(profile);

    let mut all = Vec::new();

    // F+K - F - S+M - K
    // Keep the first K pressed across the second F down.
    run_and_collect(&mut engine, 0x21, false, &mut all); // F1 down
    run_and_collect(&mut engine, 0x25, false, &mut all); // K1 down
    run_and_collect(&mut engine, 0x21, true, &mut all);  // F1 up
    run_and_collect(&mut engine, 0x21, false, &mut all); // F2 down
    run_and_collect(&mut engine, 0x25, true, &mut all);  // K1 up (delayed)
    run_and_collect(&mut engine, 0x21, true, &mut all);  // F2 up
    run_and_collect(&mut engine, 0x1F, false, &mut all); // S down
    run_and_collect(&mut engine, 0x32, false, &mut all); // M down
    run_and_collect(&mut engine, 0x32, true, &mut all);  // M up
    run_and_collect(&mut engine, 0x1F, true, &mut all);  // S up
    run_and_collect(&mut engine, 0x25, false, &mut all); // K2 down
    run_and_collect(&mut engine, 0x25, true, &mut all);  // K2 up

    // mo nn da i (scancode down order)
    let downs = collect_down_sc(&all);
    assert_eq!(downs, vec![0x32, 0x18, 0x31, 0x31, 0x20, 0x1E, 0x17]);
}
