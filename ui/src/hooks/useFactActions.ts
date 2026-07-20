import { useCallback } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import { revalidateFact, updateFact, updateFactStatus, updateSession } from '../lib/api';
import type { BootstrapData, Fact, FactStatus } from '../types';
import type { PerformMutation } from './useMutationRunner';

interface FactActionsOptions {
  selectedFact?: Fact;
  selectedEvidence?: BootstrapData['evidence'][number];
  offlineFallback: boolean;
  mutationPending: boolean;
  setData: Dispatch<SetStateAction<BootstrapData | null>>;
  performMutation: PerformMutation;
}

export function useFactActions({
  selectedFact,
  selectedEvidence,
  offlineFallback,
  mutationPending,
  setData,
  performMutation,
}: FactActionsOptions) {
  const updateStatus = useCallback(async (nextStatus: FactStatus, supersedesFactId?: string) => {
    if (offlineFallback || mutationPending || !selectedFact) return;
    const previousStatus = selectedFact.status;
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, status: nextStatus } : fact),
    } : current);
    const saved = await performMutation(() => updateFactStatus(selectedFact.id, nextStatus, supersedesFactId));
    if (saved === null) {
      setData((current) => current ? {
        ...current,
        facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, status: previousStatus } : fact),
      } : current);
    }
  }, [mutationPending, offlineFallback, performMutation, selectedFact, setData]);

  const updateContent = useCallback(async (content: string, deprecatedAfterCommit: string) => {
    if (offlineFallback || mutationPending || !selectedFact) return false;
    const previous = selectedFact;
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, text: content, deprecatedAfterCommit: deprecatedAfterCommit || undefined } : fact),
    } : current);
    const saved = await performMutation(() => updateFact(selectedFact.id, selectedFact.status, content, deprecatedAfterCommit));
    if (!saved) {
      setData((current) => current ? {
        ...current,
        facts: current.facts.map((fact) => fact.id === selectedFact.id ? previous : fact),
      } : current);
      return false;
    }
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? {
        ...fact,
        text: saved.text,
        updatedAt: saved.updatedAt,
        deprecatedAfterCommit: saved.deprecatedAfterCommit || undefined,
      } : fact),
    } : current);
    return true;
  }, [mutationPending, offlineFallback, performMutation, selectedFact, setData]);

  const setSessionExcluded = useCallback(async (excluded: boolean) => {
    if (offlineFallback || mutationPending || !selectedEvidence?.sessionId) return;
    const sessionId = selectedEvidence.sessionId;
    const saved = await performMutation(() => updateSession(sessionId, excluded));
    if (!saved) return;
    setData((current) => current ? {
      ...current,
      sessions: current.sessions.map((session) => session.id === sessionId ? { ...session, excluded: saved.excluded } : session),
      evidence: current.evidence.map((evidence) => evidence.sessionId === sessionId ? { ...evidence, excludedSession: saved.excluded } : evidence),
    } : current);
  }, [mutationPending, offlineFallback, performMutation, selectedEvidence, setData]);

  const revalidate = useCallback(async () => {
    if (offlineFallback || mutationPending || !selectedFact) return;
    const result = await performMutation(() => revalidateFact(selectedFact.id));
    if (!result) return;
    setData((current) => current ? {
      ...current,
      evidence: current.evidence.map((evidence) => selectedFact.evidenceIds.includes(evidence.id)
        ? { ...evidence, freshness: result.freshness }
        : evidence),
      facts: current.facts.map((fact) => fact.id === selectedFact.id
        ? { ...fact, updatedAt: result.validatedAt }
        : fact),
    } : current);
  }, [mutationPending, offlineFallback, performMutation, selectedFact, setData]);

  return { updateStatus, updateContent, setSessionExcluded, revalidate };
}
