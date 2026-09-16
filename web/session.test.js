/**
 * Unit tests for web/session.js — the pure session-persistence decision logic.
 *
 * Run with: node --test web/session.test.js   (Node built-in runner, zero deps)
 *
 * Tests exercise the module's public functions only (inputs → verdict/state);
 * no DOM, fetch, or timer internals are touched (issue #3, Testing Decisions).
 */
'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const session = require('./session.js');

/** A working in-memory Storage-like object. */
function memoryStorage() {
    const m = new Map();
    return {
        getItem: (k) => (m.has(k) ? m.get(k) : null),
        setItem: (k, v) => { m.set(k, String(v)); },
        removeItem: (k) => { m.delete(k); },
    };
}

/** A Storage-like object whose methods all throw (quota/security failures). */
function throwingStorage() {
    return {
        getItem: () => { throw new Error('quota'); },
        setItem: () => { throw new Error('quota'); },
        removeItem: () => { throw new Error('security'); },
    };
}

// ── createSafeStorage ─────────────────────────────────────────────────

test('safe storage round-trips through working storage', () => {
    const s = session.createSafeStorage(memoryStorage());
    s.set('sessionToken', 'abc123');
    assert.equal(s.get('sessionToken'), 'abc123');
    s.remove('sessionToken');
    assert.equal(s.get('sessionToken'), null);
});

test('a throwing setItem degrades to session-only memory without escaping', () => {
    const s = session.createSafeStorage(throwingStorage());
    assert.doesNotThrow(() => s.set('sessionToken', 'abc123'));
    assert.equal(s.get('sessionToken'), 'abc123');
});

test('a throwing getItem returns null without escaping', () => {
    const s = session.createSafeStorage(throwingStorage());
    assert.equal(s.get('sessionToken'), null);
});

test('a throwing removeItem never escapes', () => {
    const s = session.createSafeStorage(throwingStorage());
    assert.doesNotThrow(() => s.remove('sessionToken'));
});

test('null storage (blocked localStorage) still works in-memory', () => {
    const s = session.createSafeStorage(null);
    assert.doesNotThrow(() => s.set('sessionToken', 'abc123'));
    assert.equal(s.get('sessionToken'), 'abc123');
});

// ── interpretValidate ──────────────────────────────────────────────────

test('HTTP 200 means the token is valid (ok)', () => {
    assert.equal(session.interpretValidate(200), 'ok');
});

test('HTTP 401 means the session was taken over (taken-over)', () => {
    assert.equal(session.interpretValidate(401), 'taken-over');
});

test('HTTP 503 (shutdown) is treated as server gone', () => {
    assert.equal(session.interpretValidate(503), 'gone');
});

test('any other non-2xx status is treated as server gone', () => {
    assert.equal(session.interpretValidate(500), 'gone');
    assert.equal(session.interpretValidate(404), 'gone');
});

test('a network failure (no status) is treated as server gone', () => {
    assert.equal(session.interpretValidate(undefined), 'gone');
    assert.equal(session.interpretValidate(0), 'gone');
});

// ── resolveEntry ──────────────────────────────────────────────────────

test('a QR hash always means pairing, even with a stored token', () => {
    assert.equal(session.resolveEntry({ hasHash: true, storedToken: 'abc' }), 'pair');
    assert.equal(session.resolveEntry({ hasHash: true, storedToken: null }), 'pair');
});

test('a stored token without a hash resumes the session', () => {
    assert.equal(session.resolveEntry({ hasHash: false, storedToken: 'abc' }), 'resume');
});

test('no hash and no token lands on the pairing screen', () => {
    assert.equal(session.resolveEntry({ hasHash: false, storedToken: null }), 'pairing-screen');
    assert.equal(session.resolveEntry({ hasHash: false, storedToken: '' }), 'pairing-screen');
});

// ── nextWaitStep ──────────────────────────────────────────────────────

const RETRY = session.WAIT_RETRY_MS;
const CAP = session.WAIT_CAP_MS;

test('the server coming back resumes into the main screen', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'ok', now: 1000 });
    assert.equal(step, 'enter');
});

test('a taken-over session during the wait re-pairs in place', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'taken-over', now: 1000 });
    assert.equal(step, 'pair');
});

test('a taken-over session re-pairs in place even past the wait cap', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'taken-over', now: CAP + 999999 });
    assert.equal(step, 'pair');
});

test('a gone server within the cap keeps waiting', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'gone', now: CAP - 1 });
    assert.equal(step, 'retry');
});

test('a gone server at the cap falls back to the reload lock', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'gone', now: CAP });
    assert.equal(step, 'lock');
});

test('a gone server past the cap falls back to the reload lock', () => {
    const step = session.nextWaitStep({ startedAt: 0, lastVerdict: 'gone', now: CAP + RETRY });
    assert.equal(step, 'lock');
});

test('the wait cap counts from the wait start, not from each retry', () => {
    // Started 30s ago, retried every 3s: still retrying at 30s, locked at 60s.
    assert.equal(session.nextWaitStep({ startedAt: 1000, lastVerdict: 'gone', now: 1000 + 30000 }), 'retry');
    assert.equal(session.nextWaitStep({ startedAt: 1000, lastVerdict: 'gone', now: 1000 + 60000 }), 'lock');
});
