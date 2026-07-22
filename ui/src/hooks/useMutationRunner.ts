import { useCallback, useState } from 'react';
import { toUiError, type UiError } from '../lib/api';

export type PerformMutation = <T>(mutation: () => Promise<T>) => Promise<T | null>;

export function useMutationRunner(blocked: boolean) {
  const [mutationPending, setMutationPending] = useState(false);
  const [actionError, setActionError] = useState<UiError | null>(null);

  const performMutation: PerformMutation = useCallback(async <T,>(
    mutation: () => Promise<T>,
  ): Promise<T | null> => {
    if (blocked || mutationPending) return null;
    setMutationPending(true);
    setActionError(null);
    try {
      return await mutation();
    } catch (error) {
      console.error('PreviouslyOn mutation failed', error);
      setActionError(toUiError(error, 'The local change could not be saved.'));
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
