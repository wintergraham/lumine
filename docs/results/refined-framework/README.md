# Incomplete framework comparison — not performance evidence

This attempted refresh stopped after **17 of 36 trials**. The next trial,
lumine streaming repeat 1, exited with code 101 after 300 samples; its log is empty.
The sampler then discarded that failed trial's raw samples, so only the 17
completed trials and their 5,092 samples survive. No framework ranking or
performance conclusion should be drawn from this series.

The shared root filesystem was subsequently observed with zero available bytes.
A screenshot attempt also exited unsuccessfully with an empty log, and agent
creation explicitly failed with `No space left on device`. Removing regenerable
Cargo download archives recovered about 299 MiB. This is consistent with a
storage-related failure, but the empty lumine log does not establish its cause.
The shared host also had substantial unrelated activity. Completed active trials
reported **27.0–59.95 model updates per requested second**, failing the intended
58–61 update-rate acceptance range even before the process failure.

The planned protocol was three frameworks, four modes, three rotated repeats,
20 seconds of workload per process, excluding the first five seconds after
spawn. All processes used an owned Xvfb/Openbox desktop, llvmpipe Vulkan and the
same private patched Vulkan loader. The frozen lumine release includes the current
public component, styling, editor and renderer implementation. The workload
contains streaming labels and virtual rows; it does not exercise editor input,
accessibility clients, blur or transparent windows. GPUI 0.2.2 and QuickGUI
revision `811d6e2816d5229711f59683c4c9dfbb6fc74133` retain the reference build's
verified source and executable hashes.

After disk-space recovery, two isolated 20-second lumine streaming diagnostics
exited successfully with 1,197 and 1,199 updates; see [diagnostic results](diagnostic.json).
They did not reproduce the failure and are not comparative performance samples.
Separate three-second seeded screenshot runs exited
successfully for all three frozen binaries. Visual inspection confirms matching
960×720 geometry, content and rows, with font baseline/rasterization differences:
[lumine](lumine.png), [GPUI](gpui.png), [QuickGUI](quickgui.png).
The [capture audit](capture-audit.json) verifies executable hashes and the private
loader in all three capture-time process maps. QuickGUI also maps EGL; lumine and
GPUI do not. These captures establish visible output, not presentation cadence.

[Run log](run.log), [incomplete CSV](current.csv), per-trial logs/raw JSON,
[preflight](preflight.json), [lumine build proof](lumine-build-manifest.json),
[reference build proof](reference-build-manifest.json), and
[source manifest](source.json)/[archive](source.tar.gz) preserve this attempt.
All **214 archived inputs** were hash-verified. Subsequent harness changes are
intentionally separate from the archived sampler used here. The runner refuses
to overwrite this CSV; a retry must use a new output directory and matching
build/source proof. The [earlier complete series](../current-framework/README.md)
remains historical evidence with its stated limitations.
