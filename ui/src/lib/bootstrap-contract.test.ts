import { afterEach, describe, expect, it, vi } from 'vitest';
import activeFixture from '../../../fixtures/bootstrap/active.json';
import unregisteredFixture from '../../../fixtures/bootstrap/unregistered.json';
import type { BootstrapData } from '../types';
import { fetchBootstrap } from './api';

describe('bootstrap JSON fixtures', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('consumes the unregistered fixture through BootstrapData', async () => {
    installFixture(unregisteredFixture);

    const bootstrap: BootstrapData = await fetchBootstrap();

    expect(bootstrap.repository.state).toBe('unregistered');
    expect(bootstrap.contractEvaluation).toBeNull();
    expect(bootstrap.tasks).toEqual([]);
  });

  it('consumes the active fixture and its nested BootstrapData projections', async () => {
    installFixture(activeFixture);

    const bootstrap: BootstrapData = await fetchBootstrap();

    expect(bootstrap.repository.state).toBe('active');
    expect(bootstrap.tasks[0]?.codebase.currentSha).toHaveLength(40);
    expect(bootstrap.checkpoints[0]?.temporalRevalidation?.status).toBe('unchanged');
    expect(bootstrap.contextPacks['task-active']?.task_id).toBe('task-active');
  });
});

function installFixture(fixture: unknown) {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
    ok: true,
    json: async () => fixture,
  }));
}
