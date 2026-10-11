# Native platform services evidence

Linux native integration, September 2026. Sources are snapshots of the owned desktop slice; the wider workspace was concurrently developing. `binaries.json` identifies the exact native executables used. `sources/` includes the desktop integration, examples, harnesses and lockfile. No macOS runtime claim is made: `macos-check.log` is an aarch64-apple-darwin cross-check only.

- `wayland`: actual owned Sway session, application DISPLAY unset. GTK XDG portal selected a fixture file and folder, selected a save destination without writing it, and canceled from the rendered File menu. Actual Zenity prompt returned Ok; an owned URI handler received exactly `lumine-smoke:accepted`.
- `close-owner`: with a real picker open, the native owner was closed while a keeper window retained the application. The bus recorded `org.freedesktop.portal.Request.Close`, both parent and picker disappeared, the application stayed running, and the closed owner's callback was suppressed.
- `escape-response`: separate GTK backend behavior probe. Escape emits portal Response code 2; the owned API reports `Err(BackendUnavailable)`. The Cancel button in `wayland` emits code 1 and returns `Ok(None)`. This is explicitly not a claim that Escape equals user-cancel on every backend.
- `window-x11` and `window-wayland`: fullscreen entry and size restoration, minimum-size enforcement after native configuration settles, custom titlebar pointer drag (actual changed compositor rectangle on Wayland), display enumeration, native Open/Activate D-Bus callbacks, and reopening after the last window closes. Wayland records no global position and rejects placement while accepting supported size changes.

Linux prompts require Zenity. The run used a private extracted Zenity/GTK runtime from `/tmp/lumine-native-services-tools/root`, with no system installation or user's desktop changes. Prompt backend is unparented on Linux; owner lifetime no longer retains the unused native parent. URI registration and private D-Bus sessions existed only for the run. See `docs/platform-services.md` for precise API and platform limits.

Commands (owned desktop setup/cleanup is inside each harness):

```sh
export LD_LIBRARY_PATH=/tmp/lumine-ci-loader-check/build/loader
/usr/bin/python3 scripts/native_services_smoke.py /tmp/lumine-target/debug/examples/platform_services --tools-root /tmp/lumine-native-services-tools/root --output /tmp/lumine-native-services-owned-portal-second
/usr/bin/python3 scripts/native_services_smoke.py /tmp/lumine-target/debug/examples/platform_services --close-picker --output /tmp/lumine-native-services-close-owner
/usr/bin/python3 scripts/native_services_smoke.py /tmp/lumine-target/debug/examples/platform_services --escape-cancel --output /tmp/lumine-native-services-escape-probe
/usr/bin/python3 scripts/native_platform_smoke.py /tmp/lumine-target/debug/examples/native_platform --output docs/platform-validation/native-services/window-x11
/usr/bin/python3 scripts/native_platform_smoke.py /tmp/lumine-target/debug/examples/native_platform --wayland --output docs/platform-validation/native-services/window-wayland
```

The initial Linux picker implementation used rfd's blocking worker. It was replaced after source inspection proved that rfd's async API also detaches a blocking thread: dropping that future cannot safely release raw parent handles. The owned implementation subscribes before creating a portal request, owns the request path/connection, sends Close on cancellation, and destroys its parent export before releasing the native window. Parent-export code carries the upstream MIT license. macOS still uses rfd's AppKit implementation.

Earlier failures remain in `/tmp/lumine-native-services-*` and `/tmp/lumine-native-platform-wayland`: initial automation focus timing, strict Escape result difference, and the prior delayed undersized Wayland request. The final minimum-size oracle checks the settled size, not merely an early native snapshot.
