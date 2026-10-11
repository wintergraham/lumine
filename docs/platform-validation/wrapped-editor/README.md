# Native wrapped editor

The final owned Xvfb/Openbox run exercises the public `text_area(...).text_wrap(true)`
API through real X11 keyboard, pointer and resize events. The five captured stages
verify visual-line selection, appending without inserted model line breaks,
reflow while focused, caret visibility, pointer placement on a wrapped line, and
shrinking long content to one line.

The editor changes from 620 to 380 logical pixels wide, increasing shaped text
height from 276 to 483 pixels while retaining its model and selection. The caret
remains inside the 160-pixel editor viewport after the focused resize. Replacing
the text with `Short wrapped text` reduces shaped height to 23 pixels. Full models,
selection offsets and caret positions are recorded in `results.json` and
`wrapped-editor.log`; screenshots capture every stage.

Reproduce from the repository root:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build -p lumine-desktop --example wrapped_editor
python3 scripts/wrapped_editor_smoke.py /tmp/lumine-target/debug/examples/wrapped_editor --output docs/platform-validation/wrapped-editor
```

`metadata.json` records the final binary hash, compiler and commands.
`source.tar.gz` contains 114 source/build inputs; their manifest was rechecked
unchanged after building and running. Copies of both Python runner modules
preserve the smoke harness. This final run includes the preferred-column,
soft-line caret-affinity and shaping fixes and supersedes the earlier unarchived
run. It validates X11 behavior, not native macOS/Wayland input or GPU performance.
