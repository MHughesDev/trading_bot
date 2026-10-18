"""Is a filter caused by both the signal and the future return, so conditioning on it manufactures an edge.

```json
{
  "name": "collider_check",
  "version": 1,
  "namespace": "audit",
  "description": "Structural check of each filter's declared dependencies: flags one that depends on both a signal quantity and a forward-looking one. Reads declarations; never proves a collider.",
  "input_schema": {
    "type": "object",
    "required": ["filters", "signal_quantities", "forward_quantities"],
    "additionalProperties": false,
    "properties": {
      "filters": {
        "type": "array",
        "minItems": 1,
        "maxItems": 64,
        "description": "conditions applied besides the signal",
        "items": {
          "type": "object",
          "required": ["name", "depends_on"],
          "additionalProperties": false,
          "properties": {
            "name": {"type": "string"},
            "depends_on": {
              "type": "array",
              "items": {"type": "string"},
              "minItems": 1,
              "maxItems": 64,
              "description": "what it is computed from"
            },
            "causal_role": {
              "type": "string",
              "enum": ["confounder", "collider", "mediator", "instrument", "unknown"],
              "default": "unknown",
              "description": "asserted role"
            }
          }
        }
      },
      "signal_quantities": {
        "type": "array",
        "items": {"type": "string"},
        "maxItems": 256,
        "description": "the signal's outputs and inputs"
      },
      "forward_quantities": {
        "type": "array",
        "items": {"type": "string"},
        "maxItems": 256,
        "description": "anything known only after the decision"
      }
    }
  },
  "summary_template": "{n_filters} filters checked structurally; {n_flagged} flagged ({flagged_names}); {check_type} on declared dependencies, not a proof",
  "detail_fields": [
    ["n_filters", ""],
    ["n_flagged", ""],
    ["flagged_names", ""],
    ["check_type", ""],
    ["verdict", ""]
  ],
  "wall_seconds": 10
}
```

The mistake this is for
-----------------------
Conditioning on a variable that is an effect of both the signal and the future return opens a
path between them that did not exist. Select only the bars where "volatility after entry was
moderate" and a signal with no edge will appear to predict returns, because volatility after
entry is caused by what the return later does and by what the position was. The result looks like
alpha and is produced by the filter.

What the check does
-------------------
You declare, for each filter, the quantities it is computed from and (optionally) what you
believe its causal role is, and you declare which quantities belong to the signal and which are
forward-looking. The tool intersects them:

* depends on **both** a signal quantity and a forward-looking one: `possible_collider`, flagged;
* declared a `collider`: flagged, whatever it depends on;
* depends on a forward-looking quantity only: `forward_looking`, flagged as a different fault
  (it uses information from after the decision), not as a collider;
* declared a `confounder`: **not flagged** as a collider, and the output records that the
  declaration was taken as given, because a confounder is exactly the kind of variable one
  conditions on on purpose;
* anything else: not flagged.

What it does not do
-------------------
It is a **structural** check on what you declared. It cannot find a dependency you did not name,
cannot tell a confounder from a collider (that is a claim about causation the data do not settle),
and never claims to *prove* a collider. A flag is a reason to ask whether the filter can be
computed without the forward quantity or removed; no flag is not a clearance.
"""


def run(payload: dict) -> dict:
    filters = payload.get("filters") or []
    signal = {str(x) for x in (payload.get("signal_quantities") or [])}
    forward = {str(x) for x in (payload.get("forward_quantities") or [])}
    if not filters:
        return refuse("no filters were given; there is nothing to check")
    if not signal and not forward:
        return refuse(
            "neither signal_quantities nor forward_quantities has a name in it, so no dependency can "
            "be matched; a structural check needs both declared"
        )
    overlap = sorted(signal & forward)
    rows = []
    flagged = []
    for f in filters:
        name = str(f.get("name"))
        deps = [str(d) for d in (f.get("depends_on") or [])]
        if not deps:
            return refuse(f"filter {name!r} declares no dependencies; a filter is computed from something")
        role = str(f.get("causal_role") or "unknown")
        on_signal = sorted(set(deps) & signal)
        on_forward = sorted(set(deps) & forward)
        if role == "collider":
            verdict, is_flag = "declared_collider", True
            note = "declared a collider by the caller; conditioning on it opens a path between signal and return"
        elif role == "confounder":
            verdict, is_flag = "declared_confounder", False
            note = "declared a confounder, so it is not flagged as a collider; the declaration is taken as given and is not verified"
            if on_signal and on_forward:
                note += (
                    "; it depends on both a signal and a forward quantity, which is the collider pattern, "
                    "so check the declaration"
                )
        elif on_signal and on_forward:
            verdict, is_flag = "possible_collider", True
            note = "depends on a signal quantity and a forward-looking one: a common effect of the signal and the future return"
        elif on_forward:
            verdict, is_flag = "forward_looking", True
            note = "uses a forward-looking quantity: information from after the decision, a look-ahead fault rather than a collider"
        elif on_signal:
            verdict, is_flag = "signal_derived", False
            note = "computed from the signal's own quantities only; selecting on it is selecting on the signal"
        else:
            verdict, is_flag = "unrelated", False
            note = "shares no declared quantity with the signal or the forward set"
        rows.append(
            {
                "name": name,
                "causal_role": role,
                "verdict": verdict,
                "flagged": is_flag,
                "signal_dependencies": on_signal,
                "forward_dependencies": on_forward,
                "note": note,
            }
        )
        if is_flag:
            flagged.append(name)
    detail = {
        "check_type": "structural check",
        "n_filters": len(rows),
        "n_flagged": len(flagged),
        "flagged_names": ", ".join(flagged) if flagged else "none",
        "verdict": "flags raised: remove or recompute the filter without the forward quantity" if flagged else "no structural flag",
        "filters": rows,
        "quantities_in_both_sets": overlap,
        "warnings": [
            "a structural check on declared dependencies: it cannot find a dependency that was not named and never proves a collider",
            "no flag is not a clearance",
        ],
    }
    return result(detail, n=len(rows))
