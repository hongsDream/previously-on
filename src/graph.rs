use crate::contracts::RegressionContractV1;
use crate::domain::{
    deterministic_id, GraphEdgeKindV1, GraphEdgeV1, GraphNodeKindV1, GraphNodeV1,
    GraphSourceKindV1, RelationshipGraphV1, SCHEMA_VERSION_V1,
};
use crate::redaction::redact_excerpt;
use crate::store::Store;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationshipGraphSummaryV1 {
    pub node_count: usize,
    pub edge_count: usize,
    /// Deprecated V1 compatibility field. It mirrors evidence-backed edges in V1.
    pub verified_edge_count: usize,
    pub nodes_by_kind: BTreeMap<String, usize>,
    pub edges_by_kind: BTreeMap<String, usize>,
}

pub fn derive_relationship_graph(
    store: &Store,
    repository_id: &str,
    task_filter: Option<&str>,
    contracts: &[RegressionContractV1],
) -> Result<RelationshipGraphV1> {
    let repositories = store.list_repositories()?;
    if !repositories.iter().any(|item| item.id == repository_id) {
        bail!("repository not found: {repository_id}");
    }
    if let Some(task_id) = task_filter {
        let task = store
            .get_task(task_id)?
            .with_context(|| format!("task not found: {task_id}"))?;
        if task.repository_id != repository_id {
            bail!("task does not belong to repository");
        }
    }

    let tasks = store
        .list_tasks(Some(repository_id))?
        .into_iter()
        .filter(|task| task_filter.is_none_or(|filter| task.id == filter))
        .collect::<Vec<_>>();
    let task_ids = tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<BTreeSet<_>>();
    let mut nodes = BTreeMap::<String, GraphNodeV1>::new();
    let mut edges = BTreeMap::<String, GraphEdgeV1>::new();

    for task in &tasks {
        insert_node(
            &mut nodes,
            GraphNodeV1 {
                id: node_id("task", &task.id),
                kind: GraphNodeKindV1::Task,
                label: redact_excerpt(&task.title),
                task_id: Some(task.id.clone()),
            },
        );
        for session in store.list_sessions_for_task(&task.id)? {
            let session_node = node_id("session", &session.id);
            insert_node(
                &mut nodes,
                GraphNodeV1 {
                    id: session_node.clone(),
                    kind: GraphNodeKindV1::Session,
                    label: redact_excerpt(
                        session
                            .source_thread_id
                            .as_deref()
                            .unwrap_or(session.id.as_str()),
                    ),
                    task_id: Some(task.id.clone()),
                },
            );
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::TaskHasSession,
                node_id("task", &task.id),
                session_node.clone(),
                vec![session.id.clone()],
                GraphSourceKindV1::Projection,
                session.started_at,
            );
            if let Some(head) = session.head.as_deref().filter(|head| !head.is_empty()) {
                let commit_node = node_id("commit", head);
                insert_node(
                    &mut nodes,
                    GraphNodeV1 {
                        id: commit_node.clone(),
                        kind: GraphNodeKindV1::Commit,
                        label: redact_excerpt(head),
                        task_id: Some(task.id.clone()),
                    },
                );
                insert_edge(
                    &mut edges,
                    GraphEdgeKindV1::SessionObservedCommit,
                    session_node.clone(),
                    commit_node,
                    vec![session.id.clone()],
                    GraphSourceKindV1::Projection,
                    session.last_activity_at.unwrap_or(session.started_at),
                );
            }
        }
        for checkpoint in store.list_checkpoints(&task.id)? {
            let session_node = node_id("session", &checkpoint.session_id);
            if let Some(head) = checkpoint
                .git_after
                .head
                .as_deref()
                .filter(|head| !head.is_empty())
            {
                let commit_node = node_id("commit", head);
                insert_node(
                    &mut nodes,
                    GraphNodeV1 {
                        id: commit_node.clone(),
                        kind: GraphNodeKindV1::Commit,
                        label: redact_excerpt(head),
                        task_id: Some(task.id.clone()),
                    },
                );
                insert_edge(
                    &mut edges,
                    GraphEdgeKindV1::SessionObservedCommit,
                    session_node.clone(),
                    commit_node,
                    vec![checkpoint.id.clone()],
                    GraphSourceKindV1::Projection,
                    checkpoint.created_at,
                );
            }
            for change in &checkpoint.changed_files {
                let path = redact_excerpt(&change.path);
                let file_node = node_id("file", &path);
                insert_node(
                    &mut nodes,
                    GraphNodeV1 {
                        id: file_node.clone(),
                        kind: GraphNodeKindV1::File,
                        label: path,
                        task_id: Some(task.id.clone()),
                    },
                );
                insert_edge(
                    &mut edges,
                    GraphEdgeKindV1::SessionChangedFile,
                    session_node.clone(),
                    file_node,
                    vec![checkpoint.id.clone()],
                    GraphSourceKindV1::Projection,
                    checkpoint.created_at,
                );
            }
        }
    }

    let agents = store
        .list_agents(Some(repository_id))?
        .into_iter()
        .filter(|agent| {
            task_filter.is_none()
                || agent
                    .task_id
                    .as_deref()
                    .is_some_and(|task_id| task_ids.contains(task_id))
        })
        .collect::<Vec<_>>();
    let observed_threads = agents
        .iter()
        .map(|agent| agent.thread_id.as_str())
        .collect::<BTreeSet<_>>();
    for agent in &agents {
        let agent_node = node_id("agent", &agent.thread_id);
        insert_node(
            &mut nodes,
            GraphNodeV1 {
                id: agent_node.clone(),
                kind: GraphNodeKindV1::Agent,
                label: redact_excerpt(&agent.name),
                task_id: agent.task_id.clone(),
            },
        );
        if let Some(task_id) = agent
            .task_id
            .as_deref()
            .filter(|task_id| task_ids.contains(*task_id))
        {
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::AgentWorkedOnTask,
                agent_node.clone(),
                node_id("task", task_id),
                vec![agent.id.clone(), agent.thread_id.clone()],
                GraphSourceKindV1::AgentObservation,
                agent.observed_at,
            );
        }
        if let Some(parent) = agent
            .parent_thread_id
            .as_deref()
            .filter(|parent| observed_threads.contains(*parent))
        {
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::AgentParent,
                node_id("agent", parent),
                agent_node,
                vec![agent.id.clone(), parent.to_string()],
                GraphSourceKindV1::AgentObservation,
                agent.observed_at,
            );
        }
    }

    let mut latest_evaluations = BTreeMap::new();
    for evaluation in store.list_contract_evaluations(Some(repository_id))? {
        let Some(task_id) = evaluation.task_id.as_deref() else {
            continue;
        };
        if !task_ids.contains(task_id) {
            continue;
        }
        latest_evaluations
            .entry(task_id.to_string())
            .or_insert(evaluation);
    }
    let relevant_contract_ids = latest_evaluations
        .values()
        .flat_map(|evaluation| {
            evaluation
                .relevant_contracts
                .iter()
                .map(|contract| contract.id.clone())
        })
        .collect::<BTreeSet<_>>();
    let included_contracts = contracts
        .iter()
        .filter(|contract| task_filter.is_none() || relevant_contract_ids.contains(&contract.id));
    for contract in included_contracts {
        let contract_node = node_id("contract", &contract.id);
        insert_node(
            &mut nodes,
            GraphNodeV1 {
                id: contract_node.clone(),
                kind: GraphNodeKindV1::RegressionContract,
                label: redact_excerpt(&contract.title),
                task_id: None,
            },
        );
        for selector in &contract.impact_selectors {
            let path = redact_excerpt(&selector.path.value);
            let file_node = node_id("file", &format!("{:?}:{path}", selector.path.kind));
            insert_node(
                &mut nodes,
                GraphNodeV1 {
                    id: file_node.clone(),
                    kind: GraphNodeKindV1::File,
                    label: path,
                    task_id: None,
                },
            );
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::ContractCoversFile,
                contract_node.clone(),
                file_node,
                vec![contract.id.clone(), contract.origin.evidence_sha256.clone()],
                GraphSourceKindV1::RegressionContract,
                contract.origin.recorded_at,
            );
            for symbol in &selector.symbols {
                let symbol = redact_excerpt(symbol);
                let symbol_node = node_id("symbol", &symbol);
                insert_node(
                    &mut nodes,
                    GraphNodeV1 {
                        id: symbol_node.clone(),
                        kind: GraphNodeKindV1::VerifiedSymbol,
                        label: symbol,
                        task_id: None,
                    },
                );
                insert_edge(
                    &mut edges,
                    GraphEdgeKindV1::ContractDeclaresSymbol,
                    contract_node.clone(),
                    symbol_node,
                    vec![contract.id.clone(), contract.origin.evidence_sha256.clone()],
                    GraphSourceKindV1::RegressionContract,
                    contract.origin.recorded_at,
                );
            }
        }
        for required_test in &contract.required_tests {
            let test_node = node_id("test", &format!("{}\0{}", contract.id, required_test.id));
            insert_node(
                &mut nodes,
                GraphNodeV1 {
                    id: test_node.clone(),
                    kind: GraphNodeKindV1::Test,
                    label: redact_excerpt(&required_test.name),
                    task_id: None,
                },
            );
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::ContractRequiresTest,
                contract_node.clone(),
                test_node,
                vec![contract.id.clone(), required_test.id.clone()],
                GraphSourceKindV1::RegressionContract,
                contract.origin.recorded_at,
            );
        }
    }
    for (task_id, evaluation) in latest_evaluations {
        for relevant in evaluation.relevant_contracts {
            if !contracts.iter().any(|contract| contract.id == relevant.id) {
                continue;
            }
            insert_edge(
                &mut edges,
                GraphEdgeKindV1::TaskRelevantContract,
                node_id("task", &task_id),
                node_id("contract", &relevant.id),
                vec![evaluation.id.clone(), relevant.id],
                GraphSourceKindV1::ContractEvaluation,
                evaluation.evaluated_at,
            );
        }
    }

    edges.retain(|_, edge| {
        !edge.provenance_ids.is_empty()
            && nodes.contains_key(&edge.from)
            && nodes.contains_key(&edge.to)
    });
    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| (&left.kind, &left.id).cmp(&(&right.kind, &right.id)));
    let mut edges = edges.into_values().collect::<Vec<_>>();
    edges.sort_by(|left, right| (&left.kind, &left.id).cmp(&(&right.kind, &right.id)));
    Ok(RelationshipGraphV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id: repository_id.to_string(),
        task_filter: task_filter.map(str::to_string),
        nodes,
        edges,
    })
}

pub fn compact_summary(graph: &RelationshipGraphV1) -> RelationshipGraphSummaryV1 {
    let mut summary = RelationshipGraphSummaryV1 {
        node_count: graph.nodes.len(),
        edge_count: graph.edges.len(),
        verified_edge_count: graph.edges.len(),
        ..RelationshipGraphSummaryV1::default()
    };
    for node in &graph.nodes {
        *summary
            .nodes_by_kind
            .entry(enum_label(&node.kind))
            .or_default() += 1;
    }
    for edge in &graph.edges {
        *summary
            .edges_by_kind
            .entry(enum_label(&edge.kind))
            .or_default() += 1;
    }
    summary
}

fn insert_node(nodes: &mut BTreeMap<String, GraphNodeV1>, node: GraphNodeV1) {
    nodes.entry(node.id.clone()).or_insert(node);
}

#[allow(clippy::too_many_arguments)]
fn insert_edge(
    edges: &mut BTreeMap<String, GraphEdgeV1>,
    kind: GraphEdgeKindV1,
    from: String,
    to: String,
    mut provenance_ids: Vec<String>,
    source_kind: GraphSourceKindV1,
    observed_at: chrono::DateTime<Utc>,
) {
    provenance_ids = provenance_ids
        .into_iter()
        .map(|value| redact_excerpt(&value))
        .filter(|value| !value.is_empty())
        .collect();
    provenance_ids.sort();
    provenance_ids.dedup();
    if provenance_ids.is_empty() {
        return;
    }
    let kind_wire = enum_label(&kind);
    let source_kind_wire = enum_label(&source_kind);
    let id = deterministic_id("edge", &[&kind_wire, &from, &to, &source_kind_wire]);
    edges
        .entry(id.clone())
        .and_modify(|edge| {
            edge.provenance_ids.extend(provenance_ids.clone());
            edge.provenance_ids.sort();
            edge.provenance_ids.dedup();
            edge.observed_at = edge.observed_at.max(observed_at);
        })
        .or_insert(GraphEdgeV1 {
            id,
            kind,
            from,
            to,
            provenance_ids,
            source_kind,
            observed_at,
            verified: true,
        });
}

fn node_id(kind: &str, value: &str) -> String {
    deterministic_id(kind, &[value])
}

fn enum_label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}
