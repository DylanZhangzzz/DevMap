//! Immutable SQL identity evidence. No filesystem discovery or repair.
use super::migration::FrozenOrigin;
use crate::{
    error::DevMapError,
    journal::{BindingSnapshot, TaskBindingObservation, binding_id},
    route_plan::{Record, RoutePlan},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Qualification {
    NativeVerified,
    FrozenBaseline,
    Unknown,
}
impl Qualification {
    fn text(self) -> &'static str {
        match self {
            Self::NativeVerified => "native_verified",
            Self::FrozenBaseline => "frozen_baseline",
            Self::Unknown => "unknown",
        }
    }
    fn parse(value: &str) -> Result<Self, DevMapError> {
        match value {
            "native_verified" => Ok(Self::NativeVerified),
            "frozen_baseline" => Ok(Self::FrozenBaseline),
            "unknown" => Ok(Self::Unknown),
            _ => Err(fail("unknown origin qualification")),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginLink {
    pub worktree_id: String,
    pub incarnation: Option<String>,
    pub qualification: Qualification,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingLink {
    pub destination: OriginLink,
    pub source: Option<OriginLink>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingCursor {
    pub observed_at: String,
    pub current: Option<OriginLink>,
    pub history_observation_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BindingMetadata {
    pub links: BTreeMap<String, BindingLink>,
    pub cursors: BTreeMap<(String, String), BindingCursor>,
}
fn fail(message: &str) -> DevMapError {
    DevMapError::Store(message.into())
}
fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn stamp(value: &str) -> Result<time::OffsetDateTime, DevMapError> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(|_| fail("invalid origin cursor timestamp"))
}

pub(crate) fn validate_schema(c: &Connection) -> Result<(), DevMapError> {
    for sql in [
        "SELECT route_id,revision,worktree_id,incarnation,qualification FROM route_origin_links LIMIT 0",
        "SELECT observation_id,destination_worktree_id,destination_incarnation,destination_qualification,source_worktree_id,source_incarnation,source_qualification FROM binding_origin_links LIMIT 0",
        "SELECT source_scope,observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id FROM binding_origin_cursors LIMIT 0",
    ] {
        c.prepare(sql)?;
    }
    Ok(())
}

// Check storage types and lengths before allocating any SQL metadata strings.
fn table_bounds(
    c: &Connection,
    table: &str,
    fields: &[(&str, usize)],
    max_rows: usize,
) -> Result<(), DevMapError> {
    let count: i64 = c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
    if count < 0 || count as u64 > max_rows as u64 {
        return Err(fail("origin metadata coverage limit"));
    }
    let condition=fields.iter().map(|(field,max)|format!("({field} IS NOT NULL AND (typeof({field})!='text' OR length(CAST({field} AS BLOB))>{max}))")).collect::<Vec<_>>().join(" OR ");
    let bad: i64 = c.query_row(
        &format!("SELECT count(*) FROM {table} WHERE {condition}"),
        [],
        |r| r.get(0),
    )?;
    if bad != 0 {
        return Err(fail("origin metadata field limit"));
    }
    Ok(())
}
fn check_link(c: &Connection, link: &OriginLink) -> Result<(), DevMapError> {
    if !bounded(&link.worktree_id, 256)
        || link.incarnation.as_ref().is_some_and(|v| !bounded(v, 512))
        || (link.qualification == Qualification::Unknown) != link.incarnation.is_none()
    {
        return Err(fail("invalid origin link identity"));
    }
    if let Some(incarnation) = &link.incarnation {
        registered_origin(c, &link.worktree_id, incarnation)?;
    }
    Ok(())
}
/// Static persisted evidence only; retirement is returned for live qualification.
/// A historical link to a retired origin remains valid immutable history.
pub(crate) fn registered_origin(
    c: &Connection,
    worktree_id: &str,
    incarnation: &str,
) -> Result<(FrozenOrigin, Option<String>), DevMapError> {
    if !bounded(worktree_id, 256) || !bounded(incarnation, 512) {
        return Err(fail("invalid registry identity"));
    }
    let saved:Option<(String,String,Option<String>)>=c.query_row("SELECT CASE WHEN length(CAST(git_dir AS BLOB)) BETWEEN 1 AND 32768 THEN git_dir ELSE 0 END,CASE WHEN length(CAST(workspace_path AS BLOB)) BETWEEN 1 AND 32768 THEN workspace_path ELSE 0 END,CASE WHEN retired_at IS NULL OR length(CAST(retired_at AS BLOB)) BETWEEN 1 AND 128 THEN retired_at ELSE 0 END FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",params![worktree_id,incarnation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (git, root, retired) =
        saved.ok_or_else(|| fail("origin link registry reference missing"))?;
    let git_dir = std::path::PathBuf::from(&git);
    let workspace_path = std::path::PathBuf::from(&root);
    let repository:String=c.query_row("SELECT CASE WHEN length(CAST(repository_id AS BLOB)) BETWEEN 1 AND 256 THEN repository_id ELSE 0 END FROM store_meta WHERE singleton=1",[],|r|r.get(0))?;
    // Filesystem paths can legally contain newlines on Unix. Their byte bounds
    // and absolute spelling are distinct from identifier text validation;
    // native acceptance separately checks canonical reciprocal Git identity.
    if git.is_empty()
        || git.len() > 32768
        || root.is_empty()
        || root.len() > 32768
        || !git_dir.is_absolute()
        || !workspace_path.is_absolute()
        || crate::worktrees::origin_id(&repository, &git_dir) != worktree_id
    {
        return Err(fail("origin registry path or derived ID mismatch"));
    }
    if let Some(at) = &retired {
        stamp(at)?;
    }
    Ok((
        FrozenOrigin {
            worktree_id: worktree_id.into(),
            incarnation: incarnation.into(),
            git_dir,
            workspace_path,
        },
        retired,
    ))
}
fn decode(
    c: &Connection,
    id: String,
    incarnation: Option<String>,
    qualification: String,
) -> Result<OriginLink, DevMapError> {
    let link = OriginLink {
        worktree_id: id,
        incarnation,
        qualification: Qualification::parse(&qualification)?,
    };
    check_link(c, &link)?;
    Ok(link)
}
fn optional(
    c: &Connection,
    id: Option<String>,
    incarnation: Option<String>,
    qualification: String,
    absent: &str,
) -> Result<Option<OriginLink>, DevMapError> {
    match id {
        Some(id) => Ok(Some(decode(c, id, incarnation, qualification)?)),
        None if incarnation.is_none() && qualification == absent => Ok(None),
        _ => Err(fail("invalid absent origin link")),
    }
}
pub(crate) fn register_origin(
    c: &Connection,
    origin: &FrozenOrigin,
) -> Result<OriginLink, DevMapError> {
    let link = OriginLink {
        worktree_id: origin.worktree_id.clone(),
        incarnation: Some(origin.incarnation.clone()),
        qualification: Qualification::NativeVerified,
    };
    if !bounded(&link.worktree_id, 256) || !bounded(&origin.incarnation, 512) {
        return Err(fail("invalid registered origin identity"));
    }
    let exists: i64 = c.query_row(
        "SELECT count(*) FROM worktree_registry WHERE worktree_id=?1 AND incarnation=?2",
        params![origin.worktree_id, origin.incarnation],
        |r| r.get(0),
    )?;
    if exists != 0 {
        let (saved, retired) = registered_origin(c, &origin.worktree_id, &origin.incarnation)?;
        if saved != *origin || retired.is_some() {
            return Err(fail("origin registry mismatch or retired incarnation"));
        }
    } else {
        c.execute("INSERT INTO worktree_registry(worktree_id,incarnation,git_dir,workspace_path,retired_at) VALUES(?1,?2,?3,?4,NULL)",params![origin.worktree_id,origin.incarnation,origin.git_dir.to_string_lossy(),origin.workspace_path.to_string_lossy()])?;
    }
    check_link(c, &link)?;
    Ok(link)
}
pub(crate) fn frozen_link(id: &str, origins: &[FrozenOrigin]) -> OriginLink {
    let mut found = origins.iter().filter(|o| o.worktree_id == id);
    let first = found.next();
    match (first, found.next()) {
        (Some(origin), None) => OriginLink {
            worktree_id: id.into(),
            incarnation: Some(origin.incarnation.clone()),
            qualification: Qualification::FrozenBaseline,
        },
        _ => OriginLink {
            worktree_id: id.into(),
            incarnation: None,
            qualification: Qualification::Unknown,
        },
    }
}
pub(crate) fn frozen_bindings(
    snapshot: &BindingSnapshot,
    origins: &[FrozenOrigin],
) -> Result<BindingMetadata, DevMapError> {
    let mut metadata = BindingMetadata::default();
    let mut latest = BTreeMap::new();
    for record in &snapshot.records {
        let id = binding_id(record)?;
        let link = BindingLink {
            destination: frozen_link(&record.worktree_id, origins),
            source: record
                .from_worktree_id
                .as_ref()
                .map(|id| frozen_link(id, origins)),
        };
        latest.insert(
            (record.host.clone(), record.task_id.clone()),
            (id.clone(), link.destination.clone()),
        );
        metadata.links.insert(id, link);
    }
    for (key, at) in &snapshot.watermarks {
        let previous = latest.get(key);
        metadata.cursors.insert(
            key.clone(),
            BindingCursor {
                observed_at: at.clone(),
                current: previous.map(|v| v.1.clone()),
                history_observation_id: previous.map(|v| v.0.clone()),
            },
        );
    }
    Ok(metadata)
}
pub(crate) fn insert_route(
    c: &Connection,
    plan: &RoutePlan,
    link: &OriginLink,
) -> Result<(), DevMapError> {
    check_link(c, link)?;
    if link.worktree_id != plan.worktree_id {
        return Err(fail("route origin target mismatch"));
    }
    c.execute(
        "INSERT INTO route_origin_links VALUES(?1,?2,?3,?4,?5)",
        params![
            plan.route_id,
            i64::try_from(plan.revision).map_err(|_| fail("route revision overflow"))?,
            link.worktree_id,
            link.incarnation,
            link.qualification.text()
        ],
    )?;
    Ok(())
}
pub(crate) fn insert_binding(
    c: &Connection,
    record: &TaskBindingObservation,
    link: &BindingLink,
) -> Result<(), DevMapError> {
    check_link(c, &link.destination)?;
    if let Some(source) = &link.source {
        check_link(c, source)?;
    }
    if link.destination.worktree_id != record.worktree_id
        || link.source.as_ref().map(|s| &s.worktree_id) != record.from_worktree_id.as_ref()
    {
        return Err(fail("binding origin record mismatch"));
    }
    c.execute(
        "INSERT INTO binding_origin_links VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            binding_id(record)?,
            link.destination.worktree_id,
            link.destination.incarnation,
            link.destination.qualification.text(),
            link.source.as_ref().map(|s| &s.worktree_id),
            link.source.as_ref().and_then(|s| s.incarnation.as_ref()),
            link.source
                .as_ref()
                .map_or("not_applicable", |s| s.qualification.text())
        ],
    )?;
    Ok(())
}
pub(crate) fn upsert_cursor(
    c: &Connection,
    host: &str,
    task: &str,
    cursor: &BindingCursor,
) -> Result<(), DevMapError> {
    if let Some(current) = &cursor.current {
        check_link(c, current)?;
    } else if cursor.history_observation_id.is_some() {
        return Err(fail("unobserved cursor has history"));
    }
    stamp(&cursor.observed_at)?;
    c.execute("INSERT INTO binding_origin_cursors VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source_scope) DO UPDATE SET observed_at=excluded.observed_at,current_worktree_id=excluded.current_worktree_id,current_incarnation=excluded.current_incarnation,qualification=excluded.qualification,history_observation_id=excluded.history_observation_id",params![serde_json::to_string(&(host,task))?,cursor.observed_at,cursor.current.as_ref().map(|s|&s.worktree_id),cursor.current.as_ref().and_then(|s|s.incarnation.as_ref()),cursor.current.as_ref().map_or("unobserved",|s|s.qualification.text()),cursor.history_observation_id])?;
    Ok(())
}
pub(crate) fn validate_routes(
    c: &Connection,
    records: &[Record],
) -> Result<BTreeMap<(String, u64), OriginLink>, DevMapError> {
    table_bounds(
        c,
        "route_origin_links",
        &[
            ("route_id", 256),
            ("worktree_id", 256),
            ("incarnation", 512),
            ("qualification", 32),
        ],
        records.len(),
    )?;
    let expected = records
        .iter()
        .map(|r| ((r.plan.route_id.clone(), r.plan.revision), &r.plan))
        .collect::<BTreeMap<_, _>>();
    let mut result = BTreeMap::new();
    let mut stmt = c.prepare(
        "SELECT route_id,revision,worktree_id,incarnation,qualification FROM route_origin_links",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (id, revision, worktree, incarnation, qualification) = row?;
        let key = (
            id,
            u64::try_from(revision).map_err(|_| fail("invalid route link revision"))?,
        );
        let link = decode(c, worktree, incarnation, qualification)?;
        if expected
            .get(&key)
            .is_none_or(|p| p.worktree_id != link.worktree_id)
            || result.insert(key, link).is_some()
        {
            return Err(fail("route origin coverage mismatch"));
        }
    }
    if result.len() != records.len() {
        return Err(fail("missing route origin link"));
    }
    Ok(result)
}
pub(crate) fn validate_bindings(
    c: &Connection,
    snapshot: &BindingSnapshot,
) -> Result<BindingMetadata, DevMapError> {
    table_bounds(
        c,
        "binding_origin_links",
        &[
            ("observation_id", 64),
            ("destination_worktree_id", 256),
            ("destination_incarnation", 512),
            ("destination_qualification", 32),
            ("source_worktree_id", 256),
            ("source_incarnation", 512),
            ("source_qualification", 32),
        ],
        snapshot.records.len(),
    )?;
    table_bounds(
        c,
        "binding_origin_cursors",
        &[
            ("source_scope", 4096),
            ("observed_at", 128),
            ("current_worktree_id", 256),
            ("current_incarnation", 512),
            ("qualification", 32),
            ("history_observation_id", 64),
        ],
        snapshot.watermarks.len(),
    )?;
    let mut expected = BTreeMap::new();
    let mut latest = BTreeMap::new();
    for record in &snapshot.records {
        let id = binding_id(record)?;
        expected.insert(id.clone(), record);
        latest.insert((record.host.clone(), record.task_id.clone()), id);
    }
    let mut metadata = BindingMetadata::default();
    let mut stmt=c.prepare("SELECT observation_id,destination_worktree_id,destination_incarnation,destination_qualification,source_worktree_id,source_incarnation,source_qualification FROM binding_origin_links")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, String>(6)?,
        ))
    })? {
        let (id, dest, di, dq, source, si, sq) = row?;
        let link = BindingLink {
            destination: decode(c, dest, di, dq)?,
            source: optional(c, source, si, sq, "not_applicable")?,
        };
        if expected.get(&id).is_none_or(|r| {
            r.worktree_id != link.destination.worktree_id
                || r.from_worktree_id.as_ref() != link.source.as_ref().map(|s| &s.worktree_id)
        }) || metadata.links.insert(id, link).is_some()
        {
            return Err(fail("binding origin coverage mismatch"));
        }
    }
    if metadata.links.len() != snapshot.records.len() {
        return Err(fail("missing binding origin link"));
    }
    let mut stmt=c.prepare("SELECT source_scope,observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id FROM binding_origin_cursors")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })? {
        let (scope, at, id, incarnation, qualification, history) = row?;
        let key: (String, String) = serde_json::from_str(&scope)?;
        let current = optional(c, id, incarnation, qualification, "unobserved")?;
        if scope != serde_json::to_string(&key)?
            || snapshot.watermarks.get(&key) != Some(&at)
            || stamp(&at).is_err()
            || history.as_ref() != latest.get(&key)
        {
            return Err(fail("binding cursor watermark/history mismatch"));
        }
        if let Some(history_id) = &history {
            let record = expected
                .get(history_id)
                .ok_or_else(|| fail("binding cursor history missing"))?;
            if current
                .as_ref()
                .is_none_or(|link| link.worktree_id != record.worktree_id)
                || stamp(&at)? < stamp(&record.observed_at)?
            {
                return Err(fail("binding cursor association mismatch"));
            }
            if stamp(&at)? == stamp(&record.observed_at)?
                && current.as_ref() != metadata.links.get(history_id).map(|link| &link.destination)
            {
                return Err(fail("equal-time binding cursor identity mismatch"));
            }
        } else if current.is_some() {
            return Err(fail("binding cursor association lacks history"));
        }
        if metadata
            .cursors
            .insert(
                key,
                BindingCursor {
                    observed_at: at,
                    current,
                    history_observation_id: history,
                },
            )
            .is_some()
        {
            return Err(fail("duplicate binding cursor"));
        }
    }
    if metadata.cursors.len() != snapshot.watermarks.len() {
        return Err(fail("missing binding cursor"));
    }
    Ok(metadata)
}
