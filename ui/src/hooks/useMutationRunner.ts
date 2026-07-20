import { useCallback, useState } from 'react';

export type PerformMutation = <T>(mutation: () => Promise<T>) => Promise<T | null>;

export function useMutationRunner(blocked: boolean) {
  const [mutationPending, setMutationPending] = useState(false);
  const [actionError, setActionError] = useState('');

  const performMutation: PerformMutation = useCallback(async <T,>(
    mutation: () => Promise<T>,
  ): Promise<T | null> => {
    if (blocked || mutationPending) return null;
    setMutationPending(true);
    setActionError('');
    try {
      return await mutation();
    } catch (error) {
      console.error('PreviouslyOn mutation failed', error);
      setActionError(error instanceof Error ? error.message : 'The local change could not be saved.');
      return null;
    } finally {
      setMutationPending(false);
    }
  }, [blocked, mutationPending]);

  return {
    mutationPending,
    setMutationPending,
    actionError,
    setActionError,
    performMutation,
  };
}
