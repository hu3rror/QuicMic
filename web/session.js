/**
 * QuicMic Client — pure session-persistence decision logic.
 *
 * No DOM, no fetch, no timers: everything here is a pure function over plain
 * inputs, so the entry-routing / wait-state / storage-fallback decisions can be
 * unit-tested with Node's built-in test runner (`node --test web/session.test.js`)
 * without a browser. app.js is the only caller and keeps all the glue (fetch,
 * DOM, timers, transports).
 *
 * Loads both as a classic browser script (`window.QuicMicSession`, loaded before
 * app.js) and as a Node module (`module.exports`) for the tests.
 */
(function (root, factory) {
    if (typeof module === 'object' && module.exports) {
        module.exports = factory(); // Node (unit tests)
    } else {
        root.QuicMicSession = factory(); // Browser
    }
}(typeof self !== 'undefined' ? self : this, function () {
    'use strict';

    /**
     * Fail-open wrapper around a Storage-like object (localStorage). Quota and
     * security exceptions must never escape: they would be misreported as a dead
     * server and dead-end the pairing flow (issue #3). When storage throws, the
     * value degrades to an in-memory fallback for this session only; within a
     * session, the most recent write wins regardless of which layer holds it.
     */
    function createSafeStorage(storage) {
        const memory = new Map();
        return {
            get(key) {
                if (memory.has(key)) return memory.get(key);
                try {
                    return storage.getItem(key);
                } catch (e) {
                    return null;
                }
            },
            set(key, value) {
                memory.set(key, value);
                try {
                    storage.setItem(key, value);
                } catch (e) {
                    // Session-only fallback; the in-memory copy above stands in.
                }
            },
            remove(key) {
                memory.delete(key);
                try {
                    storage.removeItem(key);
                } catch (e) {
                    // Best effort — storage may keep a stale value until it works again.
                }
            },
        };
    }

    /**
     * Map an HTTP validation outcome to the domain verdict used by every
     * entry/health/wait path (ADR-0009):
     *  - 200      -> 'ok'          the token is still valid
     *  - 401      -> 'taken-over'  server alive, the session was rotated away
     *  - anything else (503 during shutdown, other non-2xx) or a network
     *    failure -> 'gone'         treat as "server unavailable"
     */
    function interpretValidate(status) {
        if (status === 200) return 'ok';
        if (status === 401) return 'taken-over';
        return 'gone';
    }

    /**
     * Which entry route a page load takes (issue #3):
     *  - 'pair': a QR hash is present — explicit pairing intent; the stored
     *    token must be discarded before pairing.
     *  - 'resume': no hash but a stored token — validate it (never rotate it).
     *  - 'pairing-screen': no credential at all.
     */
    function resolveEntry({ hasHash, storedToken }) {
        if (hasHash) return 'pair';
        if (storedToken) return 'resume';
        return 'pairing-screen';
    }

    // Bounded wait for a vanished server: re-validation cadence and cap. After
    // the cap the client stops waiting and falls back to the reload-required
    // lock (issue #3, spec decision 5).
    const WAIT_RETRY_MS = 3000;
    const WAIT_CAP_MS = 60000;

    /**
     * The bounded-wait state machine, evaluated on each ~3s re-validation tick:
     *  - 'ok'         -> 'enter'   the server is back; resume into the main screen
     *  - 'taken-over' -> 'pair'    the server is back but our token is stale: re-pair in place
     *  - 'gone' past the cap       -> 'lock'   stop waiting; reload-required
     *  - 'gone' within the cap     -> 'retry'  keep waiting
     */
    function nextWaitStep({ startedAt, lastVerdict, now }) {
        if (lastVerdict === 'ok') return 'enter';
        if (lastVerdict === 'taken-over') return 'pair';
        if (now - startedAt >= WAIT_CAP_MS) return 'lock';
        return 'retry';
    }

    return {
        WAIT_RETRY_MS,
        WAIT_CAP_MS,
        createSafeStorage,
        interpretValidate,
        resolveEntry,
        nextWaitStep,
    };
}));
