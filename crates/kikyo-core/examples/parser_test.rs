use kikyo_core::parser;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let path = Path::new("D:/Study/Kikyo/test_data/新下駄.yab");
    println!("Loading {:?}", path);

    let layout = parser::load_yab(path)?;
    println!("Loaded Layout successfully.");
    println!("Sections: {}", layout.sections.len());

    for (name, sec) in &layout.sections {
        println!("  Section: [{}]", name);
        println!("    Base Plane: {} cells", sec.base_plane.map.len());
        println!("    Sub Planes: {}", sec.sub_planes.len());
        for (tag, sub) in &sec.sub_planes {
            println!("      Sub <{}>: {} cells", tag, sub.map.len());
        }
    }

    Ok(())
}
