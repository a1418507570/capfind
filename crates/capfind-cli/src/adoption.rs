//! Adoption event log used by dashboard metrics.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use capfind_core::Capability;
use regex::Regex;
use serde_json::{json, Value};

const ADOPTIONS_FILE: &str = ".capfind/adoptions.jsonl";
pub const EVENT_SCHEMA_VERSION: &str = "capfind.adoption_event.v1";
pub const DETECTION_SCHEMA_VERSION: &str = "capfind.adoption_detection.v1";
const FUNNEL_STAGES: &[&str] = &["shown", "inspected", "adopted", "rejected"];
pub const REJECTION_REASONS: &[&str] = &[
    "wrong_ownership_boundary",
    "not_callable",
    "missing_behavior",
    "unsafe_abstraction",
    "external_api_required",
    "low_confidence",
    "stale_index",
    "user_requested_new",
    "other",
    "unspecified",
];

pub struct EventRecord<'a> {
    pub source: &'a str,
    pub task: &'a str,
    pub candidate: &'a Capability,
    pub stage: &'a str,
    pub files: &'a [String],
    pub note: Option<&'a str>,
    pub rejected_reason: Option<&'a str>,
    pub session_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
struct AddedLine {
    file: String,
    line: u32,
    text: String,
    source: &'static str,
}

pub fn record_event_with_source(repo_root: &Path, event: EventRecord<'_>) -> Result<()> {
    let Some(stage) = normalize_stage(event.stage) else {
        bail!("unknown adoption funnel stage: {}", event.stage);
    };
    let adopted = adopted_for_stage(stage);
    let reason = if stage == "rejected" {
        json!(normalize_rejection_reason(event.rejected_reason))
    } else {
        Value::Null
    };
    append_record(
        repo_root,
        json!({
            "schema_version": EVENT_SCHEMA_VERSION,
            "created_unix": now_unix(),
            "source": event.source,
            "event": stage,
            "funnel_stage": stage,
            "task": event.task,
            "candidate_id": event.candidate.id,
            "candidate_type": crate::context::candidate_type(event.candidate),
            "candidate_file": event.candidate.file,
            "candidate_line": event.candidate.line,
            "candidate_signature": event.candidate.signature,
            "adopted": adopted,
            "rejected_reason": reason,
            "session_id": event.session_id,
            "files": event.files,
            "note": event.note,
        }),
    )
}

pub fn detect_from_git_diff(
    repo_root: &Path,
    capabilities: &[Capability],
    since: &str,
    task: Option<&str>,
    candidate_ids: &[u32],
    session_id: Option<&str>,
    dry_run: bool,
) -> Result<Value> {
    let added_lines = collect_added_lines(repo_root, since)?;
    let changed_files = added_lines
        .iter()
        .map(|line| line.file.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let selected_ids = candidate_ids.iter().copied().collect::<BTreeSet<_>>();
    let task_text = task.unwrap_or("auto_detected_git_diff_adoption");

    let mut detections = Vec::new();
    let mut recorded_events = 0u64;
    let mut skipped_duplicates = 0u64;
    for candidate in capabilities
        .iter()
        .filter(|candidate| selected_ids.is_empty() || selected_ids.contains(&candidate.id))
    {
        let evidence = candidate_evidence(candidate, &added_lines)?;
        if evidence.is_empty() {
            continue;
        }
        let evidence_files = evidence_files(&evidence);
        let confidence = detection_confidence(&evidence);
        let detection_key = detection_key(candidate.id, &evidence);
        let recorded = if dry_run {
            false
        } else if detection_key_exists(repo_root, &detection_key)? {
            skipped_duplicates += 1;
            false
        } else {
            append_record(
                repo_root,
                json!({
                    "schema_version": EVENT_SCHEMA_VERSION,
                    "created_unix": now_unix(),
                    "source": "git_diff",
                    "event": "adopted",
                    "funnel_stage": "adopted",
                    "detection_key": detection_key,
                    "detection_kind": "final_code_called_candidate",
                    "task": task_text,
                    "candidate_id": candidate.id,
                    "candidate_type": crate::context::candidate_type(candidate),
                    "candidate_file": candidate.file,
                    "candidate_line": candidate.line,
                    "candidate_signature": candidate.signature,
                    "adopted": true,
                    "rejected_reason": null,
                    "session_id": session_id,
                    "files": evidence_files,
                    "note": format!("auto-detected from git diff since {since}"),
                    "confidence": confidence,
                    "evidence": evidence,
                }),
            )?;
            recorded_events += 1;
            true
        };

        detections.push(json!({
            "candidate_id": candidate.id,
            "candidate_type": crate::context::candidate_type(candidate),
            "candidate_file": candidate.file,
            "candidate_line": candidate.line,
            "candidate_signature": candidate.signature,
            "confidence": confidence,
            "recorded": recorded,
            "evidence": evidence,
        }));
    }

    Ok(json!({
        "schema_version": DETECTION_SCHEMA_VERSION,
        "since": since,
        "task": task_text,
        "session_id": session_id,
        "dry_run": dry_run,
        "diff": {
            "changed_files": changed_files,
            "added_lines": added_lines.len(),
            "tracked_diff_included": true,
            "untracked_files_included": true,
        },
        "detections": detections,
        "recorded_events": recorded_events,
        "skipped_duplicates": skipped_duplicates,
        "adoption": summary(repo_root),
    }))
}

pub fn summary(repo_root: &Path) -> Value {
    let path = repo_root.join(ADOPTIONS_FILE);
    let Ok(file) = fs::File::open(&path) else {
        return json!({
            "path": path,
            "events": 0,
            "final_events": 0,
            "adopted": 0,
            "rejected": 0,
            "auto_detected": 0,
            "by_source": {},
            "funnel": empty_funnel(),
            "rejection_taxonomy": empty_rejection_taxonomy(),
            "funnel_by_candidate": [],
            "funnel_by_task": [],
            "sessions": [],
            "quality": empty_quality_summary(),
            "adoption_rate": 0.0,
        });
    };

    let mut events = 0u64;
    let mut final_events = 0u64;
    let mut adopted = 0u64;
    let mut rejected = 0u64;
    let mut auto_detected = 0u64;
    let mut by_source = BTreeMap::new();
    let mut funnel_events = initialized_stage_counts();
    let mut rejected_by_reason = BTreeMap::new();
    let mut by_candidate = BTreeMap::<u64, FunnelRollup>::new();
    let mut by_task = BTreeMap::<String, FunnelRollup>::new();
    let mut by_session = BTreeMap::<String, FunnelRollup>::new();
    for line in std::io::BufReader::new(file)
        .lines()
        .map_while(|line| line.ok())
    {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        events += 1;
        let stage = event_stage(&value);
        if let Some(count) = funnel_events.get_mut(stage.as_str()) {
            *count += 1;
        }
        let outcome = final_adoption(&value, &stage);
        match outcome {
            Some(true) => {
                adopted += 1;
                final_events += 1;
            }
            Some(false) => {
                rejected += 1;
                final_events += 1;
                let reason = value
                    .get("rejected_reason")
                    .and_then(Value::as_str)
                    .map(|reason| normalize_rejection_reason(Some(reason)))
                    .unwrap_or_else(|| "unspecified".to_string());
                *rejected_by_reason.entry(reason).or_insert(0u64) += 1;
            }
            None => {}
        }
        let source = value
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        *by_source.entry(source.to_string()).or_insert(0u64) += 1;
        if source == "git_diff" || value.get("detection_key").is_some() {
            auto_detected += 1;
        }
        record_funnel_rollups(
            &mut by_candidate,
            &mut by_task,
            &mut by_session,
            &value,
            &stage,
            outcome,
        );
    }
    let rate = if final_events == 0 {
        0.0
    } else {
        adopted as f64 / final_events as f64
    };
    let candidate_rollups = candidate_rollups_json(&by_candidate);
    let task_rollups = keyed_rollups_json("task", &by_task);
    let session_rollups = keyed_rollups_json("session_id", &by_session);
    let quality = adoption_quality_json(&by_candidate, &by_task, &by_session);
    json!({
        "path": path,
        "events": events,
        "final_events": final_events,
        "adopted": adopted,
        "rejected": rejected,
        "auto_detected": auto_detected,
        "by_source": by_source,
        "funnel": {
            "events": funnel_events,
            "final_outcomes": final_events,
            "adoption_rate": round_rate(rate),
        },
        "rejection_taxonomy": {
            "available_reasons": REJECTION_REASONS,
            "by_reason": rejected_by_reason,
        },
        "funnel_by_candidate": candidate_rollups,
        "funnel_by_task": task_rollups,
        "sessions": session_rollups,
        "quality": quality,
        "adoption_rate": round_rate(rate),
    })
}

pub fn normalize_stage(stage: &str) -> Option<&'static str> {
    match normalize_key(stage).as_str() {
        "shown" => Some("shown"),
        "inspected" => Some("inspected"),
        "adopted" => Some("adopted"),
        "rejected" => Some("rejected"),
        _ => None,
    }
}

pub fn normalize_rejection_reason(reason: Option<&str>) -> String {
    let normalized = reason
        .map(normalize_key)
        .filter(|reason| !reason.is_empty())
        .unwrap_or_else(|| "unspecified".to_string());
    if REJECTION_REASONS.contains(&normalized.as_str()) {
        normalized
    } else {
        "other".to_string()
    }
}

pub fn adopted_for_stage(stage: &str) -> Option<bool> {
    match stage {
        "adopted" => Some(true),
        "rejected" => Some(false),
        _ => None,
    }
}

fn empty_funnel() -> Value {
    json!({
        "events": initialized_stage_counts(),
        "final_outcomes": 0,
        "adoption_rate": 0.0,
    })
}

fn empty_rejection_taxonomy() -> Value {
    json!({
        "available_reasons": REJECTION_REASONS,
        "by_reason": {},
    })
}

fn empty_quality_summary() -> Value {
    json!({
        "schema_version": "capfind.adoption_quality.v1",
        "heuristic": "laplace_smoothed_candidate_adoption_probability_from_correlated_funnel_events",
        "summary": {
            "candidate_count": 0,
            "task_count": 0,
            "session_count": 0,
            "average_adoption_probability": 0.0,
            "high_quality_candidates": 0,
            "noise_risk_candidates": 0,
            "low_confidence_candidates": 0,
        },
        "candidate_quality": [],
        "top_reusable_candidates": [],
        "noise_risk_candidates": [],
        "low_confidence_pockets": [],
        "task_quality": [],
        "session_quality": [],
    })
}

fn initialized_stage_counts() -> BTreeMap<String, u64> {
    FUNNEL_STAGES
        .iter()
        .map(|stage| ((*stage).to_string(), 0))
        .collect()
}

#[derive(Debug, Clone)]
struct FunnelRollup {
    events: u64,
    stages: BTreeMap<String, u64>,
    final_events: u64,
    adopted: u64,
    rejected: u64,
    latest_event_unix: u64,
    latest_stage: Option<String>,
    sources: BTreeMap<String, u64>,
    tasks: BTreeSet<String>,
    candidate_ids: BTreeSet<u64>,
    sessions: BTreeSet<String>,
    candidate_type: Option<String>,
    candidate_file: Option<String>,
    candidate_line: Option<u64>,
    candidate_signature: Option<String>,
}

impl Default for FunnelRollup {
    fn default() -> Self {
        Self {
            events: 0,
            stages: initialized_stage_counts(),
            final_events: 0,
            adopted: 0,
            rejected: 0,
            latest_event_unix: 0,
            latest_stage: None,
            sources: BTreeMap::new(),
            tasks: BTreeSet::new(),
            candidate_ids: BTreeSet::new(),
            sessions: BTreeSet::new(),
            candidate_type: None,
            candidate_file: None,
            candidate_line: None,
            candidate_signature: None,
        }
    }
}

impl FunnelRollup {
    fn record(&mut self, value: &Value, stage: &str, outcome: Option<bool>) {
        self.events += 1;
        if let Some(count) = self.stages.get_mut(stage) {
            *count += 1;
        }
        match outcome {
            Some(true) => {
                self.adopted += 1;
                self.final_events += 1;
            }
            Some(false) => {
                self.rejected += 1;
                self.final_events += 1;
            }
            None => {}
        }
        let created_unix = value
            .get("created_unix")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if created_unix >= self.latest_event_unix {
            self.latest_event_unix = created_unix;
            self.latest_stage = Some(stage.to_string());
        }
        if let Some(source) = string_field(value, "source") {
            *self.sources.entry(source).or_insert(0) += 1;
        }
        if let Some(task) = string_field(value, "task") {
            self.tasks.insert(task);
        }
        if let Some(candidate_id) = value.get("candidate_id").and_then(Value::as_u64) {
            self.candidate_ids.insert(candidate_id);
        }
        if let Some(session_id) = string_field(value, "session_id") {
            self.sessions.insert(session_id);
        }
        if self.candidate_type.is_none() {
            self.candidate_type = string_field(value, "candidate_type");
        }
        if self.candidate_file.is_none() {
            self.candidate_file = string_field(value, "candidate_file");
        }
        if self.candidate_line.is_none() {
            self.candidate_line = value.get("candidate_line").and_then(Value::as_u64);
        }
        if self.candidate_signature.is_none() {
            self.candidate_signature = string_field(value, "candidate_signature");
        }
    }

    fn adoption_rate(&self) -> f64 {
        if self.final_events == 0 {
            0.0
        } else {
            self.adopted as f64 / self.final_events as f64
        }
    }

    fn outcome_label(&self) -> &'static str {
        match (self.adopted > 0, self.rejected > 0) {
            (true, true) => "mixed",
            (true, false) => "adopted",
            (false, true) => "rejected",
            (false, false) => "open",
        }
    }
}

fn record_funnel_rollups(
    by_candidate: &mut BTreeMap<u64, FunnelRollup>,
    by_task: &mut BTreeMap<String, FunnelRollup>,
    by_session: &mut BTreeMap<String, FunnelRollup>,
    value: &Value,
    stage: &str,
    outcome: Option<bool>,
) {
    if let Some(candidate_id) = value.get("candidate_id").and_then(Value::as_u64) {
        by_candidate
            .entry(candidate_id)
            .or_default()
            .record(value, stage, outcome);
    }
    if let Some(task) = string_field(value, "task") {
        by_task
            .entry(task)
            .or_default()
            .record(value, stage, outcome);
    }
    if let Some(session_id) = string_field(value, "session_id") {
        by_session
            .entry(session_id)
            .or_default()
            .record(value, stage, outcome);
    }
}

fn candidate_rollups_json(rollups: &BTreeMap<u64, FunnelRollup>) -> Vec<Value> {
    let mut rows = rollups
        .iter()
        .map(|(candidate_id, rollup)| {
            let mut row = rollup_json(rollup);
            row["candidate_id"] = json!(candidate_id);
            row["candidate_type"] = json!(rollup.candidate_type);
            row["candidate_file"] = json!(rollup.candidate_file);
            row["candidate_line"] = json!(rollup.candidate_line);
            row["candidate_signature"] = json!(rollup.candidate_signature);
            row
        })
        .collect::<Vec<_>>();
    sort_rollup_rows(&mut rows);
    rows
}

fn keyed_rollups_json(key_name: &str, rollups: &BTreeMap<String, FunnelRollup>) -> Vec<Value> {
    let mut rows = rollups
        .iter()
        .map(|(key, rollup)| {
            let mut row = rollup_json(rollup);
            row[key_name] = json!(key);
            row
        })
        .collect::<Vec<_>>();
    sort_rollup_rows(&mut rows);
    rows
}

fn rollup_json(rollup: &FunnelRollup) -> Value {
    json!({
        "events": rollup.events,
        "stages": rollup.stages,
        "stage_path": stage_path(&rollup.stages),
        "final_events": rollup.final_events,
        "adopted": rollup.adopted,
        "rejected": rollup.rejected,
        "adoption_rate": round_rate(rollup.adoption_rate()),
        "outcome": rollup.outcome_label(),
        "latest_event_unix": rollup.latest_event_unix,
        "latest_stage": rollup.latest_stage,
        "sources": rollup.sources,
        "tasks": set_values(&rollup.tasks),
        "task_count": rollup.tasks.len(),
        "candidate_ids": rollup.candidate_ids.iter().copied().collect::<Vec<_>>(),
        "candidate_count": rollup.candidate_ids.len(),
        "session_ids": set_values(&rollup.sessions),
        "session_count": rollup.sessions.len(),
    })
}

fn adoption_quality_json(
    by_candidate: &BTreeMap<u64, FunnelRollup>,
    by_task: &BTreeMap<String, FunnelRollup>,
    by_session: &BTreeMap<String, FunnelRollup>,
) -> Value {
    let mut candidate_quality = by_candidate
        .iter()
        .map(|(candidate_id, rollup)| {
            let mut row = quality_row("candidate", rollup);
            row["candidate_id"] = json!(candidate_id);
            row["candidate_type"] = json!(rollup.candidate_type);
            row["candidate_file"] = json!(rollup.candidate_file);
            row["candidate_line"] = json!(rollup.candidate_line);
            row["candidate_signature"] = json!(rollup.candidate_signature);
            row
        })
        .collect::<Vec<_>>();
    sort_quality_rows(&mut candidate_quality, "adoption_probability");

    let mut top_reusable_candidates = candidate_quality
        .iter()
        .filter(|row| {
            row["quality_label"]
                .as_str()
                .is_some_and(|label| label == "likely_reusable" || label == "strong_reuse_signal")
        })
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    sort_quality_rows(&mut top_reusable_candidates, "adoption_probability");

    let mut noise_risk_candidates = candidate_quality
        .iter()
        .filter(|row| {
            row["quality_label"]
                .as_str()
                .is_some_and(|label| label == "noise_risk")
        })
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    sort_quality_rows(&mut noise_risk_candidates, "rejection_probability");

    let mut low_confidence_pockets = candidate_quality
        .iter()
        .filter(|row| {
            row["quality_label"]
                .as_str()
                .is_some_and(|label| label == "low_confidence")
        })
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    sort_quality_rows(&mut low_confidence_pockets, "events");

    let mut task_quality = by_task
        .iter()
        .map(|(task, rollup)| {
            let mut row = quality_row("task", rollup);
            row["task"] = json!(task);
            row
        })
        .collect::<Vec<_>>();
    sort_quality_rows(&mut task_quality, "adoption_probability");

    let mut session_quality = by_session
        .iter()
        .map(|(session_id, rollup)| {
            let mut row = quality_row("session", rollup);
            row["session_id"] = json!(session_id);
            row
        })
        .collect::<Vec<_>>();
    sort_quality_rows(&mut session_quality, "adoption_probability");

    json!({
        "schema_version": "capfind.adoption_quality.v1",
        "heuristic": "laplace_smoothed_candidate_adoption_probability_from_correlated_funnel_events",
        "summary": {
            "candidate_count": by_candidate.len(),
            "task_count": by_task.len(),
            "session_count": by_session.len(),
            "average_adoption_probability": average_quality_rate(&candidate_quality, "adoption_probability"),
            "high_quality_candidates": count_quality_label(&candidate_quality, &["likely_reusable", "strong_reuse_signal"]),
            "noise_risk_candidates": count_quality_label(&candidate_quality, &["noise_risk"]),
            "low_confidence_candidates": count_quality_label(&candidate_quality, &["low_confidence"]),
        },
        "candidate_quality": candidate_quality,
        "top_reusable_candidates": top_reusable_candidates,
        "noise_risk_candidates": noise_risk_candidates,
        "low_confidence_pockets": low_confidence_pockets,
        "task_quality": task_quality.into_iter().take(20).collect::<Vec<_>>(),
        "session_quality": session_quality.into_iter().take(20).collect::<Vec<_>>(),
    })
}

fn quality_row(scope: &str, rollup: &FunnelRollup) -> Value {
    let shown = stage_count(rollup, "shown");
    let inspected = stage_count(rollup, "inspected");
    let opportunities = recommendation_opportunities(rollup);
    let adoption_probability = smoothed_probability(rollup.adopted, opportunities);
    let rejection_probability = smoothed_probability(rollup.rejected, opportunities);
    let confidence = quality_confidence(rollup, opportunities);
    let label = quality_label(
        rollup,
        adoption_probability,
        rejection_probability,
        confidence,
    );
    json!({
        "scope": scope,
        "events": rollup.events,
        "opportunities": opportunities,
        "shown": shown,
        "inspected": inspected,
        "adopted": rollup.adopted,
        "rejected": rollup.rejected,
        "final_events": rollup.final_events,
        "adoption_probability": round_rate(adoption_probability),
        "rejection_probability": round_rate(rejection_probability),
        "observed_adoption_rate": round_rate(rollup.adoption_rate()),
        "inspection_rate": rate(inspected, shown),
        "confidence": round_rate(confidence),
        "quality_label": label,
        "recommended_action": quality_action(label),
        "reason": quality_reason(label),
        "stage_path": stage_path(&rollup.stages),
        "tasks": set_values(&rollup.tasks),
        "task_count": rollup.tasks.len(),
        "candidate_ids": rollup.candidate_ids.iter().copied().collect::<Vec<_>>(),
        "candidate_count": rollup.candidate_ids.len(),
        "session_ids": set_values(&rollup.sessions),
        "session_count": rollup.sessions.len(),
        "sources": rollup.sources,
        "latest_event_unix": rollup.latest_event_unix,
        "latest_stage": rollup.latest_stage,
    })
}

fn recommendation_opportunities(rollup: &FunnelRollup) -> u64 {
    stage_count(rollup, "shown")
        .max(rollup.final_events)
        .max(rollup.sessions.len() as u64)
        .max(rollup.tasks.len() as u64)
        .max(if rollup.events > 0 { 1 } else { 0 })
}

fn stage_count(rollup: &FunnelRollup, stage: &str) -> u64 {
    rollup.stages.get(stage).copied().unwrap_or_default()
}

fn smoothed_probability(successes: u64, opportunities: u64) -> f64 {
    (successes as f64 + 1.0) / (opportunities as f64 + 2.0)
}

fn quality_confidence(rollup: &FunnelRollup, opportunities: u64) -> f64 {
    let evidence_weight = opportunities
        + rollup.final_events
        + rollup.sessions.len() as u64
        + rollup.tasks.len() as u64;
    (evidence_weight as f64 / 8.0).min(1.0)
}

fn quality_label(
    rollup: &FunnelRollup,
    adoption_probability: f64,
    rejection_probability: f64,
    confidence: f64,
) -> &'static str {
    if confidence < 0.35 {
        "low_confidence"
    } else if rollup.rejected > rollup.adopted && rejection_probability >= 0.5 {
        "noise_risk"
    } else if rollup.adopted >= 3 && adoption_probability >= 0.65 && confidence >= 0.75 {
        "strong_reuse_signal"
    } else if rollup.adopted > rollup.rejected && adoption_probability >= rejection_probability {
        "likely_reusable"
    } else if rollup.adopted > 0 && rollup.rejected > 0 {
        "mixed"
    } else {
        "watch"
    }
}

fn quality_action(label: &str) -> &'static str {
    match label {
        "strong_reuse_signal" | "likely_reusable" => "prefer_candidate_when_task_matches",
        "noise_risk" => "inspect_before_recommending_or_suppress",
        "low_confidence" => "collect_more_correlated_outcomes",
        "mixed" => "inspect_context_and_ownership_before_use",
        _ => "watch_more_sessions",
    }
}

fn quality_reason(label: &str) -> &'static str {
    match label {
        "strong_reuse_signal" => "multiple correlated outcomes adopted this candidate",
        "likely_reusable" => "adoptions exceed rejections in correlated outcomes",
        "noise_risk" => "rejections exceed adoptions in correlated outcomes",
        "low_confidence" => "too few correlated events to trust probability",
        "mixed" => "correlated outcomes include both adoption and rejection",
        _ => "correlated outcomes are still neutral",
    }
}

fn rate(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        round_rate(numerator as f64 / denominator as f64)
    }
}

fn average_quality_rate(rows: &[Value], field: &str) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    let sum = rows
        .iter()
        .map(|row| row.get(field).and_then(Value::as_f64).unwrap_or_default())
        .sum::<f64>();
    round_rate(sum / rows.len() as f64)
}

fn count_quality_label(rows: &[Value], labels: &[&str]) -> u64 {
    rows.iter()
        .filter(|row| {
            row["quality_label"]
                .as_str()
                .is_some_and(|label| labels.contains(&label))
        })
        .count() as u64
}

fn sort_quality_rows(rows: &mut [Value], primary_key: &str) {
    rows.sort_by(|a, b| {
        value_f64(b, primary_key)
            .partial_cmp(&value_f64(a, primary_key))
            .unwrap_or(Ordering::Equal)
            .then(
                value_f64(b, "confidence")
                    .partial_cmp(&value_f64(a, "confidence"))
                    .unwrap_or(Ordering::Equal),
            )
            .then(value_u64(b, "events").cmp(&value_u64(a, "events")))
            .then(a.to_string().cmp(&b.to_string()))
    });
}

fn value_f64(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn value_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn sort_rollup_rows(rows: &mut [Value]) {
    rows.sort_by(|a, b| {
        b["events"]
            .as_u64()
            .cmp(&a["events"].as_u64())
            .then(b["final_events"].as_u64().cmp(&a["final_events"].as_u64()))
            .then(a.to_string().cmp(&b.to_string()))
    });
}

fn stage_path(stages: &BTreeMap<String, u64>) -> Vec<String> {
    FUNNEL_STAGES
        .iter()
        .filter(|stage| stages.get::<str>(*stage).copied().unwrap_or_default() > 0)
        .map(|stage| (*stage).to_string())
        .collect()
}

fn set_values<T: ToString>(values: &BTreeSet<T>) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn event_stage(value: &Value) -> String {
    value
        .get("funnel_stage")
        .or_else(|| value.get("event"))
        .and_then(Value::as_str)
        .and_then(normalize_stage)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("adopted")
                .and_then(Value::as_bool)
                .map(|adopted| if adopted { "adopted" } else { "rejected" }.to_string())
        })
        .unwrap_or_else(|| "shown".to_string())
}

fn final_adoption(value: &Value, stage: &str) -> Option<bool> {
    value
        .get("adopted")
        .and_then(Value::as_bool)
        .or_else(|| adopted_for_stage(stage))
}

fn normalize_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch == '-' || ch.is_whitespace() {
                '_'
            } else {
                ch.to_ascii_lowercase()
            }
        })
        .collect()
}

fn round_rate(rate: f64) -> f64 {
    (rate * 1000.0).round() / 1000.0
}

fn append_record(repo_root: &Path, record: Value) -> Result<()> {
    let path = repo_root.join(ADOPTIONS_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", record)?;
    Ok(())
}

fn collect_added_lines(repo_root: &Path, since: &str) -> Result<Vec<AddedLine>> {
    let output = Command::new("git")
        .args([
            "diff",
            "--unified=0",
            "--no-ext-diff",
            "--no-color",
            since,
            "--",
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to run git diff for adoption detection")?;
    if !output.status.success() {
        bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let mut added_lines = parse_git_diff(&String::from_utf8_lossy(&output.stdout));
    added_lines.extend(untracked_file_lines(repo_root)?);
    Ok(added_lines)
}

fn parse_git_diff(diff: &str) -> Vec<AddedLine> {
    let hunk_re = Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@").unwrap();
    let mut current_file = String::new();
    let mut next_line = 0u32;
    let mut added_lines = Vec::new();

    for raw in diff.lines() {
        if let Some(path) = raw.strip_prefix("+++ b/") {
            current_file = path.to_string();
            continue;
        }
        if raw.starts_with("+++ /dev/null") {
            current_file.clear();
            continue;
        }
        if let Some(captures) = hunk_re.captures(raw) {
            next_line = captures
                .get(1)
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            continue;
        }
        if raw.starts_with("diff --git ")
            || raw.starts_with("--- ")
            || raw.starts_with("index ")
            || raw.starts_with("new file mode ")
            || raw.starts_with("deleted file mode ")
        {
            continue;
        }
        if let Some(text) = raw.strip_prefix('+') {
            if !current_file.is_empty() && !text.trim().is_empty() {
                added_lines.push(AddedLine {
                    file: current_file.clone(),
                    line: next_line,
                    text: text.to_string(),
                    source: "git_diff",
                });
            }
            next_line = next_line.saturating_add(1);
        } else if raw.starts_with(' ') {
            next_line = next_line.saturating_add(1);
        }
    }

    added_lines
}

fn untracked_file_lines(repo_root: &Path) -> Result<Vec<AddedLine>> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(repo_root)
        .output()
        .context("failed to list untracked files for adoption detection")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let mut lines = Vec::new();
    for path in String::from_utf8_lossy(&output.stdout).lines() {
        if path.trim().is_empty() || path.starts_with(".capfind/") {
            continue;
        }
        let full_path = repo_root.join(path);
        let Ok(metadata) = fs::metadata(&full_path) else {
            continue;
        };
        if metadata.len() > 1024 * 1024 {
            continue;
        }
        let Ok(content) = fs::read_to_string(&full_path) else {
            continue;
        };
        for (idx, text) in content.lines().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            lines.push(AddedLine {
                file: path.to_string(),
                line: (idx + 1) as u32,
                text: text.to_string(),
                source: "untracked_file",
            });
        }
    }
    Ok(lines)
}

fn candidate_evidence(candidate: &Capability, added_lines: &[AddedLine]) -> Result<Vec<Value>> {
    let mut evidence = Vec::new();
    let method = candidate.method.trim();
    let method_re = meaningful_symbol(method)
        .then(|| Regex::new(&format!(r"\b{}\s*\(", regex::escape(method))))
        .transpose()?;
    let dot_method_re = meaningful_symbol(method)
        .then(|| Regex::new(&format!(r"\.\s*{}\s*\(", regex::escape(method))))
        .transpose()?;
    let class_name = candidate
        .class
        .as_deref()
        .and_then(short_class_name)
        .filter(|name| meaningful_symbol(name));
    let class_ref_re = class_name
        .map(|name| Regex::new(&format!(r"\b{}\b", regex::escape(name))))
        .transpose()?;
    let class_ctor_re = class_name
        .map(|name| Regex::new(&format!(r"\bnew\s+{}\s*\(", regex::escape(name))))
        .transpose()?;

    for line in added_lines {
        let trimmed = line.text.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }
        if let Some(http) = candidate.http.as_ref() {
            if http.path.len() > 1 && trimmed.contains(&http.path) {
                evidence.push(evidence_json(line, "http_path", "high", &http.path));
            }
        }
        if let Some(rpc) = candidate.rpc.as_ref() {
            if meaningful_symbol(&rpc.rpc) && trimmed.contains(&format!("{}(", rpc.rpc)) {
                evidence.push(evidence_json(line, "rpc_call", "high", &rpc.rpc));
            }
            if meaningful_symbol(&rpc.service) && trimmed.contains(&rpc.service) {
                evidence.push(evidence_json(
                    line,
                    "rpc_service_ref",
                    "medium",
                    &rpc.service,
                ));
            }
        }
        if let Some(re) = dot_method_re.as_ref() {
            if re.is_match(trimmed) {
                evidence.push(evidence_json(line, "method_call", "high", method));
            }
        }
        if let Some(re) = method_re.as_ref() {
            if !looks_like_declaration(trimmed)
                && !evidence
                    .iter()
                    .any(|item| item["kind"] == "method_call" && item["file"] == line.file)
                && re.is_match(trimmed)
            {
                evidence.push(evidence_json(line, "bare_method_call", "medium", method));
            }
        }
        if let Some(re) = class_ctor_re.as_ref() {
            if re.is_match(trimmed) {
                evidence.push(evidence_json(
                    line,
                    "class_constructor",
                    "high",
                    class_name.unwrap_or_default(),
                ));
            }
        }
        if let Some(re) = class_ref_re.as_ref() {
            if re.is_match(trimmed) {
                evidence.push(evidence_json(
                    line,
                    class_ref_kind(trimmed),
                    class_ref_confidence(trimmed),
                    class_name.unwrap_or_default(),
                ));
            }
        }
        if is_external_import(candidate, trimmed) {
            evidence.push(evidence_json(
                line,
                "external_import",
                "high",
                &candidate.signature,
            ));
        }
        if evidence.len() >= 8 {
            break;
        }
    }

    let high_count = evidence
        .iter()
        .filter(|item| item["confidence"] == "high")
        .count();
    let medium_count = evidence
        .iter()
        .filter(|item| item["confidence"] == "medium")
        .count();
    if high_count > 0 || medium_count >= 2 {
        Ok(evidence)
    } else {
        Ok(Vec::new())
    }
}

fn evidence_json(line: &AddedLine, kind: &str, confidence: &str, needle: &str) -> Value {
    json!({
        "file": line.file,
        "line": line.line,
        "source": line.source,
        "kind": kind,
        "confidence": confidence,
        "needle": needle,
        "line_text": line.text.trim(),
    })
}

fn evidence_files(evidence: &[Value]) -> Vec<String> {
    evidence
        .iter()
        .filter_map(|item| item.get("file").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn detection_confidence(evidence: &[Value]) -> &'static str {
    if evidence.iter().any(|item| item["confidence"] == "high") {
        "high"
    } else {
        "medium"
    }
}

fn detection_key(candidate_id: u32, evidence: &[Value]) -> String {
    let fingerprint = evidence
        .iter()
        .map(|item| {
            format!(
                "{}:{}:{}",
                item.get("file").and_then(Value::as_str).unwrap_or_default(),
                item.get("line").and_then(Value::as_u64).unwrap_or_default(),
                item.get("kind").and_then(Value::as_str).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let hash = blake3::hash(fingerprint.as_bytes()).to_hex().to_string();
    format!("git_diff:{candidate_id}:{}", &hash[..16])
}

fn detection_key_exists(repo_root: &Path, detection_key: &str) -> Result<bool> {
    let path = repo_root.join(ADOPTIONS_FILE);
    let Ok(file) = fs::File::open(&path) else {
        return Ok(false);
    };
    for line in std::io::BufReader::new(file)
        .lines()
        .map_while(|line| line.ok())
    {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value
            .get("detection_key")
            .and_then(Value::as_str)
            .is_some_and(|key| key == detection_key)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn meaningful_symbol(value: &str) -> bool {
    let value = value.trim();
    value.len() >= 4
        && !matches!(
            value,
            "main" | "test" | "get" | "set" | "list" | "find" | "save" | "run" | "call"
        )
}

fn short_class_name(class: &str) -> Option<&str> {
    class
        .rsplit(['.', '$'])
        .next()
        .filter(|name| !name.is_empty())
}

fn looks_like_declaration(line: &str) -> bool {
    let line = line.trim_start();
    matches!(
        line.split_whitespace().next(),
        Some(
            "public"
                | "private"
                | "protected"
                | "class"
                | "interface"
                | "record"
                | "enum"
                | "func"
                | "def"
        )
    )
}

fn class_ref_kind(line: &str) -> &'static str {
    if line.trim_start().starts_with("import ") {
        "class_import"
    } else {
        "class_ref"
    }
}

fn class_ref_confidence(line: &str) -> &'static str {
    if line.trim_start().starts_with("import ") {
        "high"
    } else {
        "medium"
    }
}

fn is_external_import(candidate: &Capability, line: &str) -> bool {
    if !candidate.tags.iter().any(|tag| tag == "external") {
        return false;
    }
    let signature = candidate.signature.trim();
    signature.starts_with("import ") && line.trim() == signature
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
