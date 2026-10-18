use lumine::{compose::prelude::*, text_layout::WordWrap, widgets::Ui};

#[test]
fn word_policy_inherits_resets_and_relayouts_retained_text() {
    let mut ui = Ui::new(300., 200.);
    let policy = ui.signal(WordWrap::Word);
    let read = policy.clone();
    let view = ui.mount(
        column()
            .text_size(10.)
            .text_wrap(true)
            .reactive_style(move || Styles::new().word_wrap(read.get()))
            .children([
                text("Active").w(35.99).id("inherited"),
                text("Active")
                    .w(35.99)
                    .word_wrap(WordWrap::WordOrGlyph)
                    .id("native"),
                text("Active").w(10.).text_wrap(false).id("nowrap"),
            ]),
    );
    ui.prepare_frame();
    let inherited = view.find("inherited").unwrap();
    let native = view.find("native").unwrap();
    let nowrap = view.find("nowrap").unwrap();
    assert_eq!(ui.scene.borrow().bounds(inherited).height, 14.);
    assert_eq!(ui.scene.borrow().bounds(native).height, 28.);
    assert_eq!(ui.scene.borrow().bounds(nowrap).height, 14.);
    policy.set(WordWrap::WordOrGlyph);
    ui.prepare_frame();
    assert_eq!(view.find("inherited"), Some(inherited));
    assert_eq!(ui.scene.borrow().bounds(inherited).height, 28.);
    policy.set(WordWrap::Word);
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().bounds(inherited).height, 14.);
    assert_eq!(ui.scene.borrow().font(inherited).word_wrap, WordWrap::Word);
    assert_eq!(
        ui.scene.borrow().font(native).word_wrap,
        WordWrap::WordOrGlyph
    );
}
