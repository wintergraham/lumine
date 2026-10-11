#!/usr/bin/env python3
"""Post-measurement visualization; does not participate in timed trials."""
from pathlib import Path
import json
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

out = Path(__file__).resolve().parent
records = json.loads((out / 'summary.json').read_text())['groups']
data = {(r['mode'], r['framework']): r for r in records}
modes = ['idle', 'stream', 'scroll', 'both']
colors = {'lumine': '#007f6e', 'gpui': '#4164b7', 'quickgui': '#b96a23'}
plt.rcParams.update({'font.family': 'DejaVu Sans', 'font.size': 11,
                     'axes.spines.top': False, 'axes.spines.right': False,
                     'svg.fonttype': 'none'})
fig, axes = plt.subplots(1, 2, figsize=(12.8, 5.2))
for ax, metric, divisor, label in zip(axes, ['cpu_percent_one_core', 'mean_rss_bytes'],
                                     [1., 1048576.], ['CPU (% of one logical core)', 'Mean process RSS (MiB)']):
    for i, framework in enumerate(colors):
        values = [data[mode, framework][metric] for mode in modes]
        medians = [v['median'] / divisor for v in values]
        positions = [x + (i - 1) * .25 for x in range(len(modes))]
        errors = [[(v['median'] - v['min']) / divisor for v in values],
                  [(v['max'] - v['median']) / divisor for v in values]]
        ax.bar(positions, medians, width=.23, color=colors[framework], label=framework,
               yerr=errors, capsize=3, error_kw={'linewidth': 1.1})
    ax.set_xticks(range(len(modes)), ['Idle', 'Streaming', 'Scrolling', 'Both'])
    ax.set_ylabel(label)
    ax.set_ylim(bottom=0)
    ax.grid(axis='y', color='#dedede', linewidth=.7)
    ax.set_axisbelow(True)
axes[0].legend(frameon=False, loc='upper left')
fig.suptitle('Matched streaming text and virtualized-list UI', fontsize=17, y=.98)
fig.text(.5, .90, 'Linux / Mesa llvmpipe · LP_NUM_THREADS=4 per pool · 3 repeats per mode', ha='center', fontsize=11)
fig.text(.5, .035, 'Bars: medians. Whiskers: observed min–max, not confidence intervals.\n'
         '15 seconds sampled after 5 seconds warmup; shared host. Logical updates ≠ presented frames.',
         ha='center', fontsize=10, color='#444444')
fig.subplots_adjust(left=.075, right=.98, top=.84, bottom=.20, wspace=.25)
fig.savefig(out / 'comparison.svg')
fig.savefig(out / 'comparison.png', dpi=160)
(out / 'plot-environment.json').write_text(json.dumps({'matplotlib': matplotlib.__version__,
    'source': 'summary.json', 'generated_after_measurement': True}, indent=2) + '\n')
