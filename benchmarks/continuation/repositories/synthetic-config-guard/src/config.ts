export class ConfigError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ConfigError';
  }
}

const ALLOWED_KEYS = new Set(['endpoint', 'safeMode']);

export function parseConfig(input = {}) {
  for (const key of Object.keys(input)) {
    if (!ALLOWED_KEYS.has(key)) throw new ConfigError(`unknown configuration key: ${key}`);
  }
  if ('safeMode' in input && typeof input.safeMode !== 'boolean') {
    throw new ConfigError('safeMode must be a boolean');
  }
  if ('endpoint' in input && typeof input.endpoint !== 'string') {
    throw new ConfigError('endpoint must be a string');
  }
  return {
    endpoint: input.endpoint ?? 'https://service.invalid',
    safeMode: input.safeMode ?? true,
  };
}
