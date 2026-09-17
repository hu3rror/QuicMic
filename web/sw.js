/**
 * QuicMic — installability-only service worker (ADR-0018).
 *
 * Deliberately inert: it registers and installs, but never intercepts a
 * request and never touches the Cache API. Its only purpose is satisfying
 * Chromium's installability criteria for a future trusted-CA origin; under
 * the current self-signed certificate the origin is not a secure context,
 * so this worker is never even active. Adding caching here would silently
 * reintroduce the stale-asset class ADR-0011 exists to prevent — keep it
 * empty.
 */
self.addEventListener('install', () => {
    // Nothing to pre-cache; the app must always fetch fresh (ADR-0011).
});
self.addEventListener('fetch', () => {
    // No interception: every request stays on the normal network path with
    // ETag revalidation, so live-edited or rebuilt assets are picked up
    // immediately.
});