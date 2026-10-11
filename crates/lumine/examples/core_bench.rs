//! CPU-only invalidation microbenchmarks; not equivalent to rendered framework comparisons.
use std::{hint::black_box, time::Instant};
use lumine::{
    scene::{Color, Layout, NodeKind, Scene, Style, Transform},
    virtual_list::VirtualList,
};

fn measure(name: &str, iterations: usize, mut operation: impl FnMut(usize)) {
    let start = Instant::now();
    for index in 0..iterations {
        operation(index);
    }
    let elapsed = start.elapsed();
    println!(
        "{name}: {iterations} iterations, {:.1} ns/iteration",
        elapsed.as_nanos() as f64 / iterations as f64
    );
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>().expect("positive iteration count"))
        .unwrap_or(100_000)
        .max(1);
    let mut scene = Scene::new(1000.0, 700.0);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let stream = scene.append(
        scene.root(),
        NodeKind::Text {
            text: "ready".into(),
            color: Color(255, 255, 255, 255),
            font_size: 16.0,
        },
        Style {
            width: Some(400.0),
            height: Some(100.0),
            ..Style::default()
        },
    );
    // Retain siblings to ensure text writes do not accidentally invalidate the tree.
    for _ in 0..1000 {
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(50, 50, 50, 255)),
            Style {
                width: Some(10.0),
                height: Some(10.0),
                ..Style::default()
            },
        );
    }
    scene.flush();
    measure("idle (1002 retained nodes)", iterations, |_| {
        assert!(black_box(scene.flush()).is_idle());
    });
    let mut layouts = 0;
    let mut paints = 0;
    measure("fixed text update + flush", iterations, |i| {
        scene.set_text(stream, if i % 2 == 0 { "stream A" } else { "stream B" });
        let frame = black_box(scene.flush());
        layouts += frame.layout_nodes;
        paints += frame.paint_nodes;
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(frame.paint_nodes, 1);
        assert_eq!(frame.damage.len(), 1);
    });
    println!("  layout_nodes={layouts}, paint_nodes={paints}; expected 0 and {iterations}");
    let mut composites = 0;
    measure("leaf translation + flush", iterations, |i| {
        scene.set_transform(
            stream,
            Transform {
                x: (i % 2 + 1) as f32,
                y: 0.0,
            },
        );
        let frame = black_box(scene.flush());
        composites += frame.composite_nodes;
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(frame.paint_nodes, 0);
        assert_eq!(frame.composite_nodes, 1);
    });
    println!("  composite_nodes={composites}; expected {iterations}");
    let virtual_list = VirtualList::new(1_000_000, 24.0, 3);
    let mut maximum = 0;
    measure("million-row virtual range", iterations, |i| {
        let range = black_box(virtual_list.visible_range((i as f32 * 11.0) % 24_000_000.0, 480.0));
        maximum = maximum.max(range.len());
        assert!(range.len() <= 27);
    });
    println!(
        "  maximum_mounted_rows={maximum}; expected <=27; range computation retains no row data"
    );
}
