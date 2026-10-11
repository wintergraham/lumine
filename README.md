# Lumine

Lumine is a retained native UI engine written in Rust. It combines reactive components, GPU rendering, native desktop integration and a built-in motion system for Linux, macOS and Windows.

Lumine is an independent fork of [zgui](https://github.com/senxd/zgui) and powers the native interface in [Ask](https://thisisask.com). Ask is its first application. The engine has its own workspace, examples and validation tools, and can run independently of Ask.

Components mount once. Signals update their retained targets, while the engine tracks layout, paint and composition work separately. Rendering resources are bounded, and idle motion schedules no frames.

## What is included

- Reactive signals, batching, scoped state and services, keyed views, ownership and cancellation.
- Flex and grid layout, percentage sizing, absolute positioning, clipping and virtual lists.
- A retained wgpu renderer with damage tracking, shaped text, images, vector paths, borders, shadows, transforms, shaders and backdrop blur.
- Native input, pointer capture, focus navigation, text editing, IME, clipboard, undo/redo and accessibility.
- Desktop windows, display scaling, menus, dialogs, file drops and platform services.
- Controls including buttons, fields, checkboxes, sliders, scroll views, popovers and menus.

## Built-in motion

Motion runs against the same retained scene and reactive state as the rest of the UI. It includes:

- Tweens, custom Bézier easing and interruptible springs that preserve velocity when retargeted.
- Typed keyframes, coordinated timelines, playback controls and grouped completion.
- Presence and exit transitions that retain a view until its animation finishes.
- Layout projection and shared layout IDs for animated resizing, reordering and remounting.
- Drag/snap interactions, measured scroll progress and named motion states.
- Reduced-motion policy, visibility-aware effect scheduling and an on-demand inspector.

Active motion values share a scheduler and batch their updates. Translation, scale, rotation and opacity use paint transforms; animating width or height still requires layout work. Continuous decorative effects have a separate scheduler with a configurable budget.

See the [motion guide](docs/motion.md) and [motion measurements](docs/motion-performance.md).

## Run an example

Clone the repository and run the component example:

```sh
git clone https://github.com/wintergraham/lumine.git
cd lumine
cargo run --locked -p lumine-desktop --example components
```

Explore motion, rendering effects and the control gallery:

```sh
cargo run --locked -p lumine-desktop --example motion
cargo run --locked -p lumine-desktop --example effects
cargo run --locked -p lumine-desktop --example gallery
```

More examples live in [crates/lumine-desktop/examples](crates/lumine-desktop/examples), including editors, layout, menus, multiple windows and platform services.

### Platform prerequisites

- **macOS:** Xcode command-line tools and the system frameworks.
- **Windows:** the stable MSVC Rust toolchain and Visual Studio Build Tools with the C++ workload and Windows SDK. See [Windows support](docs/windows.md).
- **Linux:** fontconfig, xkbcommon and D-Bus development libraries, plus `pkg-config`. On Debian/Ubuntu, install `libfontconfig1-dev`, `libxkbcommon-dev`, `libdbus-1-dev` and `pkg-config`. File dialogs use the desktop's XDG portal; prompts use `zenity`.

Use a Rust toolchain that supports edition 2024. Native examples require a desktop session and a supported graphics adapter.

## A small application

```rust,no_run
use lumine::compose::prelude::*;
use lumine_desktop::Application;

fn counter() -> View {
    component(|cx| {
        let count = cx.state(0_u32);
        let label = count.clone();
        column().p(24.0).gap(12.0)
            .bg(rgb(0x172030)).rounded(12.0)
            .child(text_signal(move || format!("Count: {}", label.get())))
            .child(button()
                .px(16.0).py(10.0).rounded(8.0)
                .bg(rgb(0x345888))
                .hover(|s| s.bg(rgb(0x426da4)))
                .focus(|s| s.border(2.0).border_color(rgb(0x9ac7ff)))
                .child("Increment")
                .on_click(move || { count.update(|n| *n += 1); }))
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new().run(|cx| { cx.render(counter()); })
}
```

Read the [application guide](docs/application.md), [component model](docs/composition.md) and [styling guide](docs/styling.md) for state, services, children and ownership.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`lumine`](crates/lumine) | Reactive runtime, retained scene, layout, components, controls, input and motion. |
| [`lumine-gpu`](crates/lumine-gpu) | GPU rendering, text shaping, images, shaders and rendering diagnostics. |
| [`lumine-desktop`](crates/lumine-desktop) | Native windows, presentation, platform services and desktop examples. |
| [`lumine-workload`](comparisons/workload) | Shared deterministic workloads for framework comparisons. |

The workspace also contains dependency patches in `vendor`, benchmark adapters in `comparisons`, and validation scripts in `scripts`.

## Relationship to Ask

Lumine owns reusable rendering, input, layout and motion behavior. Ask owns its screens, visual theme, browser workflows and agent behavior, and consumes a pinned Lumine revision.

Electron-related changes will be developed and tested in Ask before reusable parts move into Lumine. This repository currently provides the Rust engine and native desktop host. Electron and native React integrations are future work for Lumine.

## Documentation and status

Lumine is under active development. Platform support and individual capabilities have different levels of validation; a successful build does not establish full native compatibility.

- [Architecture](docs/architecture.md), [layout](docs/layout.md) and [application development](docs/application.md).
- [Text](docs/rich-text.md), [paint styles](docs/paint-styles.md), [canvas](docs/canvas.md), [images and shaders](docs/images-and-animation.md).
- [Native services](docs/platform-services.md), [macOS surfaces](docs/native-surfaces.md) and [Windows support](docs/windows.md).
- [Capability matrix](docs/gpui-parity.md), [validation status](docs/validation.md) and [remaining work](docs/ROADMAP.md).

Known limitations include incomplete macOS native-interaction qualification and stale IME preedit under very rapid input on the tested Sway/Fcitx Wayland stack. Core-only text measurement uses an approximate fallback until a native text measurer is installed. See the linked validation records for the affected configurations.

## Performance and validation

The repository retains measured comparisons against GPUI and QuickGUI, including streaming text, virtual lists and a dashboard workload. These records were carried over from the fork before the Lumine rename. Results apply to their recorded machines, workloads and revisions.

Start with the [benchmark methodology](docs/benchmarking.md), [result inventory](docs/results/README.md), [macOS dashboard results](docs/results/latest-macos-heavy-2026-09-27-92fd6a4/README.md) and [Linux comparison](docs/results/gpui-parity-workers4/README.md). CPU work, GPU work, presented frames and memory usage are separate measurements.

The workspace's full validation commands are:

```sh
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

GPU tests require a Vulkan or Metal adapter. Native interaction checks require the corresponding desktop environment. Existing [Linux evidence](docs/platform-validation/gpui-parity-integration/README.md) and [macOS evidence](docs/platform-validation/macos-continuation/README.md) document both results and remaining gaps.

## Licensing and provenance

Lumine has no repository-wide license for fork-specific additions or modifications. The inherited zgui MIT copyright and permission notice is preserved verbatim in [third-party/zgui-LICENSE.txt](third-party/zgui-LICENSE.txt). Dependency and font notices remain alongside their sources and assets.

See [third-party notices](third-party/README.md) for the imported revision and retained notices.
