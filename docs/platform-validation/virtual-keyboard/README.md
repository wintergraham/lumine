Native virtual-list keyboard validation
=======================================

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 cargo build -p lumine-desktop --example virtual_keyboard
python3 scripts/virtual_keyboard_smoke.py /tmp/lumine-target/debug/examples/virtual_keyboard --output docs/platform-validation/virtual-keyboard
```

The owned Xvfb/Openbox session navigates one million 32px rows inside a 160px
content viewport. A native Tab enters the list and subsequent native keys produce:

| Key | Accessible position (one-based) | Offset | Live rows | Total rows built |
| --- | ---: | ---: | ---: | ---: |
| End | 1,000,000 | 31,999,840 | 6 | 12 |
| PageUp | 999,995 | 31,999,808 | 7 | 13 |
| Home | 1 | 0 | 6 | 19 |
| ArrowDown | 2 | 0 | 6 | 19 |

Read-only `r` reports query the keyboard target's accessible position and RAII
row resource counters. No application code mounts target rows or moves offsets.
Every screenshot checks a complete 32px focus-colored row and its 2px accent
marker at the appropriate
visible location. Stage 1, showing the millionth row, was visually inspected.
Rows use transparent content so the framework's wrapper focus cue is visible.

`virtual_keyboard.log`, four screenshots, and `result.json` retain evidence. All
owned processes are cleaned up. This validates Linux X11 software rendering;
it does not establish native macOS, hardware GPU performance, or screen-reader
operation. Interactive descendants and keyboard exclusion are covered separately
by framework tests, not this example.
