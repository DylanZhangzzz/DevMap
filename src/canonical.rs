use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::DevMapError;

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, DevMapError> {
    let mut value = serde_json::to_value(value)?;
    ensure_no_floating_points(&value)?;
    // Default JSON maps are already sorted. This also preserves canonical key
    // order if a dependency enables preserve_order, without rebuilding the tree.
    value.sort_all_objects();
    Ok(serde_json::to_vec(&value)?)
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
