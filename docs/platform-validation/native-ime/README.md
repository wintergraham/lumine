Native Linux input-method validation
====================================

```sh
sudo apt-get install --no-install-recommends ibus ibus-libpinyin fonts-droid-fallback
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example native_ime
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader python3 scripts/native_ime_smoke.py /tmp/lumine-target/debug/examples/native_ime --output docs/platform-validation/native-ime
```

The probe uses real IBus/libpinyin through XIM. The script owns Xvfb, Openbox,
a private session D-Bus, and IBus with its XIM bridge, candidate panel, and engine.
It gives IBus private XDG configuration/data/cache/runtime directories and sets
`XMODIFIERS=@im=ibus` only for the test processes. It enables
`use-system-keyboard-layout` in that private configuration: otherwise `ibus engine`
tries to apply libpinyin's `default` layout with setxkbmap and fails on this Xvfb
seat. Neither the script nor the example synthesizes `InputEvent::Ime*` events.
The example observes incoming native composition events and retained model/text/
caret geometry. See the [IBus XIM setup instructions](https://github.com/ibus/ibus/wiki/ReadMe)
and [libpinyin engine source](https://github.com/libpinyin/ibus-libpinyin).

Eight native snapshots verify:

1. Typing `nihao` displays preedit `你好` while the committed model stays empty.
2. Space commits `你好` exactly once.
3. Typing `zhong` displays `你好中` without committing `中`.
4. Escape cancels that composition and restores the displayed committed text.
5. Typing `wo` displays pending `你好我`.
6. Clicking the second editor cancels pending `我` and preserves both models.
7. Typing `shijie` displays second-editor preedit `世界` while its model stays empty.
8. Space commits `世界` exactly once; the final models are `你好` and `世界`.

For every active-composition snapshot, the script finds the actual visible IBus
candidate X window and compares its screen position with the retained caret's
bottom edge plus the native client origin. The popup must follow both horizontal
caret movement and the second editor's vertical position within 3px. Screenshots
were inspected: the candidate panel appears below the composing text, and
preedit, committed text, and focus borders remain visible.
Chinese glyphs are readable through the installed Droid Sans Fallback font
(`fc-match 'sans:charset=4f60'`); no large font asset is bundled with lumine.

This exposed a host integration issue: winit 0.30.13's X11 implementation ignores
the size passed to `set_ime_cursor_area`. The desktop host now supplies the caret
baseline as the X11 spot; rectangle-based backends retain their rectangle.

`application.log` records actual preedit/commit events and model/geometry samples;
`result.json` stores all eight asserted snapshots, including candidate rectangles.
`metadata.json` records exact binary/loader/source-archive hashes, build/run
commands, compiler, IBus/libpinyin package versions, and the Chinese fallback
font. The archived Rust sources have no timestamps newer than the tested binary.
The script cleans its processes and process groups on success or failure, then
removes the private configuration. No IBus test processes remain after cleanup.
The `native_ime` example closes itself after 20 seconds. Strict example Clippy passes.

Dependencies: Xvfb, Openbox, xdotool, xwininfo, D-Bus, gsettings, IBus with its GTK
candidate panel and libpinyin, and Pillow. This is Linux X11/XIM software-rendered
validation, not Wayland IME, physical-keyboard, hardware-GPU, or macOS evidence.
