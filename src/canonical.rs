use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::error::DevMapError;

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, DevMapError> {
    let value = serde_json::to_value(value)?;
    ensure_no_floating_points(&value)?;
    let normalized = normalize(value)?;
    Ok(serde_json::to_vec(&normalized)?)
}

/// Rejects values that cannot have a stable JSON canonical representation.
pub fn ensure_no_floating_points(value: &Value) -> Result<(), DevMapError> {
    match value {
        Value::Array(values) => {
            for value in values {
                ensure_no_floating_points(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                ensure_no_floating_points(value)?;
            }
        }
        Value::Number(number) if !number.is_i64() && !number.is_u64() => {
            return Err(DevMapError::FloatingPointNotCanonical);
        }
        _ => {}
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn content_id(kind: &str, bytes: &[u8]) -> String {
    format!("{kind}:sha256-{}", sha256_hex(bytes))
}

fn normalize(value: Value) -> Result<Value, DevMapError> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(normalize)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => {
            let mut entries: Vec<_> = values.into_iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));

            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key, normalize(value)?);
            }
            Ok(Value::Object(normalized))
        }
        Value::Number(_) => Ok(value),
        scalar => Ok(scalar),
    }
}
