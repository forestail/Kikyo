use kikyo_core::{engine, keyboard_hook, parser};
use std::path::Path;
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("Starting Full Hook Test...");
    // println!("Press Ctrl + Alt + Esc to emergency stop."); // Currently disabled.

    // Load layout
    let path = Path::new("D:/Study/Kikyo/test_data/新下駄.yab");
    if path.exists() {
        println!("Loading layout from {:?}", path);
        let layout = parser::load_yab(path)?;
        engine::ENGINE.lock().load_layout(layout);
        engine::ENGINE.lock().set_enabled(true);
        println!("Engine enabled with layout.");
    } else {
        println!(
            "Layout not found at {:?}, running in pass-through mode.",
            path
        );
    }

    // Install hook
    keyboard_hook::install_hook()?;

    // Message loop
    let mut msg = MSG::default();
    unsafe { while GetMessageW(&mut msg, None, 0, 0).as_bool() {} }

    Ok(())
}
