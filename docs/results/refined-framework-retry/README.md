# Completed retry — rejected for unequal delivered work

All **36 GUI processes** exited successfully, and the integrity audit recomputed
**10,793 raw samples** and verified **220 archived inputs**. However, the series
**failed its model-update acceptance check**: lumine's second scrolling trial
reported **1,099 updates in 20 seconds (54.95/s)**, below the required 58–61/s.
The other active trials stayed within that range. This series must not be used
to claim a CPU or memory advantage between frameworks.

That under-delivering trial measured 148.69% CPU relative to one core. Less
delivered work can lower CPU consumption, so treating that value as a performance
improvement would be misleading. The retained [summary](summary.json) describes
the recorded measurements; it is not an accepted matched-work ranking.
[Acceptance metadata](acceptance.json) identifies the exact rejected trial.

The planned protocol was three frameworks, four modes, three rotated repeats,
20 seconds of model activity per process, excluding the first five seconds after
spawn. All applications used an owned Xvfb/Openbox display, the explicit Mesa
llvmpipe Vulkan ICD and the same private patched loader. The retry initially
refused to start while unrelated Rust builds were active. It started once they
finished, but unrelated builds restarted during sampling; [host observations](host-observations.jsonl)
record this. lumine team builds and tests remained paused. Shared-host activity
is a plausible contributor, not an experimentally isolated cause of the missed
updates. There was no CPU pinning, thermal isolation or exclusive host control.

The latest verification build produced the same lumine release workload executable
as the [failed first attempt](../refined-framework/README.md). Later changes added
native diagnostics and harnesses, without changing this workload's behavior.
GPUI 0.2.2 and QuickGUI revision `811d6e2816d5229711f59683c4c9dfbb6fc74133`
retain the verified reference source and executable hashes. The public component,
provider, slot, fluent-style and virtual-list application remains functionally
matched to those reference applications. See the [source audit](../component-comparison-audit.md).

Separate seeded screenshots establish visible matched content and geometry:
[lumine](lumine.png), [GPUI](gpui.png), [QuickGUI](quickgui.png). Font baselines and
rasterization differ. The [capture audit](capture-audit.json) verifies frozen
binary hashes and capture-time loader mappings. These screenshots do not prove
presentation cadence. Model updates are not displayed-frame timestamps.

[CSV](current.csv), [integrity audit](audit.json), [run log](run.log),
[sampler metadata](current.csv.metadata.json), [preflight](preflight.json),
[lumine build proof](lumine-build-manifest.json), [reference build proof](reference-build-manifest.json),
[source manifest](source.json) and [archive](source.tar.gz) preserve this complete
but unaccepted attempt. No trials were replaced or combined with another run.
The runner exits unsuccessfully when update parity fails and refuses to overwrite
the CSV. Its improved sampler also preserves raw samples on process failure.

The [earlier complete comparison](../current-framework/README.md) remains
historical evidence with its original software-rendering and shared-host
limitations. A new clean comparison needs adequate host isolation and equivalent
delivered work; repeated retries on this busy host do not establish that.

To recheck artifact integrity independently of later workspace edits:

```sh
python3 scripts/audit_comparison.py docs/results/refined-framework-retry/current.csv \
  --source-manifest docs/results/refined-framework-retry/source.json \
  --source-archive docs/results/refined-framework-retry/source.tar.gz \
  --output /tmp/refined-framework-retry-audit.json
```

The integrity command records `near_60hz_workload_updates: false`; it does not
turn the failed acceptance decision into a valid performance comparison.
