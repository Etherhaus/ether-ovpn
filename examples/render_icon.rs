//! Renders assets/vpndeck.svg to a PNG: cargo run --example render_icon -- <size> <out.png>
fn main() {
    let mut args = std::env::args().skip(1);
    let size: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    let out = args.next().unwrap_or_else(|| format!("assets/vpndeck-{size}.png"));
    let svg = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vpndeck.svg")).unwrap();
    let tree = resvg::usvg::Tree::from_data(&svg, &Default::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(&out).unwrap();
}
