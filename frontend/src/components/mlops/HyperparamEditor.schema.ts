import type { ParamDesc } from './HyperparamEditor'

export function schemaFor(framework: string, values: Record<string, unknown>): ParamDesc[] {
  switch (framework) {
    case 'xgboost':
      return XGBOOST_PARAMS
    case 'lightgbm':
      return LIGHTGBM_PARAMS
    case 'torch':
      return TORCH_PARAMS
    case 'sklearn':
      return sklearnParams(String(values.estimator ?? SKLEARN_ESTIMATOR.default))
    default:
      return []
  }
}

const XGBOOST_PARAMS: ParamDesc[] = [
  { key: 'n_estimators', label: 'Boosting rounds', type: 'int', default: 200, min: 10, max: 2000, step: 10, help: 'Number of trees / boosting iterations.' },
  { key: 'max_depth', label: 'Max depth', type: 'int', default: 6, min: 1, max: 20, step: 1, help: 'Maximum tree depth. Higher = more capacity, more overfit risk.' },
  { key: 'learning_rate', label: 'Learning rate (eta)', type: 'float', default: 0.05, min: 0.001, max: 0.5, step: 0.001 },
  { key: 'subsample', label: 'Row subsample', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05, help: 'Fraction of rows sampled per tree.' },
  { key: 'colsample_bytree', label: 'Column subsample', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'min_child_weight', label: 'Min child weight', type: 'float', default: 1.0, min: 0, max: 20, step: 0.5 },
  { key: 'gamma', label: 'Min split loss (gamma)', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_alpha', label: 'L1 reg (alpha)', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_lambda', label: 'L2 reg (lambda)', type: 'float', default: 1.0, min: 0, max: 10, step: 0.1 },
  { key: 'early_stopping_rounds', label: 'Early stopping rounds', type: 'int', default: 30, min: 0, max: 200, step: 5, help: 'Stop if val metric stalls for N rounds. 0 disables.' },
]

const LIGHTGBM_PARAMS: ParamDesc[] = [
  { key: 'n_estimators', label: 'Boosting rounds', type: 'int', default: 200, min: 10, max: 2000, step: 10 },
  { key: 'max_depth', label: 'Max depth', type: 'int', default: -1, min: -1, max: 20, step: 1, help: '-1 means no limit.' },
  { key: 'learning_rate', label: 'Learning rate', type: 'float', default: 0.05, min: 0.001, max: 0.5, step: 0.001 },
  { key: 'num_leaves', label: 'Num leaves', type: 'int', default: 31, min: 2, max: 256, step: 1, help: 'Main complexity knob for LightGBM.' },
  { key: 'feature_fraction', label: 'Feature fraction', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'bagging_fraction', label: 'Bagging fraction', type: 'float', default: 1.0, min: 0.1, max: 1.0, step: 0.05 },
  { key: 'min_child_samples', label: 'Min child samples', type: 'int', default: 20, min: 1, max: 200, step: 1 },
  { key: 'reg_alpha', label: 'L1 reg', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'reg_lambda', label: 'L2 reg', type: 'float', default: 0.0, min: 0, max: 10, step: 0.1 },
  { key: 'early_stopping_rounds', label: 'Early stopping rounds', type: 'int', default: 30, min: 0, max: 200, step: 5, help: 'Stop if val metric stalls for N rounds. 0 disables.' },
]

const TORCH_PARAMS: ParamDesc[] = [
  { key: 'arch', label: 'Architecture', type: 'select', default: 'mlp', options: ['mlp', 'lstm'], help: 'MLP feed-forward or single-step LSTM.' },
  { key: 'epochs', label: 'Epochs', type: 'int', default: 50, min: 1, max: 500, step: 1 },
  { key: 'learning_rate', label: 'Learning rate', type: 'float', default: 0.001, min: 0.0001, max: 0.1, step: 0.0001 },
  { key: 'batch_size', label: 'Batch size', type: 'int', default: 64, min: 8, max: 1024, step: 8 },
  { key: 'hidden_dim', label: 'Hidden units', type: 'int', default: 32, min: 8, max: 256, step: 8 },
  { key: 'weight_decay', label: 'Weight decay', type: 'float', default: 0.0, min: 0, max: 0.1, step: 0.001, help: 'L2 regularization on weights.' },
]

// sklearn passes kwargs straight to the chosen estimator — a kwarg the
// estimator rejects makes the whole construction fall back to defaults, so
// only surface params valid for the selected estimator.
const SKLEARN_ESTIMATOR: ParamDesc = {
  key: 'estimator',
  label: 'Estimator',
  type: 'select',
  default: 'RandomForestClassifier',
  options: ['RandomForestClassifier', 'GradientBoostingClassifier', 'LogisticRegression'],
}

export function sklearnParams(estimator: string): ParamDesc[] {
  if (estimator === 'LogisticRegression') {
    return [
      SKLEARN_ESTIMATOR,
      { key: 'C', label: 'Inverse reg (C)', type: 'float', default: 1.0, min: 0.01, max: 100, step: 0.01 },
      { key: 'max_iter', label: 'Max iterations', type: 'int', default: 100, min: 50, max: 2000, step: 50 },
    ]
  }
  // Random forest / gradient boosting both accept n_estimators + max_depth.
  return [
    SKLEARN_ESTIMATOR,
    { key: 'n_estimators', label: 'Estimators', type: 'int', default: 100, min: 10, max: 1000, step: 10 },
    { key: 'max_depth', label: 'Max depth', type: 'int', default: 6, min: 1, max: 30, step: 1 },
  ]
}
