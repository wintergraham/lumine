# Native inherited line height

The owned Xvfb/Openbox run drives real pointer controls and keyboard selection in
`examples/line_height.rs`. Four stages switch inherited spacing from 24px to 42px,
then 12px and normal. The editor keeps the same text and selection at byte 11;
its caret height and second-row position follow each line pitch exactly. Normal
spacing at 20px font size is 28px. Tight spacing may overlap glyph rows and clips
ink at the outer text rectangle, matching the existing renderer contract.

`results.json`, the model log and four screenshots capture every stage. The
source archive and manifest were captured before the final build and smoke;
`metadata.json` verifies unchanged inputs and records binary/archive hashes,
compiler and commands. All owned test processes are cleaned up on success/failure.
This is native X11 software-GPU evidence, not macOS or hardware validation.

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo build -p lumine-desktop --example line_height --locked
python3 scripts/line_height_smoke.py /tmp/lumine-target/debug/examples/line_height --output docs/platform-validation/line-height
```

The adjacent consolidated logs record 378 passing tests, 26 doctests, strict
workspace Clippy, formatting and a macOS ARM64 cross-check. GPU pixel tests also
compare incremental/full repaint and verify line-height changes reuse glyphs.
