//! Bounded, client-owned immutable presentation pages. No storage or Git access.
use crate::{canonical::sha256_hex, dock::DockReadModel};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const RESULT_LIMIT: usize = 32768;
const SESSION_LIMIT: usize = 8 * 1024 * 1024;
const TOKEN_LIMIT: usize = 4096;
const PAGE_ITEMS: usize = 8;
const TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct SummaryState {
    session: Option<Session>,
}
struct Session {
    created: Instant,
    results: BTreeMap<String, Value>,
}
struct Builder {
    envelope: Value,
    results: BTreeMap<String, Value>,
    bytes: usize,
}

pub(crate) fn error(code: &'static str) -> Value {
    json!({"content":[{"type":"text","text":"Summary unavailable; start a new summary or request a smaller source view."}],
        "structuredContent":{"schema_version":"devmap-summary/1","error":{"code":code}},"isError":true})
}
fn result(value: Value) -> Result<Value, &'static str> {
    let result = json!({"content":[{"type":"text","text":"DevMap summary: captured observations; pages retain the same snapshot."}],"structuredContent":value,"isError":false});
    if serde_json::to_vec(&result)
        .map_err(|_| "summary_encoding")?
        .len()
        > RESULT_LIMIT
    {
        return Err("summary_item_too_large");
    }
    Ok(result)
}
fn nonce() -> Result<String, &'static str> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "summary_identity_unavailable")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn shortened(value: &str) -> (String, bool) {
    let mut end = value.len().min(256);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), end < value.len())
}
impl SummaryState {
    pub(crate) fn page(&mut self, cursor: &str) -> Value {
        if cursor.len() != 64 {
            return error("summary_cursor_invalid");
        }
        if self
            .session
            .as_ref()
            .is_some_and(|s| s.created.elapsed() >= TTL)
        {
            self.session = None;
        }
        self.session
            .as_ref()
            .and_then(|s| s.results.get(cursor))
            .cloned()
            .unwrap_or_else(|| error("summary_cursor_expired_or_invalid"))
    }
    pub(crate) fn capture(&mut self, model: &DockReadModel, observations: Value) -> Value {
        match build(model, observations) {
            Ok((initial, results)) => {
                self.session = Some(Session {
                    created: Instant::now(),
                    results,
                });
                initial
            }
            Err(code) => error(code),
        }
    }
}
impl Builder {
    fn page_result(&self, collection: &str, page: Value) -> Result<Value, &'static str> {
        let mut value = self.envelope.clone();
        value["pages"] = json!({collection:page});
        result(value)
    }
    fn retain(&mut self, token: String, result: Value) -> Result<(), &'static str> {
        let size = serde_json::to_vec(&result)
            .map_err(|_| "summary_encoding")?
            .len()
            + token.len();
        self.bytes = self
            .bytes
            .checked_add(size)
            .ok_or("summary_session_too_large")?;
        if self.bytes > SESSION_LIMIT || self.results.len() >= TOKEN_LIMIT {
            return Err("summary_session_too_large");
        }
        if self.results.insert(token, result).is_some() {
            return Err("summary_identity_collision");
        }
        Ok(())
    }
    fn detail(&mut self, value: Value) -> Result<String, &'static str> {
        let bytes = serde_json::to_string(&value).map_err(|_| "summary_encoding")?;
        let hash = sha256_hex(bytes.as_bytes());
        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let mut end = (offset + 4096).min(bytes.len());
            while !bytes.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push((nonce()?, offset, end));
            if chunks.len() > TOKEN_LIMIT {
                return Err("summary_session_too_large");
            }
            offset = end;
        }
        let first = chunks.first().ok_or("summary_encoding")?.0.clone();
        for (index, (token, start, end)) in chunks.iter().enumerate() {
            let value = self.page_result("details", json!({"encoding":"json-utf8","offset_bytes":start,"total_bytes":bytes.len(),"sha256":hash,
                "content":&bytes[*start..*end],"next_cursor":chunks.get(index+1).map(|c| &c.0),"incomplete":index+1<chunks.len()}))?;
            self.retain(token.clone(), value)?;
        }
        Ok(first)
    }
    fn collection(
        &mut self,
        name: &str,
        items: Vec<Value>,
    ) -> Result<(Value, Option<String>), &'static str> {
        if items.is_empty() {
            return Ok((
                json!({"total":0,"included":0,"items":[],"next_cursor":null,"incomplete":false}),
                None,
            ));
        }
        let total = items.len();
        let mut pages: Vec<(String, Vec<Value>)> = Vec::new();
        let mut offset = 0;
        while offset < total {
            let mut rows = Vec::new();
            for item in items.iter().skip(offset).take(PAGE_ITEMS) {
                rows.push(item.clone());
                let candidate = json!({"total":total,"included":rows.len(),"items":rows,"next_cursor":"0".repeat(64),"incomplete":true});
                if self.page_result(name, candidate).is_err() {
                    rows.pop();
                    break;
                }
            }
            if rows.is_empty() {
                return Err("summary_item_too_large");
            }
            offset += rows.len();
            pages.push((nonce()?, rows));
            if pages.len() > TOKEN_LIMIT {
                return Err("summary_session_too_large");
            }
        }
        let first_token = pages[0].0.clone();
        let mut first_page = Value::Null;
        for (index, (token, rows)) in pages.iter().enumerate() {
            let page = json!({"total":total,"included":rows.len(),"items":rows,"next_cursor":pages.get(index+1).map(|p| &p.0),"incomplete":index+1<pages.len()});
            if index == 0 {
                first_page = page.clone();
            }
            let value = self.page_result(name, page)?;
            self.retain(token.clone(), value)?;
        }
        Ok((first_page, Some(first_token)))
    }
}
fn build(
    model: &DockReadModel,
    observations: Value,
) -> Result<(Value, BTreeMap<String, Value>), &'static str> {
    // Bound the captured source before building a second representation.
    if serde_json::to_vec(model)
        .map_err(|_| "summary_encoding")?
        .len()
        > SESSION_LIMIT
    {
        return Err("summary_session_too_large");
    }
    let mut builder = Builder {
        envelope: json!({"schema_version":"devmap-summary/1","snapshot_id":nonce()?,
        "repository_id":model.repository_id,"current_worktree_id":model.current_worktree_id,
        "revision":model.revision,"observation_revision":model.observation_revision,"generated_at":model.generated_at,
        "source_truncated":model.truncated,"observations":observations,
        "counts":model.counts,"counts_scope":"captured_bounded_model","snapshot_kind":"retained_observations","expires_after_seconds":60,
        "execution":{"merge_ready":false,"checks_status":"unverified","execution_location_verified":false},"pages":{}}),
        results: BTreeMap::new(),
        bytes: 0,
    };
    result(builder.envelope.clone())?;
    let mut workspaces = Vec::new();
    let mut tasks = Vec::new();
    for lane in &model.lanes {
        let (path, truncated) = shortened(&lane.workspace_path);
        let detail = builder.detail(serde_json::to_value(lane).map_err(|_| "summary_encoding")?)?;
        workspaces.push(json!({"worktree_id":lane.worktree_id,"display_path":path,"label_truncated":truncated,
            "is_current":lane.is_current,"head":lane.head,"detail_cursor":detail,
            "git_status":{"status_observed":lane.relationship.status_observed,"dirty":lane.relationship.dirty,
                "changed_file_count":lane.relationship.changed_file_count,"ahead":lane.relationship.ahead,
                "behind":lane.relationship.behind,"merged":lane.relationship.merged}}));
        for chat in &lane.chats {
            let (title, truncated) = shortened(&chat.display_title);
            let detail =
                builder.detail(serde_json::to_value(chat).map_err(|_| "summary_encoding")?)?;
            tasks.push(json!({"id":chat.codex_thread_id.as_ref().unwrap_or(&chat.session_id),"worktree_id":lane.worktree_id,
                "title":title,"label_truncated":truncated,"status":chat.status,"capture_incomplete":chat.capture_incomplete,
                "status_source":chat.status_source,"confidence":chat.confidence,"capture_grade":chat.capture_grade,
                "lifecycle":chat.lifecycle,
                "detail_cursor":detail}));
        }
    }
    let warnings = model
        .warnings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "summary_encoding")?;
    let mut initial = builder.envelope.clone();
    for (name, items) in [
        ("workspaces", workspaces),
        ("tasks", tasks),
        ("warnings", warnings),
    ] {
        let total = items.len();
        let (page, first_token) = builder.collection(name, items)?;
        initial["pages"][name] = page;
        if result(initial.clone()).is_err() {
            initial["pages"][name] = json!({"total":total,"included":0,"items":[],"next_cursor":first_token,"incomplete":total>0});
            result(initial.clone())?;
        }
    }
    let initial = result(initial)?;
    if builder.bytes
        + serde_json::to_vec(&initial)
            .map_err(|_| "summary_encoding")?
            .len()
        > SESSION_LIMIT
    {
        return Err("summary_session_too_large");
    }
    Ok((initial, builder.results))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_result_counts_json_escaping_and_rejects_oversized_identifiers() {
        assert!(result(json!({"label":"🧭".repeat(1000)})).is_ok());
        assert!(result(json!({"label":"\"\\\n".repeat(6000)})).is_err());
        let mut builder = Builder {
            envelope: json!({"pages":{}}),
            results: BTreeMap::new(),
            bytes: 0,
        };
        assert!(
            builder
                .collection(
                    "warnings",
                    vec![json!({"code":"x".repeat(RESULT_LIMIT),"subject_id":null})]
                )
                .is_err()
        );
        let (label, truncated) = shortened(&"汉🧭".repeat(100));
        assert!(truncated && label.len() <= 256);
        assert!(std::str::from_utf8(label.as_bytes()).is_ok());
    }
    #[test]
    fn detail_chunks_reassemble_full_unicode_json_and_replay_exactly() {
        let mut builder = Builder {
            envelope: json!({"snapshot_id":"fixed","pages":{}}),
            results: BTreeMap::new(),
            bytes: 0,
        };
        let original = json!({"title":"汉🧭\"\\\n".repeat(4000),"id":"stable"});
        let first = builder.detail(original.clone()).unwrap();
        let mut state = SummaryState {
            session: Some(Session {
                created: Instant::now(),
                results: builder.results,
            }),
        };
        let mut cursor = first;
        let mut restored = String::new();
        let mut expected_hash = String::new();
        loop {
            let response = state.page(&cursor);
            assert_eq!(response, state.page(&cursor));
            assert!(serde_json::to_vec(&response).unwrap().len() <= RESULT_LIMIT);
            let detail = &response["structuredContent"]["pages"]["details"];
            assert_eq!(
                detail["offset_bytes"].as_u64().unwrap() as usize,
                restored.len()
            );
            if expected_hash.is_empty() {
                expected_hash = detail["sha256"].as_str().unwrap().to_owned();
            }
            assert_eq!(detail["sha256"], expected_hash);
            restored.push_str(detail["content"].as_str().unwrap());
            if let Some(next) = detail["next_cursor"].as_str() {
                cursor = next.to_owned();
            } else {
                assert_eq!(
                    detail["total_bytes"].as_u64().unwrap() as usize,
                    restored.len()
                );
                break;
            }
        }
        assert_eq!(sha256_hex(restored.as_bytes()), expected_hash);
        assert_eq!(serde_json::from_str::<Value>(&restored).unwrap(), original);
    }
    #[test]
    fn expired_session_discards_all_cursors_without_renewing_it() {
        let token = "a".repeat(64);
        let mut state = SummaryState {
            session: Some(Session {
                created: Instant::now() - Duration::from_secs(61),
                results: BTreeMap::from([(token.clone(), json!({"isError":false}))]),
            }),
        };
        assert_eq!(state.page(&token)["isError"], true);
        assert!(state.session.is_none());
        assert_eq!(state.page(&token)["isError"], true);
    }
}
