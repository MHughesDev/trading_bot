---
name: tbot-howto
description: Use for any platform call. How to read data, submit and wait for jobs, fetch artifacts and read memory through the tbot CLI, and how to read a refusal.
---
# tbot how-to

`tbot` is your only route to the platform. Everything honest about research (the data cutoff,
trial counting, budgets) is enforced on the other side of it.

## Data

- `tbot data catalog` lists what you may see. `tbot data search <q>` ranks instruments.
- `tbot data bars <instrument> --start .. --end .. --tf 1m [--as-of ..] [--format parquet]`.
  With `--format parquet` you get an artifact handle, not rows.
- Also: `trades`, `open-interest`, `option-chain`, `fundamentals --cik`, `features --name`.
- `tbot data live <instrument>` works on the Desk project only.

## Jobs

- `tbot jobs submit <kind> '<json manifest>' [--experiment exp_..]`. If the reply says
  `already existed (no new trial counted)`, an identical job ran before and you spent nothing.
- `tbot jobs wait <job_id>...` prints one line per job and never loops. `409 budget_exhausted`
  on submit means the project has no compute budget left for that estimate.

## Artifacts and memory

- `tbot artifact get <handle>` shows the manifest. `tbot artifact download <handle> --out f`
  writes the bytes to disk. Analyse the file in code. Do not read a large extract into context.
- `tbot memory list findings [--subject X]` reads what the research loop recorded.
  `tbot memory prefer --scope global --statement ".."` records a stated user preference.

## Skills

- `tbot skills glossary` lists every skill you can see; `*` marks the ones mounted for this session.
- `tbot skills search "<what it is for>"` searches whole skill bodies and says whether it was lexical.
- `tbot skills propose <dir> ...` proposes one (see `skill-authoring`); `tbot skills proposals` shows
  what became of them.

## Knowledge

- `tbot knowledge move-check '{"move": "simplify"}'` before you spend a trial on a structural move:
  it warns, or blocks and cites the campaigns that failed.
- `tbot knowledge analogs --instrument-id N --venue-id N` finds past windows that looked like this
  one; the same episode is never returned.
- `tbot knowledge regime-profile <instrument> --asset-class <c>` and `regime-prior` say what
  regime a market is in and which families did best there, with their observation counts.
- `tbot knowledge compare-to-random <campaign>` ranks a campaign against random search with the
  same budget. `tbot knowledge source ...` records that a hypothesis came from a stored finding;
  a finding is never evidence for a new experiment.

## Reading a refusal

A refusal carries `error`, `message` and usually `fix`. Do what `fix` says. Do not retry the
same call: a refusal is a rule, not a hiccup.
