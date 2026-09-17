/**
 * Consistency tests for the installable home-screen entry (ADR-0018, issue #4).
 *
 * Run with: node --test web/pwa.test.js   (Node built-in runner, zero deps)
 *
 * These assert external configuration, not behavior: the manifest field set,
 * that every icon the manifest and the entry page reference actually exists as
 * a PNG on disk, that the page links the manifest, and that the service worker
 * stays a register-only stub (no caching / no interception) — guarding the
 * ADR-0011 caching anti-pattern.
 */
'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const WEB = __dirname;
const read = (p) => fs.readFileSync(path.join(WEB, p), 'utf8');
const exists = (p) => fs.existsSync(path.join(WEB, p));

/** True if the file starts with the PNG magic bytes (0x89 'P' 'N' 'G'). */
function isPng(p) {
    const b = fs.readFileSync(path.join(WEB, p));
    return (
        b.length >= 8 &&
        b[0] === 0x89 &&
        b[1] === 0x50 /* P */ &&
        b[2] === 0x4e /* N */ &&
        b[3] === 0x47 /* G */
    );
}

/**
 * The PNG IHDR width × height as [w, h] (big-endian: 4 length bytes + 'IHDR'
 * + width at offset 16, height at offset 20). Zero dependencies — the chunk
 * layout is fixed by the PNG spec.
 */
function pngDimensions(p) {
    const b = fs.readFileSync(path.join(WEB, p));
    assert.ok(b.length >= 24, `${p} is long enough to hold an IHDR chunk`);
    assert.equal(b.toString('latin1', 12, 16), 'IHDR', `${p} has an IHDR chunk`);
    return [b.readUInt32BE(16), b.readUInt32BE(20)];
}

// ── manifest.webmanifest ─────────────────────────────────────────────

test('manifest exists and is valid JSON with the required field set', () => {
    const manifest = JSON.parse(read('manifest.webmanifest'));

    assert.ok(manifest.name, 'name is present');
    assert.ok(manifest.short_name, 'short_name is present');
    assert.equal(manifest.start_url, '/',
        'start_url must be "/" (no hash → the ADR-0017 resume route)');
    assert.equal(manifest.display, 'standalone', 'display must be standalone');
    assert.equal(manifest.theme_color, '#0a0a0f', 'theme_color matches the app dark theme');
    assert.equal(manifest.background_color, '#0a0a0f', 'background_color matches the app dark theme');

    const sizes = manifest.icons.map((i) => i.sizes);
    assert.ok(sizes.includes('192x192'), 'icons include 192x192');
    assert.ok(sizes.includes('512x512'), 'icons include 512x512');
    for (const icon of manifest.icons) {
        assert.equal(icon.type, 'image/png', `icon ${icon.src} declares image/png`);
    }
});

test('every manifest icon exists, is a real PNG, and matches its declared size', () => {
    const manifest = JSON.parse(read('manifest.webmanifest'));
    for (const icon of manifest.icons) {
        assert.ok(exists(icon.src), `icon file exists: ${icon.src}`);
        assert.ok(isPng(icon.src), `icon is a valid PNG: ${icon.src}`);
        // Chrome validates declared sizes against the real dimensions, so the
        // manifest must not lie about them.
        const [w, h] = pngDimensions(icon.src);
        assert.equal(
            `${w}x${h}`, icon.sizes,
            `icon pixel size matches the declared sizes: ${icon.src}`
        );
    }
});

// ── index.html wiring ─────────────────────────────────────────────────

test('index.html links the manifest and a file-based apple-touch-icon', () => {
    const html = read('index.html');

    assert.match(html, /<link[^>]*rel="manifest"[^>]*href="manifest\.webmanifest"/,
        'page links manifest.webmanifest');

    const touch = html.match(/<link[^>]*rel="apple-touch-icon"[^>]*href="([^"]+)"/);
    assert.ok(touch, 'apple-touch-icon link is present');
    assert.notEqual(touch[1].slice(0, 5), 'data:', 'apple-touch-icon is a file, not an inline data URL');
    assert.ok(exists(touch[1]), `apple-touch-icon file exists: ${touch[1]}`);
    assert.ok(isPng(touch[1]), `apple-touch-icon is a valid PNG: ${touch[1]}`);
    assert.deepEqual(
        pngDimensions(touch[1]), [180, 180],
        `apple-touch-icon is 180x180: ${touch[1]}`
    );
});

// ── service worker stub ──────────────────────────────────────────────

test('service worker is a register-only stub: no caching, no interception', () => {
    const sw = read('sw.js');

    // The stub must stay inert: no Cache API usage, no interception. If
    // caching is ever added here it silently reintroduces the stale-asset
    // class ADR-0011 exists to prevent (ETag revalidation / live editing).
    assert.ok(!sw.includes('caches.'), 'no Cache API method calls');
    assert.ok(!sw.includes('addAll('), 'no cache.addAll()');
    assert.ok(!sw.includes('respondWith('), 'no fetch interception');
    assert.ok(!sw.includes('skipWaiting('), 'no update strategy');

    // Minimal registered-handler surface: install + fetch, both inert, so the
    // worker satisfies Chromium installability criteria for a future
    // trusted-CA origin without ever touching the network path.
    assert.match(sw, /addEventListener\(["']install["']/, 'install handler present');
    assert.match(sw, /addEventListener\(["']fetch["']/, 'fetch handler present');
});