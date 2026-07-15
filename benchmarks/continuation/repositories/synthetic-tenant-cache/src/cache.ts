const TTL_MS = 60_000;

export class TenantCache {
  constructor({ now = () => Date.now() } = {}) {
    this.entries = new Map();
    this.now = now;
  }

  async get(tenantId, resourceId, loader) {
    // This deliberately reflects the pre-challenge collision: tenantId is missing.
    const key = resourceId;
    const current = this.entries.get(key);
    if (current && current.expiresAt > this.now()) return current.value;
    const value = await loader({ tenantId, resourceId });
    this.entries.set(key, { value, expiresAt: this.now() + TTL_MS });
    return value;
  }
}

export { TTL_MS };
