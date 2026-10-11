# Zero-delay burst reproduction

Two final-preedit delivery failures were recorded: the warm burst in both real lumine and a standalone winit/softbuffer application remained at `n`, while the input method had emitted `ni hao`. Server traces show the final transaction rejected under a stale serial. Both cold bursts delivered fully; all four subsequent Unicode commits succeeded.

See [the unsent upstream report](upstream-report.md) for exact versions, commands, server timestamps/line references, source mechanism, and limitations. `summary.json` retains the four outcomes; original logs and `result.json` files preserve failures explicitly. Source/build provenance is linked to the earlier blocking and real-lumine archives, with matching binary hashes. No production changes were made.
