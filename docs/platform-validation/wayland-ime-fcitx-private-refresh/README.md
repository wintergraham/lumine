# Private Fcitx nonempty-preedit refresh experiment

A private Fcitx5 5.1.19 plugin patch restored final preedit in all four zero-delay observations (lumine/standalone, cold/warm), and the original eight-stage Escape cancellation/focus-switch smoke passed. No lumine production code, system package, installed plugin, or global service was changed. This is an experimentally verified **upstream patch candidate**, not an installed framework fix or a complete input-method reliability guarantee.

## Evidence of recovery

The lumine warm run reproduced the stale transaction ordering before recovering. In `native/lumine-delay0-1/sway.log`:

| Event | Timestamp (ms) | Line |
| --- | --- | --- |
| `n`, matching `commit(6)` | 3633923.571 | 3231 |
| Server issues seventh input-method `done()` | 3633929.288 | 3269 |
| Final `ni hao`, stale `commit(6)` | 3633935.918 | 3297 |
| Patch refreshes `ni hao`, matching `commit(7)` | 3633935.940 | 3300 |
| Subsequent deduplicated-geometry settlement refresh `commit(8)` | 3633942.253 | 3329 |
| Space commits `你好` once with `commit(8)` | 3635923.746 | 3340 |

The client/application preedits were `n`, `ni hao`, `ni hao`. All four original `ni` through `ni hao` transactions used a stale serial and were rejected, so this is direct evidence of recovery, not merely another run where the race did not occur. `server-serial-analysis.json` records those four mismatches. The standalone run had no stale transaction in this sample; its success alone does not independently establish recovery.

Duplicate final preedit refreshes were observed and stopped once cursor geometry settled; no runaway event cycle occurred in these runs. Every burst committed exactly one `你好`. The separate original failure archive remains unchanged at `../wayland-ime-zero-delay`.

## Patch scope

`sources/preedit-refresh.patch` changes only the Wayland input-method-v2 frontend's ordinary `done` callback. It increments the existing serial, records whether this batch activates/deactivates an input context, performs existing lifecycle handling, then republishes the current delegated input context's nonempty preedit when focus remains real and this batch is not a lifecycle transition. It uses the existing output filtering/formatting path at the new serial. It never calls commit-string or deletion replay.

The test deliberately does **not** claim recovery for stale empty-preedit cancellation, stale committed text, or deletion transactions. It does not prove safety across every external surrounding-text change, input method, virtual input-context transition, or formatter side effect. An upstream-quality patch needs broader lifecycle/state tests and a decision about duplicate refresh suppression; this minimal candidate is not proposed for system deployment here.

`cancellation-focus/result.json` records the eight-stage native smoke: Pinyin preedit and commit, Escape cancellation without model mutation, another composition cancelled by focus switching, then a fresh second-editor composition and commit. Final models remain first=`你好`, second=`世界`, with exactly two commits. These checks establish those tested guards only; they do not extend the patch's scope to every cancellation race.

## Isolation, build, and provenance

The exact tagged source archive, exact Yoga submodule archive/commit, original and patched translation units, and patch are under `sources/`. The original translation unit's SHA256 matches the prior source inspection. `download-provenance.json` records source locations and downloaded .deb hashes. The missing CMake/ECM/gettext/JSON tools and small library dependencies were downloaded and extracted privately; no packages were installed. Build workspace size after completion was about 156 MB. `build-commands.txt`, configure/build logs, CMake cache, and compiler/tool versions capture the build. Initial configuration failed only because GitHub's source tar omits the Yoga submodule; that failed log is retained, followed by successful exact-submodule configuration/build.

Only `libwaylandim.so` was copied into `/tmp/lumine-fcitx-preedit/plugin`; the harness sets `FCITX_ADDON_DIRS` for its owned Fcitx process. Each run's `fcitx-maps.txt` verifies that private plugin was loaded while Fcitx Core/Utils/Config remained the installed system libraries; none came from the private build directory. The built plugin is archived here, and `binary-hashes.json` records it, the untouched installed plugin/executable, and both application binaries. The plugin retains a build-directory RUNPATH; it is an experiment artifact, not a relocatable distribution package.

Client application binaries match the preceding unpatched experiment builds. `sources/*refresh.py` contain exact harness copies, with private-plugin selection and mapping capture; the rapid-input cases also retain automatic script/binary provenance. Full engine/client/server logs, per-trial slices, screenshots, and private configurations are retained. `artifact-manifest.json` binds all archive files.

Run commands (from repository root):

```
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-fcitx-private-refresh/sources/native_refresh.py /tmp/lumine-target/debug/examples/native_ime --sessions 1 --bursts 2 --key-delay-ms 0 --addon-dir /tmp/lumine-fcitx-preedit/plugin --output docs/platform-validation/wayland-ime-fcitx-private-refresh/native
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-fcitx-private-refresh/sources/standalone_refresh.py /tmp/lumine-target/debug/lumine-ime-blocking-probe --sessions 1 --bursts 2 --key-delay-ms 0 --present-on-ime --addon-dir /tmp/lumine-fcitx-preedit/plugin --output docs/platform-validation/wayland-ime-fcitx-private-refresh/standalone
LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader /usr/bin/python3 docs/platform-validation/wayland-ime-fcitx-private-refresh/sources/cancellation_focus_refresh.py /tmp/lumine-target/debug/examples/native_ime --addon-dir /tmp/lumine-fcitx-preedit/plugin --output docs/platform-validation/wayland-ime-fcitx-private-refresh/cancellation-focus
```
