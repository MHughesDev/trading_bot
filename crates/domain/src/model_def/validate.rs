use super::{kinds::is_compatible, ModelDefinition};

const ACCEPTED_VERSIONS: &[&str] = &["1.0", "1.1"];

#[derive(Debug, PartialEq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

pub fn validate(def: &ModelDefinition) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if !ACCEPTED_VERSIONS.contains(&def.schema_version.as_str()) {
        errors.push(ValidationError::new(
            "schema_version",
            format!(
                "must be one of {:?}; got \"{}\"",
                ACCEPTED_VERSIONS, def.schema_version
            ),
        ));
    }

    if def.asset_class.trim().is_empty() {
        errors.push(ValidationError::new("asset_class", "must not be empty"));
    }

    if !is_compatible(def.model_kind, def.framework) {
        errors.push(ValidationError::new(
            "framework",
            format!(
                "{:?} is not compatible with kind {:?}",
                def.framework, def.model_kind
            ),
        ));
    }

    if def.model_kind.is_trainable() {
        if def.target.is_none() {
            errors.push(ValidationError::new(
                "target",
                "required for trainable model kinds",
            ));
        }
        if def.adapter.is_some() {
            errors.push(ValidationError::new(
                "adapter",
                "must be null for trainable model kinds",
            ));
        }
    } else {
        if def.target.is_some() {
            errors.push(ValidationError::new(
                "target",
                "must be null for external_llm_adapter kind",
            ));
        }
        if def.adapter.is_none() {
            errors.push(ValidationError::new(
                "adapter",
                "required for external_llm_adapter kind",
            ));
        }
    }

    // Walk-forward CV block (ADR-0017): optional, shape-only check here.
    if let Some(cv) = &def.cv {
        if let Err(cv_errs) = cv.validate_shape() {
            for e in cv_errs {
                errors.push(ValidationError::new(e.path, e.message));
            }
        }
    }

    // Distributional output block (ADR-0016, v1.1): optional; validate when present.
    if let Some(output) = &def.output {
        let levels = &output.quantile_levels;
        if levels.is_empty() {
            errors.push(ValidationError::new(
                "output.quantile_levels",
                "must not be empty",
            ));
        } else {
            for (i, &l) in levels.iter().enumerate() {
                if l <= 0.0 || l >= 1.0 {
                    errors.push(ValidationError::new(
                        "output.quantile_levels",
                        format!("level[{i}]={l} not in (0, 1)"),
                    ));
                    break;
                }
                if i > 0 && l <= levels[i - 1] {
                    errors.push(ValidationError::new(
                        "output.quantile_levels",
                        format!("levels not strictly increasing at index {i}"),
                    ));
                    break;
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
