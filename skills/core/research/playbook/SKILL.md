---
name: playbook
description: Use at the start of any research task. The order of work that keeps results honest: state the hypothesis first, look at data second, spend trials deliberately, report nulls as nulls.
---
# Playbook

1. **State the hypothesis before you look.** Write the claim, the instrument, the window and
   what would falsify it. Exploration is logged. Looking first and hypothesising after is
   visible in the record.
2. **Check the data.** Read `manifest.qc_grade` and `manifest.cutoff_applied`. Run `data_qc`
   if the series is ungraded.
3. **Pin the data.** Record `manifest.data_snapshot_id` and use `--as-of` so a rerun reads the
   same rows.
4. **Spend trials deliberately.** Every Experiment-kind job counts as a trial and raises the
   bar for significance. Do not sweep without a reason.
5. **Never touch the holdout.** The cutoff is the boundary. There is no parameter that moves
   it, and moving it after a result is how a researcher fools themselves.
6. **Report the null.** "No edge found" is a valid, fileable result. Say how many trials it
   cost and what data it covered.
7. **Cost matters.** A strategy that wins before costs and loses after them did not win.
