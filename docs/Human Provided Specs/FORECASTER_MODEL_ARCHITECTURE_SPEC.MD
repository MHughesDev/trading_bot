# Code-Level Architecture Specification — Forecaster Model
## VSN → Latent CNN → Multi-Resolution xLSTM → Regime-Conditioned Fusion → Quantile Head → Online Conformal Calibration → Ensemble

> **Target architecture:** For the **APEX canonical** migration and **intended** end-state system design, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`new_specs/canonical/`](new_specs/canonical/). This document remains **historical / legacy intent** for the pre-canonical pipeline where applicable; where it conflicts with canonical APEX on **future** architecture, prefer canonical specs for roadmap decisions.


Version: 1.0  
Document type: implementation specification  
Scope: model architecture, data contracts, training/inference design, module interfaces, extension points  
Target use case: short-horizon single-asset financial forecasting at 15-second to 1-minute cadence

---

# 1. Purpose

This document defines the code-level architecture for a financial forecasting system whose role is to act as the **forecaster model** inside a larger trading stack. The model is not the execution engine and not the policy engine. Its responsibility is to ingest recent market state, transform that state into a structured latent representation, and output a **multi-horizon probabilistic forecast** of future returns with uncertainty estimates that remain usable under non-stationarity.

The core architectural conclusion embodied in this spec is:

1. raw engineered features should not be passed directly into the temporal core without filtering,
2. feature relevance is dynamic and therefore requires **variable selection**,
3. financial time series are inherently **multi-resolution**,
4. recurrent state tracking is a stronger inductive bias for financial prediction than a purely attention-first approach,
5. output must be **probabilistic**, not point-only,
6. uncertainty must be **recalibrated online**, not trusted naively,
7. all components must be organized so they can be replaced or ablated independently.

This spec therefore formalizes a modular hybrid architecture built around a **VSN-centered multi-resolution xLSTM backbone** with optional latent encoding, regime conditioning, conformal calibration, and ensembling.

---

# 2. Design Principles

## 2.1 Architectural principles

The system shall satisfy the following architectural principles:

1. **Separation of concerns**  
   Forecasting, decision-making, execution, and risk control must remain separate modules.

2. **Probabilistic forecasting by default**  
   Outputs must be distributional, represented initially through quantiles.

3. **Multi-horizon outputs**  
   The model must forecast a sequence of future horizons rather than a single next step.

4. **State-centric temporal modeling**  
   The temporal core must maintain evolving hidden state across time.

5. **Multi-resolution representation**  
   The architecture must expose both local and slower temporal dynamics.

6. **Adaptive feature relevance**  
   Feature importance must be dynamically estimated rather than statically assumed.

7. **Regime sensitivity**  
   Model internals must be able to alter emphasis based on inferred market regime.

8. **Calibration awareness**  
   Prediction intervals must be adjusted using post-hoc calibration under drift.

9. **Extensibility**  
   Individual modules must be replaceable without redesigning the full stack.

10. **No infrastructure assumptions**  
    This spec deliberately does not assume any particular hardware, accelerator, framework version, orchestration system, queueing layer, or deployment substrate.

## 2.2 Code organization principles

The codebase should be partitioned into the following concerns:

- configuration
- schemas / tensor contracts
- feature engineering
- normalization
- regime modeling
- model modules
- loss and metrics
- calibration
- training orchestration
- evaluation / backtesting adapters
- inference service adapters
- serialization and checkpointing

A reference package layout is proposed later in this document, but the exact language/runtime organization may vary.

---

# 3. Functional Requirements

The forecasting module must support:

1. ingestion of base-frequency market observations,
2. sliding-window dataset construction,
3. observed-feature construction,
4. known-future feature construction,
5. static-feature support,
6. global and regime-conditioned normalization,
7. soft regime estimation,
8. variable selection over observed features,
9. latent sequence encoding,
10. multi-resolution recurrent modeling,
11. regime-conditioned fusion,
12. multi-horizon quantile prediction,
13. conformal interval recalibration,
14. model ensembling,
15. walk-forward retraining,
16. inference-time confidence rejection conditions,
17. export of forecasts and diagnostics.

---

# 4. Mathematical Problem Definition

Let the market observation at time `t` be represented by a raw vector `o_t`.

From `o_t`, construct:

- observed historical tensor `X_obs ∈ ℝ^(B × L × F_obs)`
- known future tensor `X_known ∈ ℝ^(B × H × F_known)`
- static tensor `X_static ∈ ℝ^(B × F_static)`
- target tensor `Y ∈ ℝ^(B × H)` where each target is a future return

The system approximates:

`f_θ(X_obs, X_known, X_static) -> Q`

where `Q ∈ ℝ^(B × H × Qn)` and `Qn` is the number of quantiles.

For default quantiles `{0.1, 0.5, 0.9}`:

`Q[b, h, :] = [q_0.1, q_0.5, q_0.9]`

The target return for horizon `h` is:

`y_(t,h) = log(P_(t+h) / P_t)`

The base training loss is the multi-horizon quantile loss:

`L = Σ_b Σ_h Σ_q max(q(y - ŷ_q), (q - 1)(y - ŷ_q))`

Conformal calibration later adjusts forecast intervals using residual statistics collected on a calibration stream.

---

# 5. Tensor Contracts

This section defines the canonical tensor contracts. Every implementation should preserve these contracts even if the internal framework uses named tensors, structured batches, dataclasses, or protocol buffers.

## 5.1 Canonical dimensions

- `B`: batch size
- `L`: encoder/history length
- `H`: forecast horizon
- `F_obs`: observed feature count
- `F_known`: known-future feature count
- `F_static`: static feature count
- `D_model`: internal model width
- `D_latent`: latent encoder width
- `D_hidden`: recurrent hidden width
- `S`: number of multi-resolution branches
- `Qn`: number of output quantiles

## 5.2 Input tensors

### Observed historical tensor

`X_obs: [B, L, F_obs]`

Contains only information known up to and including time `t`.

### Known future tensor

`X_known: [B, H, F_known]`

Contains information available at forecast time for future horizons, such as cyclical time encodings and session flags.

### Static tensor

`X_static: [B, F_static]`

Contains invariant or slowly changing metadata.

### Regime tensor

Two patterns are allowed:

- time-local regime tensor: `R_seq: [B, L, F_regime]`
- current-step regime tensor: `R_cur: [B, F_regime]`

Default `F_regime = 4`.

### Target tensor

`Y: [B, H]`

Optionally expanded to `Y: [B, H, Tn]` for multitask extensions.

## 5.3 Intermediate tensors

### Variable-selected tensor

`X_vsn: [B, L, F_obs]`

### Latent sequence tensor

If the latent encoder emits per-step latents:

`Z_seq: [B, L, D_latent]`

If it emits a summary vector:

`Z_ctx: [B, D_latent]`

### Branch outputs

For branch `s`:

`H_s: [B, L, D_hidden]`

### Fused recurrent state

`H_fused: [B, L, D_hidden]`

### Decoder context

`C_dec: [B, H, D_model]`

### Quantile output

`Y_hat_q: [B, H, Qn]`

### Ensemble output

`Y_hat_q_ens: [B, H, Qn]`

---

# 6. Configuration Schema

A framework-agnostic implementation should expose a configuration object. This may be represented as a typed config class, validated dict, YAML schema, TOML schema, JSON schema, or equivalent.

A recommended logical schema is:

```yaml
system:
  name: forecaster_model
  version: 1.0

data:
  base_interval_seconds: 15
  history_length: 128
  forecast_horizon: 8
  quantiles: [0.1, 0.5, 0.9]
  target: log_return
  feature_windows:
    short: [4, 8]
    medium: [20, 60]
    long: [240, 960]
features:
  include_volume: true
  include_range: true
  include_spread: false
  include_time_features: true
  include_regime_features: true
  normalization:
    rolling_window: 1024
    use_regime_conditioned: true
regime:
  num_regimes: 4
  mode: soft
  estimator: rule_based
vsn:
  hidden_dim: 64
  dropout: 0.1
latent_encoder:
  enabled: true
  mode: causal_cnn
  latent_dim: 32
  conv_channels: [32, 64, 64]
  kernel_sizes: [3, 5, 7]
recurrent:
  branch_scales: [1, 5, 20, 100]
  cell_type: xlstm
  hidden_dim: 128
  num_layers: 1
  branch_dropout: 0.1
fusion:
  mode: regime_conditioned_gating
  hidden_dim: 64
head:
  decoder_hidden_dim: 128
  use_temporal_attention: false
loss:
  quantile_loss: true
  quantile_crossing_penalty: 0.0
  smoothness_penalty: 0.0
calibration:
  enabled: true
  mode: sliding_conformal
  alpha: 0.1
  window_size: 2048
ensemble:
  enabled: true
  members: 3
  vary_seed: true
  vary_dropout: true
  vary_history_length: true
training:
  optimizer: adamw
  learning_rate: 0.0003
  weight_decay: 0.01
  grad_clip_norm: 1.0
  early_stopping_patience: 10
  walk_forward: true
inference:
  reject_on_ood: true
  reject_on_interval_width: true
  reject_on_ensemble_disagreement: true
```

This schema is not prescriptive in syntax, only in logical scope.

---

# 7. Feature Engineering Layer

## 7.1 Purpose

The feature engineering layer transforms raw market observations into normalized, structured variables suitable for sequence modeling. Its role is not to “solve” the forecasting problem through handcrafted indicators, but to provide stable, interpretable primitives from which the model can learn temporal structure.

## 7.2 Raw market inputs

The minimum raw input set should include:

- timestamp
- open
- high
- low
- close
- volume

Optional inputs:

- bid/ask spread
- trade count
- realized trade imbalance
- depth summaries
- funding / mark price fields if relevant to venue

## 7.3 Core observed features

At each base timestep `t`, compute:

1. `log_return_1 = log(close_t / close_(t-1))`
2. `abs_return = |log_return_1|`
3. `close_open_return = log(close_t / open_t)`
4. `range_ratio = (high_t - low_t) / max(|close_t|, ε)`
5. `volume_change = log((volume_t + ε) / (volume_(t-1) + ε))`
6. optional `spread_ratio = spread_t / close_t`

## 7.4 Rolling / multi-scale features

For each configured window `k`, compute:

1. rolling mean return
2. rolling standard deviation
3. realized volatility
4. cumulative momentum
5. rolling z-score of close
6. rolling max drawdown proxy
7. rolling average range
8. rolling average volume

Mathematically:

`μ_t^(k) = (1/k) Σ_(i=0..k-1) r_(t-i)`

`σ_t^(k) = sqrt((1/k) Σ_(i=0..k-1) (r_(t-i) - μ_t^(k))^2)`

`mom_t^(k) = log(close_t / close_(t-k))`

`rv_t^(k) = sqrt(Σ_(i=0..k-1) r_(t-i)^2)`

The exact feature set is configurable, but every implementation should preserve the distinction between:

- base-frequency features,
- short-window summaries,
- medium-window summaries,
- long-window summaries.

## 7.5 Known-future features

Known-future features must be computable for future horizons at inference time. Default features:

1. sine/cosine minute-of-hour
2. sine/cosine hour-of-day
3. sine/cosine day-of-week
4. optional session flags
5. optional market-open / pre-close indicators for markets with sessions

For cyclic variable `τ` with period `T`:

`sin(2π τ / T), cos(2π τ / T)`

These are stored per horizon `h` inside `X_known`.

## 7.6 Static features

Static features are optional in a single-asset build but the interface should exist. Defaults may include:

- asset embedding id
- venue embedding id
- bar interval id
- configuration profile id

---

# 8. Normalization Layer

## 8.1 Purpose

Normalization is critical because financial feature distributions drift over time. The normalization layer standardizes features without introducing lookahead leakage.

## 8.2 Global rolling normalization

For each feature `x_t` over rolling window `W`:

`x'_t = (x_t - μ_t^W) / max(σ_t^W, ε)`

where `μ_t^W` and `σ_t^W` are computed only from observations available up to time `t`.

## 8.3 Regime-conditioned normalization

Given a soft regime vector `R_t`, a regime-conditioned normalization layer may maintain per-regime running statistics or weighted statistics.

Hard-bucket version:

`x''_t = (x_t - μ_(regime,t)) / max(σ_(regime,t), ε)`

Soft-weight version:

`μ̃_t = Σ_j p_(t,j) μ_j`

`σ̃_t = Σ_j p_(t,j) σ_j`

`x''_t = (x_t - μ̃_t) / max(σ̃_t, ε)`

Both globally normalized and regime-normalized variants may be passed downstream so the model can learn which representation is more useful.

---

# 9. Regime Estimation Layer

## 9.1 Purpose

The regime layer provides a compact approximation of macro-state. It is not intended to be the final truth about market structure; it is a conditioning mechanism that lets downstream modules alter emphasis depending on current behavior.

## 9.2 Initial regime taxonomy

Default four-regime taxonomy:

1. uptrend
2. downtrend
3. low-volatility chop
4. high-volatility chop

## 9.3 Rule-based estimator

A first implementation may use deterministic heuristics derived from:

- rolling trend slope
- realized volatility
- momentum
- autocorrelation
- distance from moving mean

The output should be a **soft vector** rather than a hard class when possible:

`R_t = [p_up, p_down, p_chop_low, p_chop_high]`

## 9.4 Extensible estimator interface

The regime estimator should implement an interface equivalent to:

```python
class RegimeEstimatorProtocol:
    def fit(self, features):
        ...
    def update(self, feature_row):
        ...
    def predict_proba(self, feature_row):
        ...
```

Possible implementations:

- rule-based
- hidden Markov model
- Gaussian mixture model
- clustering-based pattern assignment
- learned classifier

The forecasting stack must depend on the interface, not the specific estimator.

---

# 10. Variable Selection Network

## 10.1 Purpose

The VSN is the first learned stage after feature construction. Its role is to dynamically assign relevance weights across observed features so the temporal core is not forced to model noise and irrelevant channels at full strength.

## 10.2 Functional behavior

Input:

`X_obs: [B, L, F_obs]`

At each timestep, compute feature scores and transform them into normalized gates. Conceptually:

`g_t = softmax(MLP(x_t))`

`x_vsn,t = g_t ⊙ x_t`

Alternative formulations may include:

- independent sigmoid gates per feature,
- grouped feature gating,
- gated residual combination of raw and transformed inputs.

## 10.3 Grouped feature selection

For interpretability and stability, features may also be grouped into semantic blocks:

- price / return block
- volatility block
- volume block
- time block
- regime block
- optional microstructure block

The VSN may compute:

- intra-block feature gates,
- inter-block gates.

## 10.4 Recommended module interface

```python
class VariableSelectionNetwork(nn.Module):
    def __init__(self, input_dim, hidden_dim, dropout=0.0, grouped=False, groups=None):
        ...

    def forward(self, x, context=None):
        """
        x: [B, L, F_obs]
        context: optional conditioning tensor
        returns:
            x_selected: [B, L, F_obs]
            gates: [B, L, F_obs] or grouped equivalents
        """
        ...
```

## 10.5 Diagnostics

The VSN should expose gate statistics:

- mean gate weight by feature,
- variance of gate weight by feature,
- gate shifts by regime,
- gate shifts across walk-forward periods.

---

# 11. Latent Encoder

## 11.1 Purpose

The latent encoder compresses local temporal patterns into a richer representation before recurrent modeling. It helps transform noisy engineered features into a denser, learned market state.

## 11.2 Default encoder choice

Default choice: **causal 1D CNN**.

Rationale:

- strong local pattern extraction,
- low conceptual complexity,
- naturally causal,
- cheaper and simpler than a large transformer front-end,
- suitable for learning short local motifs prior to recurrent state tracking.

## 11.3 Default topology

A recommended initial topology:

- Conv1D(kernel=3, channels=32)
- Conv1D(kernel=5, channels=64)
- Conv1D(kernel=7, channels=64)
- nonlinear activation after each layer
- optional normalization layer
- optional residual skip connections

The output may be either:

1. a per-step latent sequence `Z_seq: [B, L, D_latent]`, or
2. a summary vector `Z_ctx: [B, D_latent]`

Default recommendation: preserve sequence form when possible.

## 11.4 Recommended module interface

```python
class LatentEncoder(nn.Module):
    def __init__(self, input_dim, latent_dim, channels, kernel_sizes, dropout=0.0):
        ...

    def forward(self, x):
        """
        x: [B, L, F]
        returns:
            z_seq: [B, L, D_latent]
            z_ctx: [B, D_latent]
        """
        ...
```

## 11.5 Extensible variants

Possible future replacements:

- dilated temporal CNN
- temporal convolutional network
- causal transformer encoder
- contrastive pretraining encoder
- state-space encoder

---

# 12. Multi-Resolution xLSTM Backbone

## 12.1 Purpose

This is the primary temporal modeling core. It combines:

- recurrent hidden-state evolution,
- multiple temporal resolutions,
- improved memory dynamics from xLSTM-style cells.

## 12.2 Branch scales

Define branch scales:

`S = {s_1, s_2, ..., s_n}`

Default recommendation:

`S = {1, 5, 20, 100}`

Interpretation:

- scale 1: base-resolution branch,
- scale 5: short compressed branch,
- scale 20: medium branch,
- scale 100: slow branch.

These are starting points, not universal truths.

## 12.3 Downsampling semantics

Each branch receives an input sequence sampled or pooled at its corresponding scale. A branch may downsample via:

- strided slicing,
- average pooling,
- learned pooling,
- feature-preserving aggregation.

The implementation must make the downsampling strategy explicit because it changes information retention.

## 12.4 xLSTM branch contract

Each branch processes its scale-specific sequence using an xLSTM-like recurrent module.

Conceptually:

`H_s = xLSTM_s(X_s)`

where:

- `X_s: [B, L_s, D_in]`
- `H_s: [B, L_s, D_hidden]`

## 12.5 Branch alignment

Since branches operate at different lengths, they must be aligned back to base resolution before fusion.

Possible alignment methods:

- repeat-upsample,
- interpolation,
- learned upsampling projection.

Default recommendation: deterministic repeat-alignment for early versions, because it is stable and easy to reason about.

After alignment:

`H_s_aligned: [B, L, D_hidden]`

## 12.6 Recommended backbone interface

```python
class MultiResolutionXLSTM(nn.Module):
    def __init__(self, input_dim, hidden_dim, branch_scales, num_layers=1, dropout=0.0):
        ...

    def forward(self, x):
        """
        x: [B, L, D_in]
        returns:
            branch_outputs: dict[int, Tensor]  # each [B, L, D_hidden]
        """
        ...
```

## 12.7 Implementation note on xLSTM

The exact xLSTM cell variant may differ by implementation. This spec cares about behavioral properties:

- more stable long-range memory than vanilla LSTM,
- improved gating dynamics,
- recurrent state evolution suitable for noisy financial signals.

If a strict xLSTM implementation is unavailable, the abstraction should still permit:

- drop-in replacement with a custom enhanced LSTM cell,
- replacement with a modern recurrent memory unit exposing the same interface.

---

# 13. Regime-Conditioned Fusion Layer

## 13.1 Purpose

The fusion layer combines branch outputs while conditioning their contribution on regime information. This gives the model the ability to emphasize different temporal scales in different market states.

## 13.2 Fusion input

Given branch outputs:

`H_1, H_5, H_20, H_100`

all shaped `[B, L, D_hidden]`, stack or concatenate them:

`H_stack: [B, L, S, D_hidden]`

## 13.3 Regime-conditioned weighting

Using a regime vector `R_cur: [B, F_regime]`, compute branch weights:

`α = softmax(MLP(R_cur))`

with:

`α: [B, S]`

Then fuse:

`H_fused[b, t, :] = Σ_s α[b, s] * H_stack[b, t, s, :]`

Alternative variant: compute time-varying fusion weights using `R_seq`.

## 13.4 Recommended interface

```python
class RegimeConditionedFusion(nn.Module):
    def __init__(self, num_branches, hidden_dim, regime_dim, fusion_hidden_dim):
        ...

    def forward(self, branch_outputs, regime):
        """
        branch_outputs: list[Tensor] each [B, L, D_hidden]
        regime: [B, F_regime] or [B, L, F_regime]
        returns:
            h_fused: [B, L, D_hidden]
            fusion_weights: [B, S] or [B, L, S]
        """
        ...
```

## 13.5 Diagnostics

Record:

- average branch weights by regime,
- branch-weight shifts over time,
- relation between branch weights and forecast uncertainty.

---

# 14. Forecast Decoder and Quantile Head

## 14.1 Purpose

The decoder transforms fused historical state into horizon-specific forecast representations and projects them into quantile outputs.

## 14.2 Decoder inputs

Potential inputs include:

- final or sequence recurrent state `H_fused`
- latent context `Z_ctx`
- future known features `X_known`
- static context `X_static`

A common construction is:

1. derive a historical context summary from `H_fused`,
2. tile or broadcast that summary across the horizon dimension,
3. concatenate with `X_known` and static context,
4. pass through a gated residual decoder MLP or lightweight temporal decoder.

## 14.3 Default output head

Project to `Qn` quantiles for each horizon.

Output:

`Y_hat_q: [B, H, Qn]`

For default `Qn = 3`, the output corresponds to:

- lower quantile `q10`
- median `q50`
- upper quantile `q90`

## 14.4 Recommended interface

```python
class QuantileDecoder(nn.Module):
    def __init__(self, fused_dim, known_dim, static_dim, hidden_dim, horizon, quantiles):
        ...

    def forward(self, h_fused, x_known, x_static=None, z_ctx=None):
        """
        h_fused: [B, L, D_hidden]
        x_known: [B, H, F_known]
        x_static: [B, F_static] | None
        z_ctx: [B, D_latent] | None
        returns:
            y_hat_q: [B, H, Qn]
        """
        ...
```

## 14.5 Quantile ordering

The implementation should either:

- explicitly constrain monotonic quantile ordering, or
- apply a crossing penalty during training.

---

# 15. Loss Functions and Training Objectives

## 15.1 Primary loss

The base loss is the summed pinball / quantile loss across all forecast horizons.

For true target `y` and predicted quantile `ŷ_q`:

`L_q(y, ŷ_q) = max(q(y - ŷ_q), (q - 1)(y - ŷ_q))`

Total batch loss:

`L_main = mean over batch and horizons of Σ_q L_q`

## 15.2 Optional penalties

### Quantile crossing penalty

Penalize violations of ordered quantiles:

`L_cross = mean(ReLU(q10 - q50) + ReLU(q50 - q90))`

### Forecast smoothness penalty

Encourage smoother horizon trajectories when appropriate:

`L_smooth = mean_h ||ŷ_(h+1) - ŷ_h||`

### Regime consistency penalty

Optional experimental term to encourage coherent forecast shifts under regime changes.

## 15.3 Total objective

`L_total = L_main + λ_cross L_cross + λ_smooth L_smooth + ...`

The weighting parameters must be configurable.

---

# 16. Conformal Calibration Layer

## 16.1 Purpose

Neural quantiles are useful but not automatically reliable under drift. The conformal layer adjusts interval widths based on calibration residuals so the resulting intervals maintain better empirical coverage.

## 16.2 Default mode

Recommended default: **sliding-window conformal calibration**.

## 16.3 Calibration residuals

For interval calibration, compute nonconformity scores from a calibration stream.

A simple score:

`ε_t = max(q_low_t - y_t, y_t - q_high_t, 0)`

or symmetric score variants.

For target coverage `1 - α`, estimate the empirical `(1 - α)` quantile of recent residuals:

`q_ε = Quantile_(1-α)(ε_(t-W+1:t))`

Then adjust interval:

`[q_low', q_high'] = [q_low - q_ε, q_high + q_ε]`

## 16.4 Interface

```python
class ConformalCalibrator:
    def __init__(self, alpha, window_size):
        ...

    def update(self, y_true, q_low, q_high):
        ...

    def calibrate(self, q_low, q_high):
        ...
```

## 16.5 Persistence

The calibrator state must be serializable independently of model weights.

---

# 17. Ensemble Layer

## 17.1 Purpose

The ensemble reduces sensitivity to initialization, hyperparameter variance, and local overfitting.

## 17.2 Ensemble diversity sources

Recommended diversity dimensions:

1. random seed,
2. history length,
3. dropout,
4. branch scale variants,
5. latent encoder width.

## 17.3 Aggregation

Default aggregation:

`Y_hat_q_ens = (1/N) Σ_i Y_hat_q^(i)`

Alternative aggregations:

- trimmed mean,
- median over members,
- confidence-weighted mean.

## 17.4 Ensemble diagnostics

Track:

- ensemble variance by horizon,
- disagreement rate,
- disagreement by regime,
- disagreement versus realized errors.

---

# 18. Dataset Construction

## 18.1 Sliding windows

For anchor time `t`, construct:

- history indices: `[t-L+1, ..., t]`
- forecast indices: `[t+1, ..., t+H]`

This yields one supervised sample.

## 18.2 Leakage prevention

The dataset builder must guarantee:

1. no future values included in observed history,
2. no future normalization statistics,
3. no overlap-induced leakage between train and validation/test windows where prohibited,
4. calibration windows remain strictly post-train.

## 18.3 Sample schema

A recommended sample object:

```python
@dataclass
class ForecastSample:
    x_obs: Tensor
    x_known: Tensor
    x_static: Tensor | None
    regime_seq: Tensor | None
    regime_cur: Tensor | None
    y: Tensor
    timestamps_hist: Any
    timestamps_fut: Any
```

---

# 19. Data Loader and Batch Assembly

## 19.1 Responsibilities

The data loader layer should:

- stream or load transformed samples,
- collate into batches,
- preserve temporal order within each sample,
- optionally support weighted sampling by regime.

## 19.2 Batch object

A recommended batch schema:

```python
@dataclass
class ForecastBatch:
    x_obs: Tensor         # [B, L, F_obs]
    x_known: Tensor       # [B, H, F_known]
    x_static: Tensor | None
    regime_seq: Tensor | None
    regime_cur: Tensor | None
    y: Tensor             # [B, H]
    metadata: dict
```

---

# 20. Top-Level Model Composition

## 20.1 Reference composition

```python
class ForecasterModel(nn.Module):
    def __init__(self, cfg):
        self.vsn = VariableSelectionNetwork(...)
        self.latent_encoder = LatentEncoder(...)
        self.recurrent = MultiResolutionXLSTM(...)
        self.fusion = RegimeConditionedFusion(...)
        self.decoder = QuantileDecoder(...)

    def forward(self, x_obs, x_known, x_static=None, regime_seq=None, regime_cur=None):
        x_vsn, gates = self.vsn(x_obs, context=regime_seq)
        z_seq, z_ctx = self.latent_encoder(x_vsn)
        branch_outputs = self.recurrent(z_seq)
        h_fused, fusion_weights = self.fusion(list(branch_outputs.values()), regime_cur)
        y_hat_q = self.decoder(h_fused, x_known, x_static=x_static, z_ctx=z_ctx)
        return {
            "y_hat_q": y_hat_q,
            "gates": gates,
            "branch_outputs": branch_outputs,
            "fusion_weights": fusion_weights,
            "z_ctx": z_ctx,
        }
```

## 20.2 Forward-pass semantics

Step-by-step:

1. observed sequence enters VSN,
2. VSN suppresses weak/noisy channels,
3. latent encoder compresses local temporal motifs,
4. multi-resolution recurrent branches track state at different scales,
5. branch outputs are aligned and fused using regime-dependent weights,
6. decoder combines fused state with future-known covariates,
7. quantile head emits multi-horizon probabilistic forecasts.

---

# 21. Training Orchestration

## 21.1 Trainer responsibilities

The trainer should manage:

- batch iteration,
- forward pass,
- loss computation,
- optimizer step,
- scheduler step,
- gradient clipping,
- checkpointing,
- validation,
- early stopping,
- logging.

## 21.2 Walk-forward protocol

Use time-respecting train/validate/test progression.

For each walk-forward split:

1. fit or refresh normalization state using training history only,
2. fit or refresh regime estimator using training history only,
3. train model on train split,
4. tune on validation split,
5. freeze model,
6. evaluate on next unseen test segment,
7. advance window and repeat.

## 21.3 Recommended training loop skeleton

```python
for epoch in range(num_epochs):
    model.train()
    for batch in train_loader:
        out = model(
            batch.x_obs,
            batch.x_known,
            x_static=batch.x_static,
            regime_seq=batch.regime_seq,
            regime_cur=batch.regime_cur,
        )
        loss_dict = loss_fn(out["y_hat_q"], batch.y)
        loss = loss_dict["total"]
        optimizer.zero_grad()
        loss.backward()
        clip_grad_norm_(model.parameters(), cfg.training.grad_clip_norm)
        optimizer.step()

    val_metrics = evaluate(model, val_loader, loss_fn)
    scheduler.step(val_metrics["primary"])
    checkpoint_manager.maybe_save(model, val_metrics)
```

---

# 22. Evaluation Metrics

## 22.1 Forecasting metrics

The evaluation module should include at minimum:

- quantile loss
- median MAE
- directional accuracy by horizon
- interval coverage probability
- mean interval width
- calibration error
- regime-stratified metrics
- time-of-day stratified metrics

## 22.2 Trading-adjacent metrics

Even though this is a forecasting module, downstream proxy metrics are useful:

- sign-based hit rate
- uncertainty-filtered hit rate
- signal-threshold proxy PnL
- cost-adjusted proxy PnL
- Sharpe-like proxy on thresholded forecasts

These remain secondary to true forecasting metrics inside this module.

---

# 23. Inference Pipeline

## 23.1 Runtime stages

At runtime, the inference stack should execute the following steps in order:

1. ingest latest market observation,
2. update rolling features,
3. update normalization state,
4. update regime estimate,
5. construct `X_obs`, `X_known`, `X_static`,
6. run each ensemble member,
7. aggregate member outputs,
8. apply conformal calibration,
9. compute inference diagnostics,
10. publish forecast packet.

## 23.2 Forecast packet contract

A recommended return object:

```python
@dataclass
class ForecastPacket:
    q_low: Tensor             # [H]
    q_med: Tensor             # [H]
    q_high: Tensor            # [H]
    interval_width: Tensor    # [H]
    regime: Tensor            # [F_regime]
    confidence: Tensor        # [H] or scalar
    ensemble_variance: Tensor # [H, Qn]
    diagnostics: dict
```

## 23.3 Confidence scoring

A simple initial confidence score may be derived from:

`confidence_h = q_med_h / (|q_high_h - q_low_h| + ε)`

This is not mandatory as a final metric, but it is a useful export.

---

# 24. Rejection and Guard Conditions

The module should expose rejection conditions so downstream policy logic can ignore unreliable forecasts.

Default rejection checks:

1. interval width above threshold,
2. ensemble disagreement above threshold,
3. out-of-distribution feature score above threshold,
4. missing feature values,
5. stale normalization state,
6. insufficient calibration history.

Recommended interface:

```python
class ForecastGuard:
    def __init__(self, ...):
        ...

    def check(self, forecast_output, batch_metadata):
        """returns {accepted: bool, reasons: list[str]}"""
        ...
```

---

# 25. Serialization and Checkpointing

The system should serialize independently:

1. model weights,
2. optimizer state,
3. scheduler state,
4. configuration,
5. normalization state,
6. regime estimator state,
7. conformal calibrator state,
8. feature schema version,
9. training metadata.

This separation matters because the non-neural states are operationally critical.

---

# 26. Recommended Package Layout

A framework-neutral logical layout:

```text
forecaster_model/
  config/
    schema.py
    defaults.yaml
  data/
    raw_schema.py
    dataset.py
    dataloader.py
    windowing.py
  features/
    builders.py
    rolling.py
    time_features.py
    normalization.py
  regime/
    estimator.py
    heuristics.py
  models/
    vsn.py
    latent_encoder.py
    xlstm_cell.py
    multi_resolution_xlstm.py
    fusion.py
    decoder.py
    forecaster_model.py
  losses/
    quantile.py
    auxiliary.py
  calibration/
    conformal.py
  training/
    trainer.py
    checkpoints.py
    evaluators.py
    metrics.py
  inference/
    service.py
    guards.py
    packets.py
  utils/
    logging.py
    seed.py
    serialization.py
```

This layout is illustrative, not mandatory.

---

# 27. Ablation Matrix

To validate architectural choices, the implementation roadmap should support controlled ablations.

Minimum ablations:

1. no VSN,
2. no latent encoder,
3. single-resolution xLSTM only,
4. no regime-conditioned fusion,
5. no conformal calibration,
6. no ensemble,
7. vanilla LSTM in place of xLSTM,
8. raw features vs grouped feature blocks,
9. global normalization only vs global + regime-conditioned normalization.

This is necessary so each architectural decision earns its place.

---

# 28. Extension Points

Future upgrades should fit through explicit extension points.

## 28.1 Candidate extensions

1. hidden Markov or learned regime estimator,
2. state-space encoder or backbone,
3. patch-attention summary head,
4. mixture-of-experts fusion,
5. cross-asset feature blocks,
6. order-flow feature block,
7. multitask heads for return + volatility,
8. adaptive branch scales,
9. contrastive pretraining for latent encoder.

## 28.2 Interface discipline

Every extension should preserve the top-level model contract:

`(X_obs, X_known, X_static, regime) -> quantile forecasts + diagnostics`

---

# 29. Implementation Priorities

If building incrementally, implement in this order:

1. feature engineering and leakage-safe dataset builder,
2. normalization layer,
3. soft regime estimator,
4. VSN,
5. latent CNN encoder,
6. multi-resolution xLSTM core,
7. regime-conditioned fusion,
8. quantile decoder,
9. trainer and evaluation stack,
10. conformal calibrator,
11. ensemble wrapper,
12. inference guards and serialization.

This order reduces integration risk.

---

# 30. Final Canonical Definition

The canonical model defined by this document is:

**a probabilistic, multi-horizon, multi-resolution, regime-aware recurrent forecasting system in which variable-selected observed features are transformed by a causal latent encoder, processed through parallel xLSTM branches at multiple temporal scales, fused according to inferred regime state, projected into future return quantiles, recalibrated through online conformal methods, and optionally aggregated through a small ensemble for robustness.**

In short form:

**VSN → Latent CNN → Multi-Resolution xLSTM → Regime-Conditioned Fusion → Quantile Decoder → Conformal Calibration → Ensemble**

---

# 31. Minimal Reference Pseudocode

```python
cfg = load_config()

feature_builder = FeatureBuilder(cfg)
normalizer = RollingNormalizer(cfg)
regime_estimator = RegimeEstimator(cfg)
model = ForecasterModel(cfg)
calibrator = ConformalCalibrator(cfg.calibration.alpha, cfg.calibration.window_size)

for raw_event in market_stream:
    feature_builder.update(raw_event)
    feats = feature_builder.current_features()
    feats_norm = normalizer.transform(feats)
    regime = regime_estimator.predict_proba(feats_norm)

    batch = build_inference_batch(
        feats_norm=feats_norm,
        regime=regime,
        cfg=cfg,
    )

    member_outputs = []
    for member in ensemble_members:
        out = member(
            batch.x_obs,
            batch.x_known,
            x_static=batch.x_static,
            regime_seq=batch.regime_seq,
            regime_cur=batch.regime_cur,
        )
        member_outputs.append(out["y_hat_q"])

    y_hat_q = aggregate(member_outputs)
    q_low, q_med, q_high = split_quantiles(y_hat_q)
    q_low_cal, q_high_cal = calibrator.calibrate(q_low, q_high)

    forecast = ForecastPacket(
        q_low=q_low_cal,
        q_med=q_med,
        q_high=q_high_cal,
        interval_width=q_high_cal - q_low_cal,
        regime=batch.regime_cur,
        confidence=compute_confidence(q_med, q_low_cal, q_high_cal),
        ensemble_variance=compute_ensemble_variance(member_outputs),
        diagnostics={},
    )

    publish_forecast(forecast)
```

---

# 32. Closing Implementation Note

This document specifies the architecture and interfaces, not a commitment to any particular framework or environment. The implementation should preserve:

- tensor contracts,
- module boundaries,
- causal semantics,
- calibration flow,
- serialization scope,
- walk-forward evaluation discipline.

As long as those are preserved, the exact engineering substrate can vary.
