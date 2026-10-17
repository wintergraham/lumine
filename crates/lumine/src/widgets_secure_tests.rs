use super::*;
use crate::{input::Modifiers, text_layout::FallbackTextLayout};

fn fixture(source: &str, width: f32, multiline: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(300., 250.);
    let value = ui.signal(source.to_owned());
    let editor = ui.text_input(ui.root(), "Password", value, width, multiline);
    editor.set_typography(ui.theme.text, 10., FontStyle::default());
    editor.set_secure(true);
    ui.prepare_frame();
    ui.input.focus(&ui.scene, Some(editor.node));
    (ui, editor)
}

fn key(ui: &mut Ui, key: Key, modifiers: Modifiers) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers,
        repeat: false,
    });
    ui.prepare_frame();
}

fn paint_text(ui: &Ui, editor: &EditorHandle) -> String {
    ui.prepare_frame();
    let mut scene = ui.scene.borrow_mut();
    scene.flush();
    let painted: Vec<_> = scene
        .paint_items()
        .filter_map(|item| match item.kind {
            NodeKind::Text { text, .. } => Some(text.to_string()),
            NodeKind::RichText { .. } => panic!("secure editor retained rich source text"),
            _ => None,
        })
        .collect();
    assert_eq!(painted.len(), 1);
    assert!(matches!(scene.kind(editor.text), NodeKind::Text { .. }));
    painted[0].clone()
}

#[test]
fn secure_paint_shaping_measurement_and_cache_never_receive_source_text() {
    let source = "a👩‍💻e\u{301}z";
    let (ui, editor) = fixture(source, 180., false);
    editor.set_secure(false);
    let shaped = Rc::new(RefCell::new(Vec::new()));
    let observed = shaped.clone();
    ui.scene.borrow_mut().set_font_text_shaper(
        move |text: &str, size, width, font: &FontStyle| -> Box<dyn TextLayout> {
            observed.borrow_mut().push(text.to_owned());
            // Different plaintext metrics catch accidentally unmasked measurement.
            let size = if text.chars().all(|c| c == '•') {
                size
            } else {
                size * 2.
            };
            Box::new(FallbackTextLayout::with_font(text, size, width, font))
        },
    );
    editor.set_secure(true);
    assert_eq!(paint_text(&ui, &editor), "••••");
    assert_eq!(editor.measure_text(None), (24., 14.));
    assert!(shaped.borrow().iter().all(|text| text == "••••"));
    assert_eq!(editor.value.get(), source);
    let first = ui
        .storage
        .editor_layout
        .borrow()
        .as_ref()
        .unwrap()
        .layout
        .clone();
    editor.set_selection(1, 15);
    let cache = ui.storage.editor_layout.borrow();
    let entry = cache.as_ref().unwrap();
    assert_eq!(entry.text, "••••");
    assert_eq!(
        entry.secure_offsets.as_deref(),
        Some([0, 1, 12, 15, 16].as_slice())
    );
    assert!(Rc::ptr_eq(&first, &entry.layout));
    drop(cache);
    // Identical bullet text with different source byte lengths must replace the map.
    editor.value.set("é👩‍💻e\u{301}z".into());
    let cache = ui.storage.editor_layout.borrow();
    let entry = cache.as_ref().unwrap();
    assert_eq!(
        entry.secure_offsets.as_deref(),
        Some([0, 2, 13, 16, 17].as_slice())
    );
    assert!(!Rc::ptr_eq(&first, &entry.layout));
    assert_eq!(entry.layout.hit_test(7., 1.), 2);
    drop(cache);
    editor.value.set("x".repeat(EDITOR_CACHE_TEXT_LIMIT + 1));
    assert!(ui.storage.editor_layout.borrow().is_none());
    // Secure policy also applies to a handle surviving its document.
    drop(ui);
    shaped.borrow_mut().clear();
    editor.refresh();
    assert!(
        shaped
            .borrow()
            .iter()
            .all(|text| text.chars().all(|c| c == '•'))
    );
}

#[test]
fn masked_layout_maps_unicode_hits_carets_selections_and_visual_affinity() {
    for source in ["", "a👩‍💻e\u{301}z", "🇬🇧👍🏽אב\r\n👨‍👩‍👧‍👦"]
    {
        let masked = MaskedText::new(source);
        let expected = FallbackTextLayout::new(&masked.text, 10., Some(12.));
        let layout = MaskedLayout {
            layout: Box::new(FallbackTextLayout::new(&masked.text, 10., Some(12.))),
            source_offsets: masked.source_offsets.clone(),
        };
        assert_eq!(layout.size(), expected.size());
        for (index, offset) in masked.source_offsets.iter().copied().enumerate() {
            for affinity in [TextAffinity::Before, TextAffinity::After] {
                let position = TextPosition {
                    byte_offset: offset,
                    affinity,
                };
                let display = TextPosition {
                    byte_offset: index * "•".len(),
                    affinity,
                };
                assert_eq!(
                    layout.caret_position(position),
                    expected.caret_position(display)
                );
                for end in [false, true] {
                    let edge = expected.visual_line_edge(display, end);
                    assert_eq!(
                        layout.visual_line_edge(position, end),
                        TextPosition {
                            byte_offset: masked.source_offset(edge.byte_offset),
                            ..edge
                        }
                    );
                }
                let (start, end) = expected.visual_line_range(display);
                assert_eq!(
                    layout.visual_line_range(position),
                    (
                        TextPosition {
                            byte_offset: masked.source_offset(start.byte_offset),
                            ..start
                        },
                        TextPosition {
                            byte_offset: masked.source_offset(end.byte_offset),
                            ..end
                        },
                    )
                );
            }
            assert_eq!(layout.caret(offset), expected.caret(index * "•".len()));
            if let Some(end) = masked.source_offsets.get(index + 1).copied() {
                let caret = expected.caret(index * "•".len());
                assert_eq!(layout.hit_test(caret.x + 1., caret.y + 1.), offset);
                let hit = layout.hit_position(caret.x + 5., caret.y + 1.);
                assert_eq!(hit.byte_offset, end);
                assert_eq!(hit.affinity, TextAffinity::Before);
                let selection = expected.selection(index * 3..(index + 1) * 3);
                assert_eq!(layout.selection(offset..end), selection);
                assert_eq!(layout.selection(end..offset), selection);
                if end - offset > 1 {
                    assert_eq!(layout.caret(offset + 1), layout.caret(offset));
                    assert_eq!(layout.selection(offset + 1..end), selection);
                }
            }
        }
        assert!(layout.selection(source.len()..source.len()).is_empty());
        assert_eq!(layout.hit_test(f32::MAX, f32::MAX), source.len());
    }
}

#[test]
fn toggling_preserves_identity_focus_selection_history_and_blocks_copy_and_cut() {
    let source = "a👩‍💻e\u{301}z";
    let (mut ui, editor) = fixture(source, 180., false);
    let retained = editor.clone();
    editor.paste("!");
    editor.set_selection(15, 1);
    let selection = editor.editor.borrow().selection();
    let revision = editor.editor.borrow().composition_cancel_revision();
    assert_eq!(editor.copy(), "");
    assert_eq!(editor.cut(), "");
    assert_eq!(editor.value.get(), format!("{source}!"));
    for secure in [false, true, false] {
        editor.set_secure(secure);
        assert_eq!(retained.is_secure(), secure);
        assert!(Rc::ptr_eq(&editor.editor, &retained.editor));
        assert_eq!(ui.input.focused(), Some(editor.node));
        assert_eq!(editor.editor.borrow().selection(), selection);
        assert_eq!(
            editor.editor.borrow().composition_cancel_revision(),
            revision
        );
    }
    assert_eq!(editor.copy(), "👩‍💻e\u{301}");
    let command = Modifiers {
        control: true,
        meta: true,
        ..Default::default()
    };
    key(&mut ui, Key::Character("z".into()), command);
    assert_eq!(editor.value.get(), source);
    editor.set_secure(true);
    key(
        &mut ui,
        Key::Character("z".into()),
        Modifiers {
            shift: true,
            ..command
        },
    );
    assert_eq!(editor.value.get(), format!("{source}!"));
    editor.set_read_only(true);
    editor.select_all();
    assert_eq!(editor.copy(), "");
    assert_eq!(editor.cut(), "");
    editor.set_secure(false);
    assert_eq!(editor.copy(), format!("{source}!"));
}

#[test]
fn secure_pointer_wrap_navigation_scroll_and_accessibility_use_masked_positions() {
    let source = "a👩‍💻e\u{301}z";
    let (mut ui, editor) = fixture(source, 28., true);
    editor.set_wrap(true);
    editor.set_selection(0, 0);
    ui.prepare_frame();
    let start = ui.scene.borrow().bounds(editor.caret);
    key(&mut ui, Key::End, Modifiers::default());
    assert_eq!(editor.editor.borrow().selection().focus, 12);
    assert_eq!(ui.scene.borrow().bounds(editor.caret).y, start.y);
    key(&mut ui, Key::ArrowDown, Modifiers::default());
    assert_eq!(editor.editor.borrow().selection().focus, source.len());
    editor.set_selection(0, 0);
    ui.dispatch(InputEvent::PointerDown {
        x: start.x + 7.,
        y: start.y + 1.,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerUp {
        x: start.x + 7.,
        y: start.y + 1.,
        button: PointerButton::Primary,
    });
    assert_eq!(editor.editor.borrow().selection().focus, 1);
    editor.set_selection(15, 1);
    let semantics = ui.semantics.borrow();
    let node = semantics.get(editor.node).unwrap();
    assert_eq!(node.value.as_deref(), Some("••••"));
    assert_eq!(node.text_selection, Some((9, 3)));
    drop(semantics);
    assert!(ui.set_accessible_text_selection(editor.node, 6, 3));
    assert_eq!(
        editor.editor.borrow().selection(),
        crate::text_edit::Selection {
            anchor: 12,
            focus: 1
        }
    );
    editor.set_wrap(false);
    editor.value.set("a\nz".into());
    editor.set_selection(1, 1);
    key(&mut ui, Key::End, Modifiers::default());
    assert_eq!(editor.editor.borrow().selection().focus, 3);
    key(&mut ui, Key::Home, Modifiers::default());
    assert_eq!(editor.editor.borrow().selection().focus, 0);
    editor.value.set("👩‍💻".repeat(30));
    editor.set_selection(editor.value.get().len(), editor.value.get().len());
    ui.prepare_frame();
    let caret = ui.scene.borrow().bounds(editor.caret);
    let root = ui.scene.borrow().bounds(editor.node);
    assert!(caret.x >= root.x && caret.x + caret.width <= root.x + root.width);
    assert_eq!(
        editor.editor.borrow().selection().focus,
        editor.value.get().len()
    );
}

#[test]
fn secure_ime_masks_preedit_and_preserves_it_through_visibility_changes() {
    let source = "a👩‍💻z";
    let (mut ui, editor) = fixture(source, 180., false);
    editor.set_selection(1, 12);
    ui.dispatch(InputEvent::ImePreedit {
        text: "e\u{301}🙂".into(),
        cursor: Some((3, 3)),
    });
    assert_eq!(paint_text(&ui, &editor), "••••");
    assert_eq!(editor.measure_text(None), (24., 14.));
    assert_eq!(editor.value.get(), source);
    let caret = ui.scene.borrow().bounds(editor.caret);
    let text = ui.scene.borrow().bounds(editor.text);
    assert_eq!(caret.x - text.x, 12.);
    let preedit = editor.editor.borrow().preedit().cloned();
    let revision = editor.editor.borrow().composition_cancel_revision();
    editor.set_secure(false);
    editor.set_secure(true);
    assert_eq!(editor.editor.borrow().preedit().cloned(), preedit);
    assert_eq!(
        editor.editor.borrow().composition_cancel_revision(),
        revision
    );
    assert_eq!(paint_text(&ui, &editor), "••••");
    ui.dispatch(InputEvent::ImeCommit("e\u{301}🙂".into()));
    assert_eq!(editor.value.get(), "ae\u{301}🙂z");
    assert!(editor.editor.borrow().preedit().is_none());
    // A combining preedit joins the preceding committed grapheme into one bullet.
    editor.set_selection(1, 1);
    ui.dispatch(InputEvent::ImePreedit {
        text: "\u{301}".into(),
        cursor: Some((2, 2)),
    });
    assert_eq!(paint_text(&ui, &editor), "••••");
    ui.dispatch(InputEvent::ImePreedit {
        text: String::new(),
        cursor: None,
    });
    assert_eq!(editor.value.get(), "ae\u{301}🙂z");
}

#[test]
fn secure_inline_objects_are_masked_and_normal_object_behavior_returns_when_visible() {
    let (ui, editor) = fixture("a\u{e000}b", 180., false);
    editor
        .set_inline_objects(vec![InlineObject {
            marker: '\u{e000}',
            width: 80.,
            text: "Private object".into(),
        }])
        .unwrap();
    assert_eq!(paint_text(&ui, &editor), "•••");
    assert!(editor.inline_object_bounds().get().is_empty());
    assert_eq!(editor.measure_text(None), (18., 14.));
    editor.set_secure(false);
    ui.prepare_frame();
    assert!(matches!(
        ui.scene.borrow().kind(editor.text),
        NodeKind::RichText { .. }
    ));
    assert_eq!(editor.inline_object_bounds().get()[0].rect.width, 80.);
    assert_eq!(editor.measure_text(None), (92., 14.));
    editor.set_selection(1, 4);
    assert_eq!(editor.copy(), "Private object");
    assert_eq!(editor.cut(), "Private object");
    assert_eq!(editor.value.get(), "ab");
}
