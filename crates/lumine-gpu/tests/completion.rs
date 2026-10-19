//! Explicit completion observations must not manufacture work or cross renderers.
use lumine::scene::{Color, NodeKind, QuadStyle, Rect, Scene, Style};
use lumine_gpu::GpuRenderer;

#[test]
fn completion_requires_new_work_and_rejects_other_renderers_even_on_the_same_device() {
    let mut gpu = GpuRenderer::new(16, 16).unwrap();
    let other = GpuRenderer::new_with_context(16, 16, &gpu.context()).unwrap();
    let different_device = GpuRenderer::new(16, 16).unwrap();
    assert!(gpu.submit_with_completion().is_err());
    let mut scene = Scene::new(16., 16.);
    let node = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(20, 40, 60, 255),
            ..Default::default()
        }),
        Style {
            width: Some(16.),
            height: Some(16.),
            ..Default::default()
        },
    );
    let damage = [Rect::new(0., 0., 16., 16.)];
    scene.flush();
    gpu.render(&scene, &damage).unwrap();
    gpu.wait_submitted().unwrap(); // Must leave this pending render unsubmitted.
    let first = gpu.submit_with_completion().unwrap();
    assert!(gpu.submit_with_completion().is_err());
    assert!(other.wait_completion(&first).is_err());
    assert!(different_device.wait_completion(&first).is_err());
    let mut tickets = vec![first];
    for color in [Color(80, 60, 40, 255), Color(120, 100, 80, 255)] {
        scene.set_kind(
            node,
            NodeKind::Quad(QuadStyle {
                fill: color,
                ..Default::default()
            }),
        );
        scene.flush();
        gpu.render(&scene, &damage).unwrap();
        if tickets.len() == 1 {
            tickets.push(gpu.submit_with_completion().unwrap());
        }
    }
    gpu.wait_completion(&tickets[0]).unwrap();
    assert!(tickets[0].is_complete());
    // Waiting for the oldest also leaves the third render pending.
    tickets.push(gpu.submit_with_completion().unwrap());
    gpu.poll_completions().unwrap();
    gpu.wait_completion(tickets.last().unwrap()).unwrap();
    assert!(tickets.iter().all(|ticket| ticket.is_complete()));
    assert!(
        gpu.readback()
            .unwrap()
            .chunks_exact(4)
            .all(|pixel| pixel == [120, 100, 80, 255])
    );
    // A clean render must not yield a ticket for the previous completed frame.
    gpu.render(&scene, &[]).unwrap();
    assert!(gpu.submit_with_completion().is_err());
}
