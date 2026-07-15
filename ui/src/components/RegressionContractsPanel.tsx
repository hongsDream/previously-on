import { useState } from 'react';
import { AlertTriangle, CheckCircle2, CirclePlus, Edit3, FileCheck2, ShieldCheck, X } from 'lucide-react';
import type {
  ContractEvaluationV1,
  ContractImpactSelectorV1,
  ContractRequiredTestV1,
  RegressionCandidateDraftV1,
  RegressionCandidateV1,
  RegressionContractV1,
  RequiredTestStatus,
} from '../types';

interface RegressionContractsPanelProps {
  contracts: RegressionContractV1[];
  candidates: RegressionCandidateV1[];
  evaluation: ContractEvaluationV1 | null;
  disabled: boolean;
  mutationPending: boolean;
  onCreateCandidate: (candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onUpdateCandidate: (id: string, candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onApproveCandidate: (id: string) => Promise<boolean>;
  onSupersedeContract: (id: string, supersededBy: string) => Promise<boolean>;
}

export function RegressionContractsPanel({
  contracts,
  candidates,
  evaluation,
  disabled,
  mutationPending,
  onCreateCandidate,
  onUpdateCandidate,
  onApproveCandidate,
  onSupersedeContract,
}: RegressionContractsPanelProps) {
  const [editorCandidateId, setEditorCandidateId] = useState<string | 'new' | null>(null);
  const activeContracts = contracts.filter((contract) => contract.status === 'active');
  const pendingCandidates = candidates.filter((candidate) => candidate.status === 'pending');
  const relevantContracts = evaluation?.relevantContracts ?? [];
  const requiredTests = evaluation?.requiredTests ?? [];
  const warnings = evaluation?.warnings ?? [];
  const editingCandidate = editorCandidateId && editorCandidateId !== 'new'
    ? pendingCandidates.find((candidate) => candidate.id === editorCandidateId)
    : undefined;
  const isBlocked = evaluation?.readiness === 'contract_blocked';

  const saveCandidate = async (draft: RegressionCandidateDraftV1) => {
    const saved = editingCandidate
      ? await onUpdateCandidate(editingCandidate.id, draft)
      : await onCreateCandidate(draft);
    if (saved) setEditorCandidateId(null);
    return saved;
  };

  return (
    <section className="contract-panel" aria-labelledby="regression-contracts-title">
      <header className="contract-panel-header">
        <div className="contract-panel-title">
          <ShieldCheck size={18} aria-hidden="true" />
          <div>
            <h2 id="regression-contracts-title">Regression contracts</h2>
            <p>Team-shared bug history and required verification from this checkout.</p>
          </div>
        </div>
        <button
          className="secondary-button"
          type="button"
          disabled={disabled || mutationPending || editorCandidateId !== null}
          onClick={() => setEditorCandidateId('new')}
        >
          <CirclePlus size={15} /> New candidate
        </button>
      </header>

      <div className={`contract-readiness readiness-${evaluation?.readiness ?? 'unknown'}`} role="status" aria-live="polite">
        {isBlocked ? <AlertTriangle size={18} aria-hidden="true" /> : <CheckCircle2 size={18} aria-hidden="true" />}
        <div>
          <strong>{isBlocked ? 'Not ready to complete' : evaluation?.readiness === 'ready' ? 'Ready to complete' : 'Readiness unavailable'}</strong>
          <span>
            {isBlocked
              ? 'One or more relevant required tests are missing, stale, or failing.'
              : evaluation?.readiness === 'ready'
                ? 'All relevant required tests passed after the latest related change.'
                : 'No contract evaluation has been recorded for this checkout.'}
          </span>
        </div>
      </div>

      {relevantContracts.length > 0 ? (
        <section className="contract-section" aria-labelledby="relevant-contracts-title">
          <header><h3 id="relevant-contracts-title">Relevant to current changes</h3><span>{relevantContracts.length}</span></header>
          <div className="contract-match-list">
            {relevantContracts.map((match) => (
              <article key={match.id} className="contract-match">
                <strong>{match.title}</strong>
                <p>{match.invariant}</p>
                <ul aria-label="Selector match reasons">
                  {match.matchReasons.map((reason) => <li key={reason}>{reason}</li>)}
                </ul>
              </article>
            ))}
          </div>
        </section>
      ) : null}

      <section className="contract-section" aria-labelledby="required-tests-title">
        <header><h3 id="required-tests-title">Required tests</h3><span>{requiredTests.length}</span></header>
        {requiredTests.length === 0 ? (
          <p className="contract-empty-copy">No required tests are relevant to the current changes.</p>
        ) : (
          <ul className="required-test-list">
            {requiredTests.map((test) => (
              <li key={`${test.contractId}-${test.testId}`}>
                <TestState status={test.state} />
                <span><strong>{test.name}</strong>{test.detail ? <small>{test.detail}</small> : null}</span>
                <code>{[test.program, ...test.args].join(' ')}</code>
              </li>
            ))}
          </ul>
        )}
      </section>

      {warnings.length > 0 ? (
        <div className="contract-warnings" role="note">
          <strong>Evaluation warnings</strong>
          <ul>{warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
        </div>
      ) : null}

      {editorCandidateId ? (
        <CandidateEditor
          key={editorCandidateId}
          candidate={editingCandidate}
          disabled={disabled || mutationPending}
          onCancel={() => setEditorCandidateId(null)}
          onSave={saveCandidate}
        />
      ) : null}

      <div className="contract-grid">
        <section className="contract-section" aria-labelledby="contract-candidates-title">
          <header><h3 id="contract-candidates-title">Candidates</h3><span>{pendingCandidates.length}</span></header>
          {pendingCandidates.length === 0 ? <p className="contract-empty-copy">No candidates are awaiting review.</p> : (
            <div className="contract-card-list">
              {pendingCandidates.map((candidate) => (
                <article className="contract-card" key={candidate.id}>
                  <div className="contract-card-heading">
                    <div><small>{candidate.evidenceKind === 'manual' ? 'Manual candidate' : 'Evidence-based candidate'}</small><h4>{candidate.title}</h4></div>
                    <span className="candidate-state">Awaiting review</span>
                  </div>
                  <p>{candidate.invariant}</p>
                  <SelectorSummary selectors={candidate.impactSelectors} />
                  <RequiredTestSummary tests={candidate.requiredTests} />
                  <div className="contract-card-actions">
                    <button className="secondary-button" type="button" disabled={disabled || mutationPending} onClick={() => setEditorCandidateId(candidate.id)} aria-label={`Edit ${candidate.title}`}><Edit3 size={14} /> Edit</button>
                    <button className="primary-button" type="button" disabled={disabled || mutationPending} onClick={() => void onApproveCandidate(candidate.id)} aria-label={`Approve ${candidate.title}`}><FileCheck2 size={14} /> Approve</button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>

        <section className="contract-section" aria-labelledby="active-contracts-title">
          <header><h3 id="active-contracts-title">Git contracts</h3><span>{contracts.length}</span></header>
          {contracts.length === 0 ? <p className="contract-empty-copy">No Git contracts are active in this checkout.</p> : (
            <div className="contract-card-list">
              {contracts.map((contract) => (
                <ContractCard
                  key={contract.id}
                  contract={contract}
                  replacements={activeContracts.filter((candidate) => candidate.id !== contract.id)}
                  disabled={disabled || mutationPending}
                  onSupersede={onSupersedeContract}
                />
              ))}
            </div>
          )}
        </section>
      </div>
    </section>
  );
}

function ContractCard({ contract, replacements, disabled, onSupersede }: {
  contract: RegressionContractV1;
  replacements: RegressionContractV1[];
  disabled: boolean;
  onSupersede: (id: string, supersededBy: string) => Promise<boolean>;
}) {
  const [replacementId, setReplacementId] = useState(replacements[0]?.id ?? '');

  return (
    <article className={`contract-card contract-${contract.status}`}>
      <div className="contract-card-heading">
        <div><small>Git contract · {contract.id}</small><h4>{contract.title}</h4></div>
        <span className={`contract-state contract-state-${contract.status}`}>{contract.status}</span>
      </div>
      <p>{contract.invariant}</p>
      <SelectorSummary selectors={contract.impactSelectors} />
      <RequiredTestSummary tests={contract.requiredTests} />
      {contract.status === 'active' ? (
        <div className="contract-supersede">
          <label htmlFor={`replacement-${contract.id}`}>Replacement</label>
          <select id={`replacement-${contract.id}`} aria-label={`Replacement for ${contract.title}`} value={replacementId} disabled={disabled || replacements.length === 0} onChange={(event) => setReplacementId(event.target.value)}>
            {replacements.length === 0 ? <option value="">No replacement available</option> : replacements.map((replacement) => <option key={replacement.id} value={replacement.id}>{replacement.title}</option>)}
          </select>
          <button className="secondary-button" type="button" disabled={disabled || !replacementId} onClick={() => void onSupersede(contract.id, replacementId)} aria-label={`Supersede ${contract.title}`}>Supersede</button>
        </div>
      ) : contract.supersededBy ? <small className="superseded-copy">Superseded by {contract.supersededBy}</small> : null}
    </article>
  );
}

function CandidateEditor({ candidate, disabled, onCancel, onSave }: {
  candidate?: RegressionCandidateV1;
  disabled: boolean;
  onCancel: () => void;
  onSave: (candidate: RegressionCandidateDraftV1) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState<RegressionCandidateDraftV1>(() => candidate ? candidateToDraft(candidate) : emptyDraft());
  const [validationError, setValidationError] = useState('');

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const error = validateDraft(draft);
    if (error) {
      setValidationError(error);
      return;
    }
    setValidationError('');
    await onSave(draft);
  };

  return (
    <form className="candidate-editor" onSubmit={(event) => void submit(event)} aria-label={candidate ? `Edit ${candidate.title}` : 'Create manual contract candidate'}>
      <header><div><h3>{candidate ? 'Edit candidate' : 'Manual candidate'}</h3><p>Store only a redacted invariant, selectors, and argv-based required tests.</p></div><button className="icon-button" type="button" onClick={onCancel} aria-label="Close candidate editor"><X size={17} /></button></header>
      {validationError ? <div className="candidate-error" role="alert">{validationError}</div> : null}
      <fieldset disabled={disabled}>
        <label>Title<input value={draft.title} onChange={(event) => setDraft((current) => ({ ...current, title: event.target.value }))} /></label>
        <label>Invariant<textarea rows={3} value={draft.invariant} onChange={(event) => setDraft((current) => ({ ...current, invariant: event.target.value }))} /></label>

        <div className="editor-group-heading"><strong>Impact selectors</strong><button className="secondary-button" type="button" onClick={() => setDraft((current) => ({ ...current, impactSelectors: [...current.impactSelectors, emptySelector()] }))}>Add selector</button></div>
        {draft.impactSelectors.map((selector, index) => (
          <div className="selector-editor-row" key={`selector-${index}`}>
            <label>Path match<select value={selector.path.kind} onChange={(event) => updateSelector(index, { ...selector, path: { ...selector.path, kind: event.target.value as 'exact' | 'prefix' } })}><option value="exact">Exact</option><option value="prefix">Prefix</option></select></label>
            <label>Git path<input value={selector.path.value} onChange={(event) => updateSelector(index, { ...selector, path: { ...selector.path, value: event.target.value } })} /></label>
            <label>Literal symbols<input value={selector.symbols.join(', ')} placeholder="AuthContext, tenantId" onChange={(event) => updateSelector(index, { ...selector, symbols: event.target.value.split(',').map((value) => value.trim()).filter(Boolean) })} /></label>
            <button className="icon-button" type="button" disabled={draft.impactSelectors.length === 1} onClick={() => setDraft((current) => ({ ...current, impactSelectors: current.impactSelectors.filter((_, candidateIndex) => candidateIndex !== index) }))} aria-label={`Remove selector ${index + 1}`}><X size={15} /></button>
          </div>
        ))}

        <div className="editor-group-heading"><strong>Required tests</strong><button className="secondary-button" type="button" onClick={() => setDraft((current) => ({ ...current, requiredTests: [...current.requiredTests, emptyRequiredTest()] }))}>Add test</button></div>
        {draft.requiredTests.map((test, index) => (
          <div className="test-editor-row" key={test.id}>
            <label>Test name<input value={test.name} onChange={(event) => updateTest(index, { ...test, name: event.target.value })} /></label>
            <label>Program<input value={test.program} onChange={(event) => updateTest(index, { ...test, program: event.target.value })} /></label>
            <label>Arguments (one per line)<textarea rows={2} value={test.args.join('\n')} onChange={(event) => updateTest(index, { ...test, args: event.target.value.split('\n').filter((value) => value.length > 0) })} /></label>
            <label>Working directory<input value={test.workingDirectory} onChange={(event) => updateTest(index, { ...test, workingDirectory: event.target.value })} /></label>
            <label>Timeout seconds<input type="number" min={1} max={3600} value={test.timeoutSeconds} onChange={(event) => updateTest(index, { ...test, timeoutSeconds: Number(event.target.value) })} /></label>
            <button className="icon-button" type="button" disabled={draft.requiredTests.length === 1} onClick={() => setDraft((current) => ({ ...current, requiredTests: current.requiredTests.filter((_, candidateIndex) => candidateIndex !== index) }))} aria-label={`Remove required test ${index + 1}`}><X size={15} /></button>
          </div>
        ))}
      </fieldset>
      <footer><button className="secondary-button" type="button" onClick={onCancel}>Cancel</button><button className="primary-button" type="submit" disabled={disabled}>{candidate ? 'Save candidate' : 'Create candidate'}</button></footer>
    </form>
  );

  function updateSelector(index: number, selector: ContractImpactSelectorV1) {
    setDraft((current) => ({ ...current, impactSelectors: current.impactSelectors.map((item, candidateIndex) => candidateIndex === index ? selector : item) }));
  }

  function updateTest(index: number, test: ContractRequiredTestV1) {
    setDraft((current) => ({ ...current, requiredTests: current.requiredTests.map((item, candidateIndex) => candidateIndex === index ? test : item) }));
  }
}

function SelectorSummary({ selectors }: { selectors: ContractImpactSelectorV1[] }) {
  return <ul className="selector-summary" aria-label="Impact selectors">{selectors.map((selector, index) => <li key={`${selector.path.kind}-${selector.path.value}-${index}`}><code>{selector.path.kind}:{selector.path.value}</code>{selector.symbols.length > 0 ? <span>symbols: {selector.symbols.join(', ')}</span> : null}</li>)}</ul>;
}

function RequiredTestSummary({ tests }: { tests: ContractRequiredTestV1[] }) {
  return <ul className="contract-test-summary" aria-label="Contract required tests">{tests.map((test) => <li key={test.id}><strong>{test.name}</strong><code>{formatCommand(test)}</code></li>)}</ul>;
}

function TestState({ status }: { status: RequiredTestStatus }) {
  return <span className={`test-state test-state-${status}`}>{status}</span>;
}

function formatCommand(test: ContractRequiredTestV1) {
  return [test.program, ...test.args].join(' ');
}

function emptySelector(): ContractImpactSelectorV1 {
  return { path: { kind: 'exact', value: '' }, symbols: [] };
}

function emptyRequiredTest(): ContractRequiredTestV1 {
  const suffix = globalThis.crypto?.randomUUID?.() ?? String(Date.now());
  return { id: `manual-${suffix}`, name: '', program: '', args: [], workingDirectory: '.', timeoutSeconds: 900 };
}

function emptyDraft(): RegressionCandidateDraftV1 {
  return { title: '', invariant: '', impactSelectors: [emptySelector()], requiredTests: [emptyRequiredTest()] };
}

function candidateToDraft(candidate: RegressionCandidateV1): RegressionCandidateDraftV1 {
  return {
    title: candidate.title,
    invariant: candidate.invariant,
    impactSelectors: candidate.impactSelectors.map((selector) => ({ path: { ...selector.path }, symbols: [...selector.symbols] })),
    requiredTests: candidate.requiredTests.map((test) => ({ ...test, args: [...test.args] })),
  };
}

function validateDraft(draft: RegressionCandidateDraftV1) {
  if (!draft.title.trim()) return 'Title is required.';
  if (!draft.invariant.trim()) return 'Invariant is required.';
  if (draft.impactSelectors.some((selector) => !selector.path.value.trim())) return 'Every selector requires a Git path.';
  if (draft.requiredTests.some((test) => !test.name.trim() || !test.program.trim() || !test.workingDirectory.trim())) return 'Every required test needs a name, program, and working directory.';
  if (draft.requiredTests.some((test) => !Number.isInteger(test.timeoutSeconds) || test.timeoutSeconds < 1 || test.timeoutSeconds > 3600)) return 'Test timeouts must be whole seconds from 1 through 3600.';
  return '';
}
