//! A small, declarative native UI catalog. Agent JSON is data: it cannot name
//! code, URLs, tools, or commands. Only an explicit action click prepares a
//! normal chat prompt containing the user's current form values.

mod state;
pub(crate) use state::{Input, State, bounded_input};

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, rc::Rc};

pub(crate) const MAX_SOURCE: usize = 32 * 1024;
pub(crate) const MAX_INPUT: usize = 2048;
const MAX_COMPONENTS: usize = 32;
const MAX_FIELDS: usize = 16;
const MAX_PROMPT: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("the interactive card exceeds its size limit")]
    TooLarge,
    #[error("invalid interactive card JSON")]
    Json(#[from] serde_json::Error),
    #[error("invalid interactive card: {0}")]
    Invalid(&'static str),
}

#[derive(Debug)]
pub(crate) struct Document {
    pub(crate) title: String,
    pub(crate) components: Vec<Component>,
    pub(crate) source: Rc<str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    #[serde(default)]
    title: String,
    components: Vec<Component>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Component {
    Action {
        label: String,
        prompt: String,
        #[serde(default)]
        mode: ActionMode,
    },
    Card {
        title: String,
        text: String,
    },
    Chart {
        label: String,
        points: Vec<Point>,
    },
    Choice {
        id: String,
        label: String,
        options: Vec<String>,
        #[serde(default)]
        value: String,
    },
    Input {
        id: String,
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        value: String,
    },
    Metric {
        label: String,
        value: String,
        #[serde(default)]
        detail: String,
    },
    Slider {
        id: String,
        label: String,
        min: f64,
        max: f64,
        #[serde(default = "one")]
        step: f64,
        value: f64,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Text {
        text: String,
    },
    Toggle {
        id: String,
        label: String,
        #[serde(default)]
        value: bool,
    },
}

fn one() -> f64 {
    1.
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionMode {
    #[default]
    Draft,
    Send,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    pub(crate) label: String,
    pub(crate) value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Value {
    Text(String),
    Number(f64),
    Toggle(bool),
}

pub(crate) type Values = BTreeMap<String, Value>;

fn text(value: &str, max: usize) -> bool {
    value.len() <= max
        && !value
            .chars()
            .any(|ch| crate::notifications::unsafe_char(ch) && !matches!(ch, '\n' | '\t'))
}

impl Component {
    pub(crate) fn field(&self) -> Option<(&str, Value)> {
        match self {
            Self::Choice { id, value, .. } | Self::Input { id, value, .. } => {
                Some((id, Value::Text(value.clone())))
            }
            Self::Slider { id, value, .. } => Some((id, Value::Number(*value))),
            Self::Toggle { id, value, .. } => Some((id, Value::Toggle(*value))),
            _ => None,
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Action { label, prompt, .. } => {
                text(label, 160)
                    && !label.is_empty()
                    && text(prompt, 4096)
                    && !prompt.trim().is_empty()
            }
            Self::Card { title, text: body } => text(title, 160) && text(body, 8192),
            Self::Text { text: body } => text(body, 8192),
            Self::Metric {
                label,
                value,
                detail,
            } => text(label, 160) && text(value, 128) && text(detail, 512),
            Self::Choice {
                label,
                options,
                value,
                ..
            } => {
                text(label, 160)
                    && (1..=12).contains(&options.len())
                    && options
                        .iter()
                        .all(|option| !option.is_empty() && text(option, 160))
                    && options
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == options.len()
                    && options.contains(value)
            }
            Self::Toggle { label, .. } => text(label, 160),
            Self::Input {
                label,
                placeholder,
                value,
                ..
            } => {
                text(label, 160)
                    && text(placeholder, 160)
                    && text(value, MAX_INPUT)
                    && !value.contains('\n')
            }
            Self::Slider {
                label,
                min,
                max,
                step,
                value,
                ..
            } => {
                text(label, 160)
                    && [min, max, step, value]
                        .iter()
                        .all(|n| n.is_finite() && n.abs() <= 1e9)
                    && max > min
                    && *step > 0.
                    && *step <= max - min
                    && (max - min) / step <= 10000.
                    && (min..=max).contains(&value)
            }
            Self::Table { columns, rows } => {
                (1..=8).contains(&columns.len())
                    && rows.len() <= 32
                    && columns.iter().all(|cell| text(cell, 160))
                    && rows.iter().all(|row| {
                        row.len() == columns.len() && row.iter().all(|cell| text(cell, 512))
                    })
            }
            Self::Chart { label, points } => {
                text(label, 160)
                    && (1..=32).contains(&points.len())
                    && points.iter().all(|point| {
                        text(&point.label, 160)
                            && point.value.is_finite()
                            && (0. ..=1e9).contains(&point.value)
                    })
            }
        }
    }

    pub(crate) fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Choice { options, .. }, Value::Text(value)) => options.contains(value),
            (Self::Input { .. }, Value::Text(value)) => {
                text(value, MAX_INPUT) && !value.contains('\n')
            }
            (Self::Toggle { .. }, Value::Toggle(_)) => true,
            (Self::Slider { min, max, .. }, Value::Number(value)) => {
                value.is_finite() && (min..=max).contains(&value)
            }
            _ => false,
        }
    }
}

impl Document {
    pub(crate) fn parse(source: &str) -> Result<Self, Error> {
        if source.len() > MAX_SOURCE {
            return Err(Error::TooLarge);
        }
        let mut spec: Spec = serde_json::from_str(source)?;
        if !text(&spec.title, 160) || !(1..=MAX_COMPONENTS).contains(&spec.components.len()) {
            return Err(Error::Invalid("title or component count"));
        }
        let mut fields = std::collections::HashSet::new();
        for component in &mut spec.components {
            if let Component::Choice { options, value, .. } = component
                && value.is_empty()
                && let Some(first) = options.first()
            {
                *value = first.clone();
            }
            if !component.valid() {
                return Err(Error::Invalid("component fields or bounds"));
            }
            if let Some((id, _)) = component.field()
                && (id.is_empty()
                    || id.len() > 64
                    || !id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                    || !fields.insert(id.to_owned()))
            {
                return Err(Error::Invalid("field ids must be unique ASCII names"));
            }
        }
        if fields.len() > MAX_FIELDS {
            return Err(Error::Invalid("too many editable fields"));
        }
        Ok(Self {
            title: spec.title,
            components: spec.components,
            source: source.into(),
        })
    }

    pub(crate) fn defaults(&self) -> Values {
        self.components
            .iter()
            .filter_map(Component::field)
            .map(|(id, value)| (id.to_owned(), value))
            .collect()
    }

    pub(crate) fn set(&self, values: &mut Values, id: &str, value: Value) -> bool {
        let valid = self.components.iter().any(|component| {
            component.field().is_some_and(|(field, _)| field == id) && component.accepts(&value)
        });
        if valid {
            values.insert(id.to_owned(), value);
        }
        valid
    }

    /// Actions include exactly this catalog's fields, never arbitrary keys a
    /// stale event or caller supplied. JSON preserves literal user text.
    pub(crate) fn prompt(&self, prefix: &str, values: &Values) -> Result<String, Error> {
        let mut checked = self.defaults();
        for (id, value) in values {
            if !self.set(&mut checked, id, value.clone()) {
                return Err(Error::Invalid("form value"));
            }
        }
        if !self.components.iter().any(
            |component| matches!(component, Component::Action { prompt, .. } if prompt == prefix),
        ) {
            return Err(Error::Invalid("action is no longer present"));
        }
        let prompt = if checked.is_empty() {
            prefix.to_owned()
        } else {
            format!(
                "{prefix}\n\nSelected values:\n{}",
                serde_json::to_string_pretty(&checked)?
            )
        };
        if prompt.len() > MAX_PROMPT {
            return Err(Error::TooLarge);
        }
        Ok(prompt)
    }
}

/// Slider gestures are local state only; the value leaves the view solely
/// through an action click. Clamping also handles the last fractional step.
pub(crate) fn slider_value(fraction: f64, min: f64, max: f64, step: f64) -> f64 {
    if fraction <= 0. {
        return min;
    }
    if fraction >= 1. {
        return max;
    }
    (min + ((fraction.clamp(0., 1.) * (max - min)) / step).round() * step).clamp(min, max)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
