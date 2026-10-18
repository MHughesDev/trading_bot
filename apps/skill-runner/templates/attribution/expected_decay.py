"""What haircut to put on a backtested edge before anyone sizes it.

```json
{
  "name": "expected_decay",
  "version": 1,
  "namespace": "attribution",
  "description": "A prior for how much of an in-sample edge survives out of sample, keyed by whether the idea is published. Cite it in a dossier next to the measured result, never in place of it.",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "source": {
        "type": "string",
        "enum": ["published", "in_house", "unknown"],
        "default": "unknown",
        "description": "where the idea came from; a published anomaly starts with the larger haircut"
      },
      "archetype": {"type": "string", "description": "a label carried through to the output, for the dossier"},
      "in_sample_metric": {"type": "number", "description": "an in-sample figure (for example a Sharpe) to apply the haircut to"}
    }
  },
  "summary_template": "{source} idea: expect to keep about {kept_pct}% of the in-sample edge ({applied_haircut_pct}% haircut, a prior); {expected_live}",
  "detail_fields": [
    ["source", ""],
    ["archetype", ""],
    ["applied_haircut_pct", "pct"],
    ["kept_pct", "pct"],
    ["oos_haircut_pct", "pct"],
    ["post_publication_haircut_pct", "pct"],
    ["expected_live", ""]
  ],
  "wall_seconds": 10
}
```

A prior, and where it comes from
--------------------------------
McLean and Pontiff (2016) studied 97 published return predictors in US equities and
found that their returns were about **26% lower out of sample** than in the original
sample, and about **58% lower after publication**. The gap between the two is the
part attributable to arbitrageurs reading the paper; the first is what remains after
accounting for the fact that the in-sample figure was selected for being good.

So the tool gives two numbers and applies one:

* `published`: the post-publication figure (58%), because a published idea has been
  both selected and read.
* `in_house` and `unknown`: the out-of-sample figure (26%), because even an idea nobody
  published was selected for looking good on the data that found it.

What it does not know
---------------------
Both figures are estimated on US equity anomalies over decades. Nothing here is
estimated on crypto, on intraday horizons or on this platform's own experiments, and
the tool says so in every result rather than leaving the figure to be read as
authoritative. It is a **prior**: a dossier states it beside the measured
out-of-sample and forward results, and replaces it with them as they arrive. Applying
it to an in-sample metric is arithmetic, not forecasting, and the output names it as
such.
"""

OOS = 26.0
POST_PUBLICATION = 58.0
CITATION = "McLean and Pontiff (2016), Journal of Finance 71(1): 5-32"


def run(payload: dict) -> dict:
    source = str(payload.get("source") or "unknown")
    if source not in ("published", "in_house", "unknown"):
        return refuse(f"`source` is published, in_house or unknown, not {source!r}")
    haircut = POST_PUBLICATION if source == "published" else OOS
    kept = 100.0 - haircut
    metric = payload.get("in_sample_metric")
    if metric is None:
        expected_live = "no in-sample figure was given to apply it to"
        applied = None
    else:
        applied = float(metric) * kept / 100.0
        expected_live = f"an in-sample {float(metric):.3g} would become about {applied:.3g} (arithmetic, not a forecast)"
    detail = {
        "source": source,
        "archetype": payload.get("archetype"),
        "applied_haircut_pct": haircut,
        "kept_pct": kept,
        "oos_haircut_pct": OOS,
        "post_publication_haircut_pct": POST_PUBLICATION,
        "in_sample_metric": None if metric is None else float(metric),
        "metric_after_haircut": applied,
        "expected_live": expected_live,
        "citation": CITATION,
        "warnings": [
            "estimated on US equity anomalies; nothing here is estimated on crypto, on intraday "
            "horizons or on this platform's own experiments",
            "a prior: state it beside the measured out-of-sample and forward results and let those replace it",
        ],
    }
    return result(detail)
