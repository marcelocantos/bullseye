// Copyright 2026 Marcelo Cantos
// SPDX-License-Identifier: Apache-2.0

//! Planner preferences, stored by Bullseye and interpreted by dispatchers (🎯T107).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::api::{CodedError, ErrorCode};
use crate::schema::TargetsFile;

/// Minimum capability, ordered independently of any provider's model names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelClass {
    Economy,
    Standard,
    Frontier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEffort {
    Low,
    Medium,
    High,
    Max,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPreference {
    pub class: ModelClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ModelEffort>,
    /// Provider key → exact model id or dispatcher-supported alias. Never executed here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execute: Option<ModelPreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ModelPreference>,
}

impl ModelPreferences {
    pub fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.execute.is_none() && self.review.is_none() {
            errors.push("models must specify execute or review; clear models to remove it".into());
        }
        for (role, preference) in [("execute", &self.execute), ("review", &self.review)] {
            if let Some(preference) = preference {
                for (provider, pin) in &preference.pins {
                    if provider.is_empty()
                        || !provider.bytes().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_'
                        })
                        || pin.trim().is_empty()
                        || pin.trim() != pin
                        || pin.chars().any(char::is_control)
                    {
                        errors.push(format!("models.{role}.pins requires a lowercase provider key and a non-empty, trimmed model id or alias"));
                    }
                }
            }
        }
        errors
    }

    /// Unspecified execution preferences can be worked by any class. Review does not gate execution.
    pub fn accepts(&self, available: ModelClass) -> bool {
        self.execute.as_ref().is_none_or(|p| available >= p.class)
    }
}

pub fn require_adoption(
    file: &TargetsFile,
    requested: bool,
    adopted: bool,
) -> Result<(), CodedError> {
    if requested && !adopted && !file.targets.values().any(|t| t.models.is_some()) {
        return Err(CodedError::new(
            ErrorCode::Validation,
            format!(
                "refusing the first models block: this stamps schema_version {}, so older Bullseye consumers refuse the whole ledger. Upgrade all consumers, then acknowledge with adopt_model_preferences: true (or --adopt-model-preferences).",
                crate::schema::MODEL_PREFERENCES_SCHEMA_VERSION
            ),
        ));
    }
    Ok(())
}

pub fn render_preferences(out: &mut String, models: Option<&ModelPreferences>) {
    if let Some(models) = models {
        // JSON keeps the role, class, effort and pins together even in a compact text view.
        out.push_str(&format!(
            "  models: {}\n",
            serde_json::to_string(models).expect("preferences serialize")
        ));
    }
}

/// Explicit representation migration, including immutable historical rows. No lifecycle or prose changes.
/// Unknown or conflicting preferences refuse the entire migration before any row changes.
pub fn migrate_tags(file: &mut TargetsFile, adopted: bool) -> Result<Vec<String>, CodedError> {
    let mut migrated = file.clone();
    let mut changed = Vec::new();
    for (id, target) in &mut migrated.targets {
        let mut preferences = target.models.clone().unwrap_or_default();
        let mut retained = Vec::new();
        let mut found = false;
        for tag in &target.tags {
            let (role, family) = if let Some(family) = tag.strip_prefix("model:") {
                (&mut preferences.execute, family)
            } else if let Some(family) = tag.strip_prefix("review:") {
                (&mut preferences.review, family)
            } else {
                retained.push(tag.clone());
                continue;
            };
            let class = match family {
                "haiku" => ModelClass::Economy,
                "sonnet" => ModelClass::Standard,
                "opus" | "fable" => ModelClass::Frontier,
                _ => {
                    return Err(CodedError::new(
                        ErrorCode::Validation,
                        format!(
                            "🎯{id}: unknown legacy model family in {tag:?}; migration made no changes"
                        ),
                    ));
                }
            };
            let preference = ModelPreference {
                class,
                effort: None,
                pins: BTreeMap::from([("claude".into(), family.into())]),
            };
            if role
                .as_ref()
                .is_some_and(|existing| existing != &preference)
            {
                return Err(CodedError::new(
                    ErrorCode::Validation,
                    format!(
                        "🎯{id}: conflicting preference for {tag:?}; migration made no changes"
                    ),
                ));
            }
            *role = Some(preference);
            found = true;
        }
        if found {
            target.models = Some(preferences);
            target.tags = retained;
            changed.push(id.clone());
        }
    }
    require_adoption(file, !changed.is_empty(), adopted)?;
    *file = migrated;
    Ok(changed)
}
