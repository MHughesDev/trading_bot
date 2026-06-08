# System Specification Documentation Summary
## Complete Three-Phase Trading System Reference

**Created:** 2026-06-03  
**Location:** `docs/SYSTEM_SPECIFICATION.md` (and related files)  
**Status:** ✅ Committed and pushed to branch `claude/cool-lamport-BN3Bz`

---

## What Was Documented

You now have a **complete, unified specification** for the trading bot's current three-phase architecture:

### 📋 **SYSTEM_SPECIFICATION.md** (Main Reference)
The authoritative document covering:

#### Phase 1: Price Prediction (15–50ms)
- **ForecasterModel pipeline:**
  1. VSN (Variable Selection Network) — feature importance weighting
  2. Latent Encoder (3-layer CNN) — feature compression
  3. Multi-Resolution xLSTM — 4 parallel temporal branches (scales: 1, 5, 20, 100)
  4. Regime-Conditioned Fusion — adaptive branch weighting
  5. Quantile Decoder — probabilistic output (q_low, q_med, q_high)

- **Configuration:** 128 timestep history, 8-step forecast horizon, 3 quantiles (10th, 50th, 90th percentile)
- **Code reference:** `forecaster_model/models/forecaster_model.py`

#### Phase 2: Decision Handling (5–20ms)
- **3-Stage Trigger Engine:**
  1. **Setup:** Asymmetry + State Alignment + Confidence scores
  2. **Pre-Trigger:** Data freshness & execution quality validation
  3. **Confirm:** Trigger type classification & direction determination

- **Route Selection:** SCALPING | INTRADAY | SWING | CARRY
- **Output:** ActionProposal with direction, size_fraction, stop_distance
- **Code reference:** `decision_engine/trigger_engine.py`, `carry_sleeve/engine.py`

#### Phase 3: Trade Execution (30–100ms)
- **8 Hard Risk Gates** (precedence-ordered): Feed staleness → Data staleness → Spread → Drawdown → Tradability → Data health → System mode → Proposal validity

- **Canonical Sizing Stack** (6 multipliers):
  1. Degradation (system health)
  2. Position Inertia (flip penalty)
  3. Asymmetry Boost (edge boost)
  4. Liquidation Mode (offense/defense)
  5. Edge Budget (heat/concentration throttle)
  6. Concentration Multiplier (per-symbol + portfolio limits)

- **Output:** TradeAction → OrderIntent → Exchange API
- **Code reference:** `risk_engine/engine.py`, `risk_engine/canonical_sizing.py`

---

### 🎯 **PHASE_DESIGN_CHECKLIST.md** (Design Iteration Guide)
A template for evaluating **alternative designs or refactoring**:

- **Pre-design review:** Understanding current design, stakeholder alignment
- **Per-phase evaluation:** Input/output contracts, quality metrics, architecture alternatives
- **Cross-phase concerns:** Determinism, latency budget, monitoring, configuration
- **Design proposal template:** Problem statement, trade-offs, implementation plan, success criteria
- **Testing checklists:** Unit, integration, backtesting, stress tests

**Use this when exploring:**
- [ ] Switching from xLSTM to Transformer (Phase 1)
- [ ] Machine-learned trigger thresholds (Phase 2)
- [ ] Dynamic risk limits based on live Sharpe ratio (Phase 3)
- [ ] Order execution optimization (VWAP/TWAP splitting)

---

### 📖 **PHASE_QUICK_REFERENCE.md** (Desk Reference)
A one-page quick lookup card:

- **Visual flow diagram** of all three phases
- **Trigger stage formulas** with exact weights (0.35×A + 0.25×S + ...)
- **Risk gate thresholds** (300s, 50 bps, 15%, etc.)
- **Sizing multiplier stack** with example calculations
- **Configuration tables** (all tunable parameters)
- **File locations** (where is what?)
- **Common failure modes** with diagnostics
- **Key metrics** (per phase) and success criteria

**Print and post on your monitor** — designed for quick reference during development.

---

## How These Documents Relate

```
SYSTEM_SPECIFICATION.md (THE BIBLE)
    ├─ Authoritative source of truth
    ├─ Complete details + formulas + pseudocode
    ├─ Covers: inputs, outputs, config, monitoring
    └─ Use for: Understanding, debugging, reference

PHASE_DESIGN_CHECKLIST.md (EVALUATION FRAMEWORK)
    ├─ Derived from SYSTEM_SPECIFICATION.md
    ├─ Template for proposing changes
    ├─ Per-phase analysis questions
    ├─ Trade-off matrices
    └─ Use for: Design iterations, refactoring, alternatives

PHASE_QUICK_REFERENCE.md (QUICK LOOKUP)
    ├─ Condensed from SYSTEM_SPECIFICATION.md
    ├─ Visual & tabular format
    ├─ Exact numbers (thresholds, weights)
    ├─ File/code locations
    └─ Use for: Daily development, desk reference, debugging
```

---

## What You Can Do Now

### 1. **Understand the Current System** ✅
- Read the main spec to understand how the three phases fit together
- See exact formulas and decision logic
- Understand constraints and assumptions

### 2. **Evaluate Alternative Designs**
- Use `PHASE_DESIGN_CHECKLIST.md` to systematically compare alternatives
- Consider trade-offs (latency, accuracy, complexity)
- Fill out the design proposal template before coding

### 3. **Onboard New Team Members** ✅
- Give them `SYSTEM_SPECIFICATION.md` (technical depth)
- Have them bookmark `PHASE_QUICK_REFERENCE.md` (quick answers)
- Use `PHASE_DESIGN_CHECKLIST.md` when assigning tasks

### 4. **Debug Issues in Production**
- Quick reference tells you thresholds and gate logic
- Full spec gives you the math behind each decision
- Reason codes in decision records point to which phase failed

### 5. **Plan Refactoring**
- Identify which phase has the bottleneck (latency or accuracy)
- Use the checklist to evaluate alternatives
- Ensure no hidden dependencies between phases

---

## Quick Examples: Design Iterations

### Example 1: "Can we make Phase 1 faster?"

**Steps:**
1. Read `PHASE_QUICK_REFERENCE.md` → Phase 1: Latency budget is 15–50ms
2. Read `SYSTEM_SPECIFICATION.md` Phase 1 → Understand pipeline
3. Open `PHASE_DESIGN_CHECKLIST.md` → Phase 1 section
4. Ask: Replace xLSTM with Transformer? Trade-offs?
5. Fill out design proposal template
6. Backtest before deploying

---

### Example 2: "Can we reduce false signals in Phase 2?"

**Steps:**
1. Read `PHASE_QUICK_REFERENCE.md` → Phase 2 thresholds (0.22, 0.18, 0.2)
2. Open `PHASE_DESIGN_CHECKLIST.md` → Phase 2 "Threshold Tuning"
3. Calculate: What % of triggers are profitable?
4. Propose: Raise setup_threshold from 0.22 → 0.25?
5. A/B test on historical data
6. Update config and monitor metrics

---

### Example 3: "Why are orders getting blocked?"

**Steps:**
1. Look at decision record → check `last_risk_block_codes`
2. Reference `PHASE_QUICK_REFERENCE.md` → Risk gates table
3. Read `SYSTEM_SPECIFICATION.md` Phase 3 → which gate is firing?
4. Check `docs/MONITORING_CANONICAL.MD` → metrics for that gate
5. Adjust threshold or system mode

---

## Key Takeaways

| Aspect | What You Have Now |
|--------|-------------------|
| **System Understanding** | Complete spec with formulas, code refs, contracts |
| **Design Framework** | Checklist for evaluating alternatives |
| **Quick Reference** | One-page card with thresholds, configs, failure modes |
| **Determinism** | All phases deterministic (replayable) |
| **Latency** | Well-understood budget (50–170ms total) |
| **Monitoring** | Key metrics defined per phase |
| **Configuration** | All tunable parameters documented |
| **Testing** | Checklists for unit, integration, backtest |

---

## Files Created

```
docs/
├── SYSTEM_SPECIFICATION.md          (2400 lines, ~20KB)
│   └── Authoritative reference (Phase 1, 2, 3)
│
├── PHASE_DESIGN_CHECKLIST.md        (600 lines, ~10KB)
│   └── Design iteration template
│
└── PHASE_QUICK_REFERENCE.md         (400 lines, ~8KB)
    └── Quick lookup card
```

**Total:** ~3400 lines of documentation covering current system + design framework

---

## Next Steps

### Immediate
1. **Read** `docs/SYSTEM_SPECIFICATION.md` (20 min)
2. **Bookmark** `docs/PHASE_QUICK_REFERENCE.md` (on your desktop)
3. **Review** `docs/PHASE_DESIGN_CHECKLIST.md` when proposing changes

### For Design Iterations
1. Identify which phase to change
2. Open `PHASE_DESIGN_CHECKLIST.md` for that phase
3. Fill out the "Design Proposal Template"
4. Get stakeholder alignment
5. Implement with tests
6. Update `SYSTEM_SPECIFICATION.md` if design changes

### For Team Onboarding
- New engineer? → Give them the full spec
- Quick question? → Point to quick reference
- Design review? → Use the checklist

---

## Questions Your Docs Can Answer Now

✅ What does the ForecasterModel pipeline do?  
✅ How are the 3 trigger stages computed?  
✅ What are all 8 risk gates?  
✅ What are the 6 sizing multipliers?  
✅ How do I tune Phase 2 thresholds?  
✅ What's the latency budget?  
✅ How do I replay a decision?  
✅ What's the fastest way to understand Phase 3?  
✅ Can I change Phase 1 without breaking Phase 2?  
✅ What metrics should I monitor?  

---

## Git Status

```bash
# Committed to branch: claude/cool-lamport-BN3Bz
# Files:
#   - docs/SYSTEM_SPECIFICATION.md
#   - docs/PHASE_DESIGN_CHECKLIST.md
#   - docs/PHASE_QUICK_REFERENCE.md
#
# Status: Pushed and ready for review
```

---

## Why This Matters

**Before:** System knowledge was scattered across:
- Code comments
- Historical PRs
- Developer heads
- Various .md files with different levels of detail

**After:** System knowledge is:
- ✅ Centralized in one authoritative spec
- ✅ Structured (clear sections per phase)
- ✅ Comprehensive (inputs, outputs, formulas, code refs)
- ✅ Actionable (design checklist for iterations)
- ✅ Accessible (quick reference card)
- ✅ Traceable (all decisions have reason codes)

This enables:
- **Faster onboarding** (read one document)
- **Better design reviews** (use the checklist)
- **Confident refactoring** (understand constraints)
- **Easier debugging** (quick reference tells you thresholds)
- **Reproducible decisions** (all formulas documented)

---

**Ready to explore alternative designs?**  
→ Open `docs/PHASE_DESIGN_CHECKLIST.md` and fill out a proposal!

**Want to understand the code better?**  
→ Start with `docs/SYSTEM_SPECIFICATION.md`, Phase 1 or 3 depending on interest.

**Quick question about Phase 2 thresholds?**  
→ `docs/PHASE_QUICK_REFERENCE.md` has the answer.
