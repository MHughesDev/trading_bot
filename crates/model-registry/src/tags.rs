//! Registry tags, annotations & spec templates (I-6.4, Phase 6).
//!
//! Tags:        free-form labels on models/ensembles/pipelines; searchable.
//! Annotations: key-value metadata attached to any artifact.
//! Templates:   forkable starter specs (`spec_templates` table); feed Create wizards.
//!
//! Postgres tables (runtime sqlx, no compile-time macros):
//!   `artifact_tags`   (`artifact_id` TEXT, `artifact_kind` TEXT, tag TEXT, `created_at`)
//!   `artifact_annots` (`artifact_id` TEXT, `artifact_kind` TEXT, key TEXT, value JSONB, `updated_at`)
//!   `spec_templates`  (id TEXT PK, name TEXT, kind TEXT, description TEXT,
//!                    `definition_json` JSONB, `created_by` TEXT, `created_at`)

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

// ── Public record types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArtifactTag {
    pub artifact_id: String,
    pub artifact_kind: String,
    pub tag: String,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArtifactAnnotation {
    pub artifact_id: String,
    pub artifact_kind: String,
    pub key: String,
    pub value: Value,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecTemplate {
    pub id: String,
    pub name: String,
    /// `"model"` | `"ensemble"` | `"pipeline"`
    pub kind: String,
    pub description: String,
    pub definition: Value,
    pub created_by: String,
    pub created_at: chrono::DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub kind: String,
    pub description: String,
    pub definition: Value,
}

// ── TagRegistry ───────────────────────────────────────────────────────────────

pub struct TagRegistry {
    pg: PgPool,
}

impl TagRegistry {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    // ── Tags ─────────────────────────────────────────────────────────────────

    pub async fn add_tag(
        &self,
        artifact_id: &str,
        artifact_kind: &str,
        tag: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO artifact_tags (artifact_id, artifact_kind, tag, created_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT DO NOTHING",
        )
        .bind(artifact_id)
        .bind(artifact_kind)
        .bind(tag)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn remove_tag(
        &self,
        artifact_id: &str,
        artifact_kind: &str,
        tag: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "DELETE FROM artifact_tags \
             WHERE artifact_id = $1 AND artifact_kind = $2 AND tag = $3",
        )
        .bind(artifact_id)
        .bind(artifact_kind)
        .bind(tag)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn list_tags(
        &self,
        artifact_id: &str,
        artifact_kind: &str,
    ) -> anyhow::Result<Vec<ArtifactTag>> {
        let rows: Vec<(String, String, String, chrono::DateTime<Utc>)> = sqlx::query_as(
            "SELECT artifact_id, artifact_kind, tag, created_at \
             FROM artifact_tags \
             WHERE artifact_id = $1 AND artifact_kind = $2 \
             ORDER BY tag",
        )
        .bind(artifact_id)
        .bind(artifact_kind)
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(
                |(artifact_id, artifact_kind, tag, created_at)| ArtifactTag {
                    artifact_id,
                    artifact_kind,
                    tag,
                    created_at,
                },
            )
            .collect())
    }

    /// Search artifacts by tag within a kind.
    pub async fn search_by_tag(
        &self,
        artifact_kind: &str,
        tag: &str,
    ) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT artifact_id FROM artifact_tags \
             WHERE artifact_kind = $1 AND tag ILIKE $2 \
             ORDER BY created_at DESC LIMIT 200",
        )
        .bind(artifact_kind)
        .bind(format!("%{tag}%"))
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    // ── Annotations ──────────────────────────────────────────────────────────

    pub async fn set_annotation(
        &self,
        artifact_id: &str,
        artifact_kind: &str,
        key: &str,
        value: Value,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO artifact_annots (artifact_id, artifact_kind, key, value, updated_at) \
             VALUES ($1, $2, $3, $4, NOW()) \
             ON CONFLICT (artifact_id, artifact_kind, key) DO UPDATE \
               SET value = EXCLUDED.value, updated_at = NOW()",
        )
        .bind(artifact_id)
        .bind(artifact_kind)
        .bind(key)
        .bind(&value)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    pub async fn get_annotations(
        &self,
        artifact_id: &str,
        artifact_kind: &str,
    ) -> anyhow::Result<Vec<ArtifactAnnotation>> {
        let rows: Vec<(String, String, String, Value, chrono::DateTime<Utc>)> = sqlx::query_as(
            "SELECT artifact_id, artifact_kind, key, value, updated_at \
             FROM artifact_annots \
             WHERE artifact_id = $1 AND artifact_kind = $2 \
             ORDER BY key",
        )
        .bind(artifact_id)
        .bind(artifact_kind)
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(
                |(artifact_id, artifact_kind, key, value, updated_at)| ArtifactAnnotation {
                    artifact_id,
                    artifact_kind,
                    key,
                    value,
                    updated_at,
                },
            )
            .collect())
    }

    // ── Templates ─────────────────────────────────────────────────────────────

    pub async fn create_template(
        &self,
        req: CreateTemplateRequest,
        user_id: &str,
    ) -> anyhow::Result<SpecTemplate> {
        let id = format!("tmpl_{}", Uuid::new_v4().simple());
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO spec_templates \
               (id, name, kind, description, definition_json, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&id)
        .bind(&req.name)
        .bind(&req.kind)
        .bind(&req.description)
        .bind(&req.definition)
        .bind(user_id)
        .bind(now)
        .execute(&self.pg)
        .await?;

        Ok(SpecTemplate {
            id,
            name: req.name,
            kind: req.kind,
            description: req.description,
            definition: req.definition,
            created_by: user_id.to_string(),
            created_at: now,
        })
    }

    pub async fn list_templates(&self, kind: Option<&str>) -> anyhow::Result<Vec<SpecTemplate>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            Value,
            String,
            chrono::DateTime<Utc>,
        )> = if let Some(k) = kind {
            sqlx::query_as(
                "SELECT id, name, kind, description, definition_json, created_by, created_at \
                     FROM spec_templates WHERE kind = $1 ORDER BY created_at DESC",
            )
            .bind(k)
            .fetch_all(&self.pg)
            .await
        } else {
            sqlx::query_as(
                "SELECT id, name, kind, description, definition_json, created_by, created_at \
                     FROM spec_templates ORDER BY created_at DESC",
            )
            .fetch_all(&self.pg)
            .await
        }
        .unwrap_or_default();

        Ok(rows
            .into_iter()
            .map(
                |(id, name, kind, description, def, created_by, created_at)| SpecTemplate {
                    id,
                    name,
                    kind,
                    description,
                    definition: def,
                    created_by,
                    created_at,
                },
            )
            .collect())
    }

    /// Fork a template: return the definition so the caller can create a new artifact.
    pub async fn fork_template(&self, template_id: &str) -> anyhow::Result<Value> {
        let row: Option<(Value,)> =
            sqlx::query_as("SELECT definition_json FROM spec_templates WHERE id = $1")
                .bind(template_id)
                .fetch_optional(&self.pg)
                .await?;

        let (def,) = row.ok_or_else(|| anyhow::anyhow!("template not found: {template_id}"))?;
        Ok(def)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_template_request_deserialises() {
        let json = serde_json::json!({
            "name": "LightGBM quantile starter",
            "kind": "model",
            "description": "A ready-to-fork LightGBM quantile forecaster",
            "definition": { "framework": "lightgbm", "model_kind": "forecaster" }
        });
        let req: CreateTemplateRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.kind, "model");
        assert_eq!(req.name, "LightGBM quantile starter");
    }

    #[test]
    fn spec_template_round_trips() {
        let tmpl = SpecTemplate {
            id: "tmpl_abc".into(),
            name: "test".into(),
            kind: "pipeline".into(),
            description: "A test template".into(),
            definition: serde_json::json!({ "dag": [] }),
            created_by: "user1".into(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&tmpl).unwrap();
        let tmpl2: SpecTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(tmpl.id, tmpl2.id);
        assert_eq!(tmpl.kind, tmpl2.kind);
    }
}
