// Framework-neutral session/identity/status helpers lifted out of App.svelte.
//
// These carry the ambient seams App.svelte previously reached for inline (the
// page location, localStorage, the UUID source) as explicit parameters, so the
// shell injects the real `window.*`/`crypto.randomUUID` and the helpers stay
// testable and DOM-free. `isDiagnosticsOnlyStatus` is already pure.

/** True when a status string is one of the renderer-diagnostics-only notices. */
export function isDiagnosticsOnlyStatus(message: string): boolean {
  return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
}

/** The ws/wss base URL derived from the page location (scheme upgraded for https). */
export function wsBaseUrl(location: { protocol: string; host: string }): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${location.host}`;
}

/** A fresh per-session client id (ephemeral, not persisted). */
export function clientIdentity(randomUuid: () => string): string {
  return `shell-${randomUuid().slice(0, 8)}`;
}

// The stable per-user id, persisted in storage. A first visit mints one; a storage
// failure falls back to an ephemeral id so the session still has an identity.
export function userIdentity(
  storage: { getItem(key: string): string | null; setItem(key: string, value: string): void },
  randomUuid: () => string
): string {
  const key = "shape-ai-user-id";
  try {
    const existing = storage.getItem(key);
    if (existing) return existing;
    const fresh = `user-${randomUuid().slice(0, 8)}`;
    storage.setItem(key, fresh);
    return fresh;
  } catch {
    return `user-${randomUuid().slice(0, 8)}`;
  }
}
