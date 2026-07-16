import { useEffect, useMemo, useState } from 'react';
import { GitFork, List, Network } from 'lucide-react';
import { ApiUnavailableError, fetchRelationshipGraph } from '../lib/api';
import type { GraphNodeKindV1, GraphNodeV1, RelationshipGraphSummaryV1, RelationshipGraphV1, Task } from '../types';

interface RelationshipGraphPanelProps {
  repositoryId: string;
  tasks: Task[];
  summary: RelationshipGraphSummaryV1;
  refreshVersion: number;
  disabled: boolean;
}

const kindOrder: GraphNodeKindV1[] = ['task', 'session', 'commit', 'file', 'regression_contract', 'verified_symbol', 'test', 'agent'];

export function RelationshipGraphPanel({ repositoryId, tasks, summary, refreshVersion, disabled }: RelationshipGraphPanelProps) {
  const [taskFilter, setTaskFilter] = useState('');
  const [view, setView] = useState<'graph' | 'list'>(() => isCompactViewport() ? 'list' : 'graph');
  const [graph, setGraph] = useState<RelationshipGraphV1 | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!tasks.some((task) => task.id === taskFilter)) setTaskFilter('');
  }, [tasks, taskFilter]);

  useEffect(() => {
    const media = typeof window.matchMedia === 'function' ? window.matchMedia('(max-width: 900px)') : null;
    if (!media) return;
    const handleChange = (event: MediaQueryListEvent) => {
      if (event.matches) setView('list');
    };
    media.addEventListener?.('change', handleChange);
    return () => media.removeEventListener?.('change', handleChange);
  }, []);

  useEffect(() => {
    if (disabled || !repositoryId) {
      setGraph(null);
      setLoading(false);
      setError(disabled ? 'Relationship details are unavailable in the read-only sample workspace.' : 'Repository identity is unavailable.');
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setError('');
    fetchRelationshipGraph(repositoryId, taskFilter || undefined, controller.signal)
      .then(setGraph)
      .catch((caught: unknown) => {
        if (caught instanceof DOMException && caught.name === 'AbortError') return;
        const message = caught instanceof ApiUnavailableError
          ? 'The local relationship graph is temporarily unavailable.'
          : caught instanceof Error ? caught.message : 'The relationship graph could not be loaded.';
        setGraph(null);
        setError(message);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [disabled, refreshVersion, repositoryId, taskFilter]);

  return (
    <section className="overview-panel relationship-graph-panel" aria-labelledby="relationship-graph-title">
      <header>
        <span><GitFork size={17} /><strong id="relationship-graph-title">Verified relationship graph</strong></span>
        <small>{summary.nodeCount} nodes · {summary.verifiedEdgeCount}/{summary.edgeCount} verified edges</small>
      </header>
      <div className="graph-toolbar">
        <label htmlFor="relationship-task-filter">Task filter
          <select id="relationship-task-filter" value={taskFilter} disabled={disabled || loading} onChange={(event) => setTaskFilter(event.target.value)}>
            <option value="">All repository tasks</option>
            {tasks.map((task) => <option key={task.id} value={task.id}>{task.title}</option>)}
          </select>
        </label>
        <div className="graph-view-toggle" role="group" aria-label="Relationship graph view">
          <button type="button" aria-pressed={view === 'graph'} onClick={() => setView('graph')}><Network size={14} /> Graph</button>
          <button type="button" aria-pressed={view === 'list'} onClick={() => setView('list')}><List size={14} /> List</button>
        </div>
      </div>

      {loading ? <p className="graph-state" role="status">Loading verified relationships…</p> : null}
      {!loading && error ? <p className="graph-state graph-error" role="alert">{error}</p> : null}
      {!loading && !error && graph ? (
        graph.nodes.length || graph.edges.length ? (
          view === 'graph' ? <GraphVisual graph={graph} /> : <GraphList graph={graph} />
        ) : <p className="graph-state">No verified relationships match this filter.</p>
      ) : null}
    </section>
  );
}

function GraphVisual({ graph }: { graph: RelationshipGraphV1 }) {
  const layout = useMemo(() => graphLayout(graph.nodes), [graph.nodes]);
  const nodesById = useMemo(() => new Map(graph.nodes.map((node) => [node.id, node])), [graph.nodes]);
  return (
    <div className="graph-visual-wrap">
      <p className="sr-only">Visual overview of {graph.nodes.length} nodes and {graph.edges.length} explicit verified relationships. Use List view for complete relationship details.</p>
      <svg className="relationship-graph-visual" viewBox={`0 0 920 ${layout.height}`} role="img" aria-label={`Relationship graph with ${graph.nodes.length} nodes and ${graph.edges.length} edges`}>
        <g className="graph-edges" aria-hidden="true">
          {graph.edges.map((edge) => {
            const from = layout.positions.get(edge.from);
            const to = layout.positions.get(edge.to);
            if (!from || !to) return null;
            return <line key={edge.id} x1={from.x} y1={from.y} x2={to.x} y2={to.y} className={edge.verified ? 'verified' : 'unverified'}><title>{edgeLabel(edge.kind, nodesById.get(edge.from), nodesById.get(edge.to))}</title></line>;
          })}
        </g>
        <g className="graph-nodes" aria-hidden="true">
          {graph.nodes.map((node) => {
            const position = layout.positions.get(node.id)!;
            return (
              <g key={node.id} transform={`translate(${position.x - 47} ${position.y - 18})`} className={`graph-node graph-node-${node.kind.replaceAll('_', '-')}`}>
                <rect width="94" height="36" rx="6" />
                <text x="47" y="14" className="graph-node-kind">{kindLabel(node.kind)}</text>
                <text x="47" y="27"><title>{node.label}</title>{truncate(node.label, 15)}</text>
              </g>
            );
          })}
        </g>
      </svg>
      <p className="graph-visual-help">Only explicit canonical, projection, and Regression Contract edges are shown. No similarity inference is used.</p>
    </div>
  );
}

function GraphList({ graph }: { graph: RelationshipGraphV1 }) {
  const nodesById = new Map(graph.nodes.map((node) => [node.id, node]));
  return (
    <div className="graph-list-fallback">
      <section aria-labelledby="graph-node-list-title">
        <h3 id="graph-node-list-title">Nodes</h3>
        <ul className="graph-node-list">{graph.nodes.map((node) => <li key={node.id}><span>{kindLabel(node.kind)}</span><strong>{node.label}</strong><code title={node.id}>{node.id}</code></li>)}</ul>
      </section>
      <div className="graph-table-scroll">
        <table>
          <caption>Explicit relationship edges and provenance</caption>
          <thead><tr><th scope="col">Relationship</th><th scope="col">From</th><th scope="col">To</th><th scope="col">Provenance</th><th scope="col">Source</th><th scope="col">Observed</th><th scope="col">Verified</th></tr></thead>
          <tbody>{graph.edges.map((edge) => (
            <tr key={edge.id}>
              <th scope="row">{edge.kind}</th>
              <td><strong>{nodesById.get(edge.from)?.label ?? edge.from}</strong><code>{edge.from}</code></td>
              <td><strong>{nodesById.get(edge.to)?.label ?? edge.to}</strong><code>{edge.to}</code></td>
              <td><ul>{edge.provenanceIds.map((id) => <li key={id}><code>{id}</code></li>)}</ul></td>
              <td>{edge.sourceKind}</td>
              <td>{formatObservedAt(edge.observedAt)}</td>
              <td><span className={edge.verified ? 'graph-verified' : 'graph-unverified'}>{edge.verified ? 'Verified' : 'Unverified'}</span></td>
            </tr>
          ))}</tbody>
        </table>
      </div>
    </div>
  );
}

function graphLayout(nodes: GraphNodeV1[]) {
  const presentKinds = kindOrder.filter((kind) => nodes.some((node) => node.kind === kind));
  const positions = new Map<string, { x: number; y: number }>();
  const maxRows = Math.max(1, ...presentKinds.map((kind) => nodes.filter((node) => node.kind === kind).length));
  presentKinds.forEach((kind, column) => {
    const kindNodes = nodes.filter((node) => node.kind === kind);
    const x = presentKinds.length === 1 ? 460 : 62 + column * (796 / (presentKinds.length - 1));
    kindNodes.forEach((node, row) => positions.set(node.id, { x, y: 52 + row * 54 }));
  });
  return { positions, height: Math.max(150, 88 + maxRows * 54) };
}

function edgeLabel(kind: string, from?: GraphNodeV1, to?: GraphNodeV1) {
  return `${from?.label ?? 'Unknown'} ${kind} ${to?.label ?? 'Unknown'}`;
}

function kindLabel(kind: GraphNodeKindV1) {
  return kind.replaceAll('_', ' ');
}

function truncate(value: string, length: number) {
  return value.length > length ? `${value.slice(0, length - 1)}…` : value;
}

function formatObservedAt(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Unavailable' : date.toLocaleString();
}

function isCompactViewport() {
  return typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 900px)').matches;
}
