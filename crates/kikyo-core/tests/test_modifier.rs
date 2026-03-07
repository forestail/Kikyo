#[test]
fn test_ctrl() {
    let mut engine = kikyo_core::engine::Engine::default();
    println!(
        "needs_left_ctrl_handling: {}",
        engine.needs_left_ctrl_handling()
    );
    let layout_content = std::fs::read_to_string("../../layout/新下駄配列.kky").unwrap();
    let kb_map = kikyo_core::keyboard_map::new_jis_106();
    let layout = kikyo_core::parser::parse_layout_content(&layout_content, &kb_map).unwrap();
    engine.load_layout(layout);
    println!(
        "needs_left_ctrl_handling after load: {}",
        engine.needs_left_ctrl_handling()
    );
}
