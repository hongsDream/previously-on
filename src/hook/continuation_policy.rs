use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{json, Value};

use crate::domain::{
    deterministic_id, ContinuationAdviceV1, ContinuationReasonV1, ContinuationStateV1,
    EventEnvelopeV1, EventKind, TemporalStatusV1,
};
use crate::store::Store;

pub(super) struct ProposedContinuation {
    pub(super) advice: ContinuationAdviceV1,
    pub(super) claim_generation: String,
}

pub(super) fn rollover_advice(
    store: &Store,
    event: &EventEnvelopeV1,
) -> Result<Option<ProposedContinuation>> {
    if let Some(proposed) = continuation_advice_for_source(store, event)? {
        return Ok(Some(proposed));
    }
    if let Some(proposed) = pending_continuation_advice(store, event)? {
        return Ok(Some(proposed));
    }
    let Some(session) = store.get_session(&event.session_id)? else {
        return Ok(None);
    };
    if session.continuation_state == ContinuationStateV1::Suggested {
        return Ok(None);
    }
    let Some(task_id) = session.task_id.as_deref().or(event.task_id.as_deref()) else {
        return Ok(None);
    };
    let Some(task) = store.get_task(task_id)? else {
        return Ok(None);
    };
    let mut reasons = Vec::new();
    if session.compaction_count >= crate::domain::PROVISIONAL_COMPACTION_THRESHOLD {
        reasons.push(ContinuationReasonV1::CompactionLimit);
    }
    if session
        .context_usage
        .as_ref()
        .and_then(crate::domain::ContextUsageV1::utilization)
        .is_some_and(|ratio| ratio >= crate::domain::PROVISIONAL_CONTEXT_USAGE_THRESHOLD)
    {
        reasons.push(ContinuationReasonV1::ContextUsageLimit);
    }
    let last_activity_at = session.last_activity_at.unwrap_or(session.started_at);
    if event.occurred_at.signed_duration_since(last_activity_at) >= chrono::Duration::hours(72) {
        let checkpoints = store.list_checkpoints(task_id)?;
        let files = store.list_file_changes(task_id)?;
        let baseline = checkpoints.last().map(|checkpoint| &checkpoint.git_after);
        let validation_root = baseline
            .map(|snapshot| snapshot.root.as_str())
            .filter(|path| !path.is_empty())
            .or_else(|| event.payload.get("repository_path").and_then(Value::as_str))
            .unwrap_or(&event.repository_id);
        if crate::git::revalidate_task(validation_root, baseline, &files)
            .map(|result| {
                matches!(
                    result.status,
                    TemporalStatusV1::Changed
                        | TemporalStatusV1::Diverged
                        | TemporalStatusV1::Broken
                )
            })
            .unwrap_or(false)
        {
            reasons.push(ContinuationReasonV1::OldSessionCodeChanged);
        }
    }
    if reasons.is_empty() {
        return Ok(None);
    }
    Ok(Some(ProposedContinuation {
        advice: ContinuationAdviceV1 {
            action: "new_thread".to_string(),
            reasons,
            task_id: task.id,
            task_title: task.title,
            last_activity_at,
            compaction_count: session.compaction_count,
            context_usage: session.context_usage,
        },
        claim_generation: "initial".to_string(),
    }))
}

pub(super) fn claim_continuation_suggestion(
    store: &Store,
    source: &EventEnvelopeV1,
    advice: &ContinuationAdviceV1,
    claim_generation: &str,
) -> Result<Option<ContinuationAdviceV1>> {
    let occurred_at = store
        .list_session_events(&source.repository_id, &source.session_id)?
        .into_iter()
        .find(|event| event.event_id == claim_generation)
        .and_then(|pending| {
            pending
                .occurred_at
                .checked_add_signed(chrono::Duration::microseconds(1))
        })
        .unwrap_or(source.occurred_at);
    let mut event = EventEnvelopeV1::new(
        format!(
            "continuation-suggested:{}:{}:v1",
            source.session_id, claim_generation
        ),
        &source.repository_id,
        &source.session_id,
        EventKind::ContinuationSuggested,
        occurred_at,
        json!({
            "continuation_advice": advice,
            "triggering_source_id": source.source_id,
            "delivery_state": "claimed",
            // This is an idempotency generation identifier, not a credential. Avoid a `token`
            // field name so the security scrubber can continue treating all token-shaped fields
            // as secrets without destroying delivery state.
            "claim_generation": claim_generation,
            "repository_path": source.payload.get("repository_path")
        }),
    );
    let claim_id = deterministic_id(
        "continuation-suggestion-claim",
        &[&source.repository_id, &source.session_id, claim_generation],
    );
    event.event_id = claim_id.clone();
    event.dedupe_key = claim_id;
    event.task_id = Some(advice.task_id.clone());
    match store.insert_event(&event)? {
        crate::store::InsertOutcome::Inserted => Ok(Some(advice.clone())),
        crate::store::InsertOutcome::Duplicate => {
            Ok(continuation_advice_for_source(store, source)?.map(|proposed| proposed.advice))
        }
    }
}

fn continuation_advice_for_source(
    store: &Store,
    source: &EventEnvelopeV1,
) -> Result<Option<ProposedContinuation>> {
    let events = store.list_session_events(&source.repository_id, &source.session_id)?;
    let latest_rearm = events.iter().rev().find_map(|event| {
        (event.kind == EventKind::ContinuationSuggested
            && event.payload.get("delivery_state").and_then(Value::as_str)
                == Some("pending_replay"))
        .then(|| event.event_id.clone())
    });
    Ok(events
        .into_iter()
        .find(|event| {
            event.kind == EventKind::ContinuationSuggested
                && event
                    .payload
                    .get("delivery_state")
                    .and_then(Value::as_str)
                    .unwrap_or("claimed")
                    == "claimed"
                && event
                    .payload
                    .get("triggering_source_id")
                    .and_then(Value::as_str)
                    == Some(source.source_id.as_str())
                && latest_rearm.as_deref().map_or_else(
                    || {
                        event
                            .payload
                            .get("claim_generation")
                            .and_then(Value::as_str)
                            .unwrap_or("initial")
                            == "initial"
                    },
                    |rearm| {
                        event
                            .payload
                            .get("claim_generation")
                            .and_then(Value::as_str)
                            == Some(rearm)
                    },
                )
        })
        .and_then(|event| {
            let advice =
                serde_json::from_value(event.payload.get("continuation_advice")?.clone()).ok()?;
            let claim_generation = event
                .payload
                .get("claim_generation")
                .and_then(Value::as_str)
                .unwrap_or("initial")
                .to_string();
            Some(ProposedContinuation {
                advice,
                claim_generation,
            })
        }))
}

fn pending_continuation_advice(
    store: &Store,
    source: &EventEnvelopeV1,
) -> Result<Option<ProposedContinuation>> {
    let events = store.list_session_events(&source.repository_id, &source.session_id)?;
    let consumed = events
        .iter()
        .filter(|event| event.kind == EventKind::ContinuationSuggested)
        .filter_map(|event| {
            (event.payload.get("delivery_state").and_then(Value::as_str) == Some("claimed"))
                .then(|| {
                    event
                        .payload
                        .get("claim_generation")?
                        .as_str()
                        .map(str::to_string)
                })
                .flatten()
        })
        .collect::<BTreeSet<_>>();
    Ok(events.into_iter().rev().find_map(|event| {
        if event.kind != EventKind::ContinuationSuggested
            || event.payload.get("delivery_state").and_then(Value::as_str) != Some("pending_replay")
            || consumed.contains(&event.event_id)
        {
            return None;
        }
        let advice =
            serde_json::from_value(event.payload.get("continuation_advice")?.clone()).ok()?;
        Some(ProposedContinuation {
            advice,
            claim_generation: event.event_id,
        })
    }))
}

pub(super) fn rearm_continuation_after_discarded_ack(
    store: &Store,
    source: &EventEnvelopeV1,
    advice: &ContinuationAdviceV1,
) -> Result<()> {
    let rearm_id = deterministic_id(
        "continuation-suggestion-rearm",
        &[
            &source.repository_id,
            &source.session_id,
            &source.dedupe_key,
        ],
    );
    let mut event = EventEnvelopeV1::new(
        format!("continuation-rearmed:{}:v1", source.source_id),
        &source.repository_id,
        &source.session_id,
        EventKind::ContinuationSuggested,
        source
            .occurred_at
            .checked_add_signed(chrono::Duration::microseconds(1))
            .unwrap_or(source.occurred_at),
        json!({
            "continuation_advice": advice,
            "delivery_state": "pending_replay",
            "discarded_source_id": source.source_id,
            "repository_path": source.payload.get("repository_path")
        }),
    );
    event.event_id = rearm_id.clone();
    event.dedupe_key = rearm_id;
    event.task_id = Some(advice.task_id.clone());
    store.insert_event(&event)?;
    Ok(())
}
