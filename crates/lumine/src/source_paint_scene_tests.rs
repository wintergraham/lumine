use super::*;
use crate::source_paint::{SourcePaintCapability, SourcePaintPolicy, SourcePaintScope};
use std::num::NonZeroU64;

fn capability(owner: u64) -> SourcePaintCapability {
    SourcePaintCapability {
        owner: NonZeroU64::new(owner).unwrap(),
        contract_sha256: [owner as u8; 32],
        policy: SourcePaintPolicy::SkiaSoftwareSrgb8OpaqueBackdrop,
    }
}
fn node(scene: &mut Scene, parent: NodeId) -> NodeId {
    scene.append(
        parent,
        NodeKind::Rect(Color(169, 190, 230, 20)),
        Style {
            width: Some(20.),
            height: Some(20.),
            ..Default::default()
        },
    )
}
fn observed(scene: &Scene, id: NodeId) -> Option<SourcePaintCapability> {
    scene
        .paint_items()
        .find(|item| item.id == id)
        .unwrap()
        .source_paint_capability
}
fn mark(scene: &mut Scene, id: NodeId, owner: u64) {
    scene.set_source_paint_scope(id, Some(SourcePaintScope::Capability(capability(owner))));
}

#[test]
fn source_paint_scope_only_reaches_actual_source_descendants() {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    let chrome = node(&mut scene, root);
    let source = node(&mut scene, root);
    let leaf = node(&mut scene, source);
    let error_overlay = node(&mut scene, root);
    assert!(!scene.has_source_paint_scopes());
    assert!(!scene.has_source_paint_capabilities());
    assert!(
        scene
            .paint_items()
            .all(|item| item.source_paint_capability.is_none())
    );
    mark(&mut scene, source, 1);
    scene.flush();
    assert_eq!(observed(&scene, source), Some(capability(1)));
    assert_eq!(observed(&scene, leaf), Some(capability(1)));
    for native in [root, chrome, error_overlay] {
        assert_eq!(observed(&scene, native), None);
    }
    for item in scene.layer_items(None) {
        assert_eq!(item.source_paint_capability, observed(&scene, item.id));
    }
    // Starting traversal below the marker cannot lose ancestor ownership.
    assert_eq!(
        scene.layer_items(Some(leaf))[0].source_paint_capability,
        Some(capability(1))
    );
}

#[test]
fn source_paint_nested_conflict_and_preserve_boundaries_are_sticky() {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    let source = node(&mut scene, root);
    let nested = node(&mut scene, source);
    let deep = node(&mut scene, nested);
    mark(&mut scene, source, 1);
    mark(&mut scene, nested, 1);
    assert_eq!(observed(&scene, deep), Some(capability(1)));
    mark(&mut scene, nested, 2);
    mark(&mut scene, deep, 1);
    assert_eq!(observed(&scene, nested), None);
    assert_eq!(observed(&scene, deep), None);
    scene.flush();
    assert_eq!(
        scene.layer_items(Some(deep))[0].source_paint_capability,
        None
    );
    scene.set_source_paint_scope(nested, Some(SourcePaintScope::PreserveExisting));
    assert_eq!(observed(&scene, deep), None);
    scene.set_source_paint_scope(nested, None);
    assert_eq!(observed(&scene, deep), Some(capability(1)));
    scene.set_source_paint_scope(source, None);
    scene.set_source_paint_scope(deep, None);
    scene.set_source_paint_scope(nested, Some(SourcePaintScope::PreserveExisting));
    assert!(scene.has_source_paint_scopes());
    assert!(!scene.has_source_paint_capabilities());
}

#[test]
fn source_paint_reparent_recomputes_actual_ancestry_without_cached_ownership() {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    let source = node(&mut scene, root);
    let native = node(&mut scene, root);
    let leaf = node(&mut scene, source);
    mark(&mut scene, source, 1);
    assert_eq!(observed(&scene, leaf), Some(capability(1)));
    // Scene has no public cross-parent move API. Exercise the actual retained
    // ancestry mutation here, with the same layout invalidation a mover needs.
    scene.node_mut(source).children.retain(|id| *id != leaf);
    scene.node_mut(native).children.push(leaf);
    scene.node_mut(leaf).parent = Some(native);
    scene.invalidate_layout(source);
    scene.invalidate_layout(native);
    scene.flush();
    assert_eq!(observed(&scene, leaf), None);
    assert_eq!(
        scene.layer_items(Some(leaf))[0].source_paint_capability,
        None
    );
    mark(&mut scene, native, 2);
    assert_eq!(observed(&scene, leaf), Some(capability(2)));
    // A moved root carrying a conflicting contract must fail closed.
    mark(&mut scene, leaf, 1);
    assert_eq!(observed(&scene, leaf), None);
}

#[test]
fn source_paint_removal_and_slot_reuse_do_not_retain_markers() {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    let source = node(&mut scene, root);
    mark(&mut scene, source, 1);
    assert!(scene.has_source_paint_scopes());
    scene.remove(source);
    let native = node(&mut scene, root);
    assert_eq!(source.index, native.index);
    assert_ne!(source.generation, native.generation);
    assert!(!scene.has_source_paint_scopes());
    assert!(!scene.has_source_paint_capabilities());
    assert_eq!(scene.source_paint_scope(native), None);
    assert_eq!(observed(&scene, native), None);
}

#[test]
fn source_paint_retained_toggle_invalidates_layers_scroll_and_full_viewport() {
    let mut scene = Scene::new(100., 100.);
    let root = scene.root();
    let source = node(&mut scene, root);
    let child = node(&mut scene, source);
    scene.set_scroll_copy(source, true);
    scene.set_isolated(child, true);
    scene.flush();
    scene.set_transform(source, Transform { x: 0., y: -2. });
    assert!(!scene.pending_moves.is_empty());
    let raster = scene.raster_revision();
    let source_layer = scene.layer_revision(source);
    let child_layer = scene.layer_revision(child);
    let child_paint = scene.paint_revision(child);
    mark(&mut scene, source, 1);
    assert!(scene.pending_moves.is_empty());
    assert!(scene.scroll_moves().is_empty());
    assert!(scene.raster_revision() > raster);
    assert!(scene.layer_revision(source) > source_layer);
    assert!(scene.layer_revision(child) > child_layer);
    assert!(scene.paint_revision(child) > child_paint);
    let report = scene.flush();
    assert!(report.damage.iter().any(|damage| *damage == scene.viewport));
    let raster = scene.raster_revision();
    mark(&mut scene, source, 1);
    assert_eq!(scene.raster_revision(), raster, "same marker is a no-op");
    assert!(scene.flush().is_idle());
    scene.set_source_paint_scope(source, None);
    assert!(!scene.has_source_paint_scopes());
    assert_eq!(observed(&scene, source), None);
    assert!(
        scene
            .flush()
            .damage
            .iter()
            .any(|damage| *damage == scene.viewport)
    );
}
