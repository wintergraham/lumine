# Failed initial current-build Wayland IME run

The basic composition test timed out waiting for `ni hao`. The client received
only preedit `n`. Fcitx received every injected key and sent successive `n`, `ni`,
`ni h`, `ni ha`, and `ni hao` preedits with input-method commit serial 2. The
client sent a cursor rectangle change and text-input commit after receiving `n`;
Fcitx received the next input-method `done` only after sending the queued updates.

This trace is consistent with stale input-method transactions during a backlog,
but does not isolate responsibility to lumine, winit, Fcitx or Sway. No renderer or
editor correction was made on the strength of this observation. Later paced
functional smoke tests do not resolve or disprove this failure. Rapid typing
under load remains a separate compatibility issue to investigate.

The source archive preserves the original burst-injection harness and matching
Rust sources; `check.log`, `native/application.log`, `native/protocol.log`, and
`native/fcitx.log` preserve the failure. The executable was freshly rebuilt from
unchanged Rust inputs; the build output is retained here.

The [input-method-v2 protocol definition](https://raw.githubusercontent.com/Smithay/wayland-rs/master/wayland-protocols-misc/protocols/input-method-unstable-v2.xml)
requires a commit serial matching the count of issued done events for state
replacement. This supports the backlog interpretation; the trace alone does not
establish a complete causal diagnosis.
