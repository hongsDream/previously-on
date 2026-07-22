import type { Language } from '../i18n-context';

const STORAGE_KEY = 'previously-on:preferences:v1';

export interface BrowserPreferencesV1 {
  schemaVersion: 1;
  language?: Language;
  repositoryId?: string;
}

export function readPreferences(): BrowserPreferencesV1 {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null') as Partial<BrowserPreferencesV1> | null;
    if (stored?.schemaVersion !== 1) return { schemaVersion: 1 };
    return {
      schemaVersion: 1,
      language: stored.language === 'en' || stored.language === 'ko' ? stored.language : undefined,
      repositoryId: typeof stored.repositoryId === 'string' && stored.repositoryId.trim()
        ? stored.repositoryId
        : undefined,
    };
  } catch {
    return { schemaVersion: 1 };
  }
}

export function updatePreferences(update: Partial<Omit<BrowserPreferencesV1, 'schemaVersion'>>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...readPreferences(),
      ...update,
      schemaVersion: 1,
    }));
  } catch {
    // Preferences remain active for this browser session when storage is unavailable.
  }
}
