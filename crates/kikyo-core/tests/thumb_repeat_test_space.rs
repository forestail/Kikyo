use kikyo_core::chord_engine::{ThumbKeySelect, ThumbShiftSinglePress};
use kikyo_core::engine::Engine;
use kikyo_core::types::{InputEvent, KeyAction, Layout, Plane, Section};
use std::collections::HashMap;

fn run_and_collect(engine: &mut Engine, sc: u16, up: bool, out: &mut Vec<InputEvent>) {
    if let KeyAction::Inject(evs) = engine.process_key(sc, false, up, false) {
        out.extend(evs);
    }
}

fn count_scancode(events: &[InputEvent], target_sc: u16) -> usize {
    events
        .iter()
        .filter(|e| {
            if let InputEvent::Scancode(sc, _, false) = e {
                *sc == target_sc
            } else {
                false
            }
        })
        .count()
}

fn create_dummy_layout() -> Layout {
    let mut layout = Layout::default();
    let mut sections = HashMap::new();

    // Create sections to satisfy engine
    let section = Section {
        name: "ローマ字シフト無し".to_string(),
        base_plane: Plane::default(),
        sub_planes: HashMap::new(),
    };
    sections.insert("ローマ字シフト無し".to_string(), section);

    // Required by Engine to enable thumb keys!
    let section_thumb = Section {
        name: "ローマ字左親指シフト".to_string(),
        base_plane: Plane::default(),
        sub_planes: HashMap::new(),
    };
    sections.insert("ローマ字左親指シフト".to_string(), section_thumb);

    // Trigger keys (Thumb shift) need sections too usually to be "active" but engine check relies on profile mainly.
    // But engine.load_layout builds trigger keys.
    // Here we manually set profile so it's fine.

    layout.sections = sections;
    layout
}

#[test]
fn test_thumb_repeat_space_enable() {
    let layout = create_dummy_layout();

    let mut engine = Engine::default();
    engine.set_ignore_ime(true);
    engine.load_layout(layout);

    let mut profile = engine.get_profile();
    // Configure Left Thumb: Space (0x39)
    profile.thumb_left.key = ThumbKeySelect::Space;
    profile.thumb_left.single_press = ThumbShiftSinglePress::Enable; // "有効"
    profile.thumb_left.repeat = true; // REPEAT ON

    // Also set char_key_repeat_unassigned to false to mimic strict environment where fallback might fail?
    // Or even if true, we want to ensure it works.
    // Let's try default first (which is true).

    engine.set_profile(profile);

    let mut all = Vec::new();

    // 1. Press Space (Down)
    run_and_collect(&mut engine, 0x39, false, &mut all);

    // 2. Press Space (Repeat 1)
    run_and_collect(&mut engine, 0x39, false, &mut all);

    // 3. Press Space (Repeat 2)
    run_and_collect(&mut engine, 0x39, false, &mut all);

    // 4. Release Space
    run_and_collect(&mut engine, 0x39, true, &mut all);

    let space_counts = count_scancode(&all, 0x39);

    println!("Events: {:?}", all);

    // Expect repeats.
    assert!(
        space_counts >= 2,
        "Expected multiple Space outputs for repeat (Space/Enable), got {}",
        space_counts
    );
}
