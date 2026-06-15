use super::{kinds::is_compatible, ModelDefinition, DEFINITION_VERSION};

#[derive(Debug, PartialEq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { path: path.into(), message: message.into() }
    }
}

pub fn validate(def: &ModelDefinition) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if def.schema_version != DEFINITION_VERSION {
        errors.push(ValidationError::new(
            "schema_version",
            format!("must be \"{DEFINITION_VERSION}\"; got \"{}\"", def.schema_version),
        ));
    }

    if def.asset_class.trim().is_empty() {
        errors.push(ValidationError::new("asset_class", "must not be empty"));
    }

    if !is_compatible(def.model_kind, def.framework) {
        errors.push(ValidationError::new(
            "framework",
            format!("{:?} is not compatible with kind {:?}", def.framework, def.model_kind),
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

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}
