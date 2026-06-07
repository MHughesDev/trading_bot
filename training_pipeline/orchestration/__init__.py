"""Training-only orchestration: nightly campaign, walk-forward specs, promotion bookkeeping.

Moved out of ``orchestration/`` (which retains operational, runtime-reachable modules such as
``real_data_bars`` and the per-asset ``init_*`` pipeline). Not started by any runtime process.
"""
