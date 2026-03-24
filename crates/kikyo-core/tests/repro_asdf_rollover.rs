use kikyo_core::chord_engine::ImeMode;
use kikyo_core::engine::Engine;
use kikyo_core::types::{InputEvent, KeyAction};

fn collect_down_scancodes(actions: &[KeyAction]) -> Vec<u16> {
    let mut out = Vec::new();
    for action in actions {
        if let KeyAction::Inject(evs) = action {
            for ev in evs {
                if let InputEvent::Scancode(sc, _ext, up) = ev {
                    if !up {
                        out.push(*sc);
                    }
                }
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Ev {
    ADn,
    SDn,
    DDn,
    FDn,
    AUp,
    SUp,
    DUp,
    FUp,
}

fn generate_all_asdf_orders() -> Vec<Vec<Ev>> {
    fn rec(seq: &mut Vec<Ev>, out: &mut Vec<Vec<Ev>>, d: usize, up_done: [bool; 4]) {
        if seq.len() == 8 {
            out.push(seq.clone());
            return;
        }

        // Next down in fixed order: A,S,D,F
        if d < 4 {
            let ev = match d {
                0 => Ev::ADn,
                1 => Ev::SDn,
                2 => Ev::DDn,
                _ => Ev::FDn,
            };
            seq.push(ev);
            rec(seq, out, d + 1, up_done);
            seq.pop();
        }

        // Ups can occur after the corresponding down is already emitted.
        for i in 0..4 {
            if up_done[i] || i >= d {
                continue;
            }
            let ev = match i {
                0 => Ev::AUp,
                1 => Ev::SUp,
                2 => Ev::DUp,
                _ => Ev::FUp,
            };
            let mut next = up_done;
            next[i] = true;
            seq.push(ev);
            rec(seq, out, d, next);
            seq.pop();
        }
    }

    let mut out = Vec::new();
    rec(&mut Vec::new(), &mut out, 0, [false; 4]);
    out
}

fn run_order(engine: &mut Engine, order: &[Ev]) -> Vec<u16> {
    let mut actions = Vec::new();
    for ev in order {
        let action = match ev {
            Ev::ADn => engine.process_key(0x1E, false, false, false),
            Ev::SDn => engine.process_key(0x1F, false, false, false),
            Ev::DDn => engine.process_key(0x20, false, false, false),
            Ev::FDn => engine.process_key(0x21, false, false, false),
            Ev::AUp => engine.process_key(0x1E, false, true, false),
            Ev::SUp => engine.process_key(0x1F, false, true, false),
            Ev::DUp => engine.process_key(0x20, false, true, false),
            Ev::FUp => engine.process_key(0x21, false, true, false),
        };
        actions.push(action);
    }
    collect_down_scancodes(&actions)
}

#[test]
fn sweep_asdf_rollover_orders_on_naginata_layout() {
    let layout_path = format!(
        "{}/../../layout/薙刀式配列v17ベスト版(JIS縦書き).kky",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(&layout_path).expect("layout read failed");
    let layout = kikyo_core::parser::parse_layout_content(
        &content,
        &kikyo_core::keyboard_map::new_jis_106(),
    )
    .expect("layout parse failed");

    let orders = generate_all_asdf_orders();
    let mut failures = Vec::new();

    for order in orders {
        let mut engine = Engine::default();
        engine.set_ime_mode(ImeMode::ForceAlpha);
        engine.load_layout(layout.clone());

        let mut profile = engine.get_profile();
        profile.char_key_continuous = true;
        profile.char_key_overlap_ratio = 0.5;
        engine.set_profile(profile);

        let downs = run_order(&mut engine, &order);
        if downs != vec![0x1E, 0x1F, 0x20, 0x21] {
            failures.push((order, downs));
        }
    }

    assert!(
        failures.is_empty(),
        "unexpected outputs for some asdf rollover orders: {:?}",
        failures
    );
}
