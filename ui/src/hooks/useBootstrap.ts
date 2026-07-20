import { useCallback, useEffect, useState } from 'react';
import { fallbackData } from '../data/fallback';
import { ApiUnavailableError, fetchBootstrap } from '../lib/api';
import {
  emptyWorkspaceSelection,
  normalizeBootstrap,
  resolveTaskSelection,
  type WorkspaceSelectionIds,
} from '../lib/workspace';
import type { BootstrapData } from '../types';

export function useBootstrap() {
  const [data, setData] = useState<BootstrapData | null>(null);
  const [offlineFallback, setOfflineFallback] = useState(false);
  const [fatalError, setFatalError] = useState('');
  const [selection, setSelection] = useState<WorkspaceSelectionIds>(emptyWorkspaceSelection);

  const installBootstrap = useCallback((
    next: BootstrapData,
    preferred: Partial<WorkspaceSelectionIds> = emptyWorkspaceSelection,
  ) => {
    const normalized = normalizeBootstrap(next);
    const resolved = resolveTaskSelection(
      normalized,
      preferred.taskId,
      preferred.checkpointId,
      preferred.evidenceId,
    );
    setData(normalized);
    setSelection(resolved);
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    fetchBootstrap(controller.signal)
      .then((bootstrap) => installBootstrap(bootstrap))
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'AbortError') return;
        if (!(error instanceof ApiUnavailableError)) {
          setFatalError(error instanceof Error ? error.message : 'The local API returned an invalid response.');
          return;
        }
        setOfflineFallback(true);
        installBootstrap(fallbackData);
      });
    return () => controller.abort();
  }, [installBootstrap]);

  return {
    data,
    setData,
    offlineFallback,
    fatalError,
    selection,
    setSelection,
    installBootstrap,
  };
}
