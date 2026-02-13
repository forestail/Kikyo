use criterion::{black_box, criterion_group, criterion_main, Criterion};
use kikyo_core::engine::Engine;
use kikyo_core::parser::parse_yab_content;

const BENCH_LAYOUT: &str = r#"
[ローマ字シフト無し]
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
q,w,e,xx,xx,xx,xx,xx,xx,xx,xx,xx
a,s,d,f,xx,xx,xx,k,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<k>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,x,y,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx

<q><w>
xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx,xx
xx,xx,a,xx,xx,xx,xx,xx,xx,xx,xx,xx
"#;

fn make_engine(char_overlap_ratio: f64) -> Engine {
    let layout = parse_yab_content(BENCH_LAYOUT).expect("failed to parse benchmark layout");
    let mut engine = Engine::default();
    engine.set_ignore_ime(true);
    engine.load_layout(layout);

    let mut profile = engine.get_profile();
    profile.char_key_overlap_ratio = char_overlap_ratio;
    engine.set_profile(profile);

    engine
}

fn bench_single_tap(c: &mut Criterion) {
    let mut engine = make_engine(0.35);
    c.bench_function("engine/single_tap_defined_key", |b| {
        b.iter(|| {
            black_box(engine.process_key(0x1E, false, false, false)); // A down
            black_box(engine.process_key(0x1E, false, true, false)); // A up
        });
    });
}

fn bench_undefined_passthrough(c: &mut Criterion) {
    let mut engine = make_engine(0.35);
    c.bench_function("engine/undefined_key_passthrough", |b| {
        b.iter(|| {
            black_box(engine.process_key(0x2C, false, false, false)); // Z down
            black_box(engine.process_key(0x2C, false, true, false)); // Z up
        });
    });
}

fn bench_two_key_chord(c: &mut Criterion) {
    let mut engine = make_engine(0.0);
    c.bench_function("engine/two_key_chord_k_plus_d", |b| {
        b.iter(|| {
            black_box(engine.process_key(0x25, false, false, false)); // K down
            black_box(engine.process_key(0x20, false, false, false)); // D down
            black_box(engine.process_key(0x20, false, true, false)); // D up
            black_box(engine.process_key(0x25, false, true, false)); // K up
        });
    });
}

fn bench_three_key_chord(c: &mut Criterion) {
    let mut engine = make_engine(0.0);
    c.bench_function("engine/three_key_chord_q_w_e", |b| {
        b.iter(|| {
            black_box(engine.process_key(0x10, false, false, false)); // Q down
            black_box(engine.process_key(0x11, false, false, false)); // W down
            black_box(engine.process_key(0x12, false, false, false)); // E down
            black_box(engine.process_key(0x10, false, true, false)); // Q up
            black_box(engine.process_key(0x11, false, true, false)); // W up
            black_box(engine.process_key(0x12, false, true, false)); // E up
        });
    });
}

criterion_group!(
    benches,
    bench_single_tap,
    bench_undefined_passthrough,
    bench_two_key_chord,
    bench_three_key_chord
);
criterion_main!(benches);
