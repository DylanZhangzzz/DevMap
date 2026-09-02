use serde_json::json;

use crate::error::DevMapError;
use crate::events::{
    ActorIdentity, CaptureGrade, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity,
    SessionContext,
};
use crate::journal::{JournalRecord, JournalStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementTraceInput {
    pub source_kind: String,
    pub source_locator: Option<String>,
    pub quoted_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDecisionInput {
    pub decision: String,
    pub basis: Vec<String>,
    pub alternatives: Vec<String>,
    pub rationale: String,
    pub scope: String,
    pub authority: String,
    pub revisit_trigger: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceInput {
    pub kind: String,
    pub target: String,
    pub command: Option<String>,
    pub outcome: String,
}

#[derive(Debug, Clone)]
pub struct CaptureKernel {
    journal: JournalStore,
    capture_grade: CaptureGrade,
    host: HostIdentity,
    actor: ActorIdentity,
    context: SessionContext,
}

impl CaptureKernel {
    pub fn new(
        journal: JournalStore,
        capture_grade: CaptureGrade,
        host: HostIdentity,
        actor: ActorIdentity,
        context: SessionContext,
    ) -> Self {
        Self {
            journal,
            capture_grade,
            host,
            actor,
            context,
        }
    }

    pub fn record_requirement(
        &self,
        event_id: &str,
        occurred_at: &str,
        input: RequirementTraceInput,
        raw_transcript_opt_in: bool,
    ) -> Result<JournalRecord, DevMapError> {
        self.record_requirement_with_id(Some(event_id), occurred_at, input, raw_transcript_opt_in)
    }

    pub fn record_requirement_with_id(
        &self,
        event_id: Option<&str>,
        occurred_at: &str,
        input: RequirementTraceInput,
        raw_transcript_opt_in: bool,
    ) -> Result<JournalRecord, DevMapError> {
        if raw_transcript_opt_in {
            return Err(DevMapError::RawTranscriptDisabled);
        }

        let source_kind = required("requirement_trace.source_kind", input.source_kind)?;
        let source_locator = optional("requirement_trace.source_locator", input.source_locator)?;
        let quoted_text = required("requirement_trace.quoted_text", input.quoted_text)?;
        self.record(
            event_id,
            "devmap_record_requirement",
            EventType::InstructionObserved,
            occurred_at,
            json!({
                "capture_grade": self.capture_grade,
                "requirement_trace": {
                    "source": {"kind": source_kind, "locator": source_locator},
                    "approved_quotation": quoted_text,
                },
            }),
        )
    }

    pub fn record_decision(
        &self,
        event_id: &str,
        occurred_at: &str,
        input: AgentDecisionInput,
    ) -> Result<JournalRecord, DevMapError> {
        self.record_decision_with_id(Some(event_id), occurred_at, input)
    }

    pub fn record_decision_with_id(
        &self,
        event_id: Option<&str>,
        occurred_at: &str,
        input: AgentDecisionInput,
    ) -> Result<JournalRecord, DevMapError> {
        let decision = required("agent_decision.decision", input.decision)?;
        let basis = required_list("agent_decision.basis", input.basis)?;
        let alternatives = required_list("agent_decision.alternatives", input.alternatives)?;
        let rationale = required("agent_decision.rationale", input.rationale)?;
        let scope = required("agent_decision.scope", input.scope)?;
        let authority = required("agent_decision.authority", input.authority)?;
        let revisit_trigger = required("agent_decision.revisit_trigger", input.revisit_trigger)?;
        self.record(
            event_id,
            "devmap_record_decision",
            EventType::DecisionRecorded,
            occurred_at,
            json!({
                "capture_grade": self.capture_grade,
                "agent_decision": {
                    "decision": decision,
                    "basis": basis,
                    "alternatives": alternatives,
                    "rationale": rationale,
                    "scope": scope,
                    "authority": authority,
                    "revisit_trigger": revisit_trigger,
                },
            }),
        )
    }

    pub fn record_evidence(
        &self,
        event_id: &str,
        occurred_at: &str,
        input: EvidenceInput,
    ) -> Result<JournalRecord, DevMapError> {
        self.record_evidence_with_id(Some(event_id), occurred_at, input)
    }

    pub fn record_evidence_with_id(
        &self,
        event_id: Option<&str>,
        occurred_at: &str,
        input: EvidenceInput,
    ) -> Result<JournalRecord, DevMapError> {
        let kind = required("evidence.kind", input.kind)?;
        let target = validate_evidence_target(input.target)?;
        let command = optional("evidence.command", input.command)?;
        let outcome = required("evidence.outcome", input.outcome)?;
        let provisional = target.starts_with("workspace:");
        self.record(
            event_id,
            "devmap_record_evidence",
            EventType::EvidenceRecorded,
            occurred_at,
            json!({
                "capture_grade": self.capture_grade,
                "evidence": {"kind": kind, "target": target, "command": command, "outcome": outcome},
                "provisional": provisional,
            }),
        )
    }

    pub fn record_gap(
        &self,
        event_id: &str,
        occurred_at: &str,
        reason: &str,
        mutation_target: &str,
    ) -> Result<JournalRecord, DevMapError> {
        let reason = required("capture_gap.reason", reason.to_owned())?;
        let mutation_target = validate_evidence_target(mutation_target.to_owned())?;
        self.record(
            Some(event_id),
            "gap",
            EventType::CaptureGap,
            occurred_at,
            json!({
                "capture_grade": self.capture_grade,
                "reason": reason,
                "mutation_target": mutation_target,
            }),
        )
    }

    fn record(
        &self,
        event_id: Option<&str>,
        default_event_kind: &str,
        event_type: EventType,
        occurred_at: &str,
        payload: serde_json::Value,
    ) -> Result<JournalRecord, DevMapError> {
        let mut records = self.journal.append_batch_with(|sequence| {
            let event_id = event_id.map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "mcp-{default_event_kind}-{}-{sequence}",
                    self.context.session_id()
                )
            });
            Ok(vec![EventEnvelope::new(
                EVENT_SCHEMA_VERSION,
                event_id,
                event_type,
                sequence,
                occurred_at,
                self.host.clone(),
                self.actor.clone(),
                self.context.clone(),
                payload,
            )?])
        })?;
        Ok(records.remove(0))
    }
}

fn required(field: &'static str, value: String) -> Result<String, DevMapError> {
    if value.trim().is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    Ok(value)
}

fn optional(field: &'static str, value: Option<String>) -> Result<Option<String>, DevMapError> {
    value.map(|value| required(field, value)).transpose()
}

fn required_list(field: &'static str, values: Vec<String>) -> Result<Vec<String>, DevMapError> {
    if values.is_empty() {
        return Err(DevMapError::InvalidDomain(field));
    }
    values
        .into_iter()
        .map(|value| required(field, value))
        .collect()
}

fn validate_evidence_target(target: String) -> Result<String, DevMapError> {
    let (kind, digest) = target
        .split_once(':')
        .ok_or_else(|| DevMapError::InvalidEvidenceTarget(target.clone()))?;
    if !matches!(kind, "commit" | "artifact" | "workspace") || !is_lowercase_digest(digest) {
        return Err(DevMapError::InvalidEvidenceTarget(target));
    }
    Ok(format!("{kind}:{digest}"))
}

fn is_lowercase_digest(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
