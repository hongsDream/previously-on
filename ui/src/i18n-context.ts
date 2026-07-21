import { createContext, useContext } from 'react';

export type Language = 'en' | 'ko';

export interface I18nValue {
  language: Language;
  locale: string;
  setLanguage: (language: Language) => void;
  t: (message: string, values?: Record<string, string | number>) => string;
}

export const I18nContext = createContext<I18nValue | null>(null);

export function useI18n() {
  const value = useContext(I18nContext);
  if (!value) throw new Error('useI18n must be used inside I18nProvider');
  return value;
}
