# Independent Vulkan device lifecycle stress

A targeted test reproduced a SIGSEGV with the system Vulkan loader 1.4.341.0-1.
The same binary passed three runs with privately built loader 1.4.345, with all
other environment settings unchanged. Each run starts 24 worker threads and
creates, renders, reads back, verifies, and destroys 10 independent GPU contexts
per worker: 240 lifecycles per run, 720 successful fixed-loader lifecycles total.
These are independent contexts, not the normal shared-device multiwindow path.

The fault stack enters `libvulkan.so.1` from
`vkSetDebugUtilsObjectNameEXT`, while wgpu creates its indirect-draw validation
pipeline during device creation. This matches the parallel wgpu crash reported in
[Vulkan-Loader issue 1863](https://github.com/KhronosGroup/Vulkan-Loader/issues/1863).
[PR 1866](https://github.com/KhronosGroup/Vulkan-Loader/pull/1866) synchronizes the
loader's global instance/device lists; loader 1.4.345 includes that fix.
The stack match and controlled loader replacement strongly support this upstream
race as the reproduced failure's cause. The earlier unsymbolized workspace
failure cannot be proven identical, and finite passing stress runs do not prove
all renderer or driver concurrency correct.

A separate diagnostic run with only `WGPU_DEBUG=0` also passed on the old loader.
That is an isolation experiment, not a product workaround: lumine still honors wgpu
configuration and retains debug-build defaults. The three fixed-loader runs did
not disable debug labels or validation flags. The Khronos Vulkan validation layer
was not installed in this environment; wgpu internal validation remained enabled.
The driver was Mesa llvmpipe 26.0.8. The upstream report involved hardware RADV,
so this loader issue should not be described as inherently software-GPU-specific.

## Reproduction

The opt-in test is ignored by ordinary workspace tests, which retain bounded
four-device concurrency coverage and serialize other GPU fixture lifetimes:

```sh
CARGO_TARGET_DIR=/tmp/lumine-target CARGO_PROFILE_DEV_DEBUG=0 \
  cargo test -p lumine-gpu --test pixels \
  -- --ignored --exact independent_device_lifecycle_stress --nocapture
```

`LUMINE_GPU_STRESS_WORKERS` accepts 1–64 (default 24), and
`LUMINE_GPU_STRESS_ITERATIONS` accepts 1–100 (default 10). Independent device creation
can consume substantial driver resources. Use a Vulkan loader containing PR 1866
(for example 1.4.345 or a distribution backport) for concurrent-device stress.
No system library was replaced for this investigation.

The controlled fixed-loader runs used the already-built binary recorded in
`metadata.json`, not a rebuild:

```sh
LD_LIBRARY_PATH=/tmp/lumine-vulkan-loader-1.4.345/build/loader \
  gdb -q -batch \
  -ex 'set print thread-events off' \
  -ex 'catch load libvulkan[.]so' -ex run \
  -ex 'info sharedlibrary libvulkan' -ex 'disable 1' \
  -ex continue -ex 'bt 30' \
  --args /tmp/lumine-target/debug/deps/pixels-6d784e015f9db18b \
  --ignored --exact independent_device_lifecycle_stress --nocapture
```

Each fixed-loader log confirms the private library path, a passing test, and a
normally exited inferior. GDB itself exits 1 because the final `bt` has no stack
after normal exit; the inferior result, not GDB's status alone, classifies the run.
Conversely, GDB exited 0 after capturing the old-loader SIGSEGV.

`metadata.json` records the test binary and source archive hashes, baseline
environment, packages, and fixed-loader intervention. `source.tar.gz` freezes the
stress fixture and workspace source at binary build time. `build-provenance.json`,
`build-commands.sh`, and `build.log` record the private loader build; its builder's
`tests_run: false` means the builder did not execute tests, before these probes.
The system-loader crash, debug-off ablation, and all three fixed-loader logs are
retained here. Native macOS and hardware-GPU validation remain separate open work.

The Linux GPU CI job now builds these pinned loader/header commits privately with
[`scripts/build_vulkan_loader.sh`](../../../scripts/build_vulkan_loader.sh) and
sets its library directory for that job. The script was exercised locally in a
fresh directory, followed by the declarative portal GPU readback test using the
resulting loader. This does not establish that remote CI has run. The script
requires a new destination directory and never installs into the system prefix.
