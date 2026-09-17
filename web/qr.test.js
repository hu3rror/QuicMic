/**
 * Unit tests for web/qr.js — the pure in-app QR scan logic (ADR-0019).
 *
 * Run with: node --test web/qr.test.js   (Node built-in runner, zero deps)
 *
 * Tests exercise the module's public functions only (text → PIN, frame → text,
 * error → message); no DOM, getUserMedia, or timers are touched. The decode
 * path runs the vendored jsQR (web/jsqr.js) against a committed PPM fixture of
 * a real Pairing QR, so the vendor file is exercised end-to-end in Node.
 */
'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const qr = require('./qr.js');

// ── parseScannedText ──────────────────────────────────────────────────

test('extracts the 6-digit PIN from the Pairing QR text', () => {
    const cases = [
        ['https://192.168.1.5:8080#123456', '123456'],
        ['https://10.0.0.7:8443#000000', '000000'],
        ['#123456', '123456'],
        ['  https://host:8080#123456\n', '123456'],
    ];
    for (const [text, expected] of cases) {
        assert.equal(qr.parseScannedText(text), expected, `for input ${JSON.stringify(text)}`);
    }
});

test('rejects anything that is not a trailing 6-digit hash', () => {
    const cases = [null, undefined, '', 42, {}, 'not a qr', '123456',
        'https://host:8080#12345', 'https://host:8080#1234567',
        'https://host:8080#abc456', 'https://host:8080#12345 6',
        'https://host:8080#123456#', 'https://host:8080'];
    for (const text of cases) {
        assert.equal(qr.parseScannedText(text), null, `for input ${JSON.stringify(text)}`);
    }
});

// ── decodeQr (vendored jsQR) ──────────────────────────────────────────

/**
 * Parse a P6 (raw RGB) PPM into { width, height, rgba }. The fixture is
 * generated with a plain header + raw bytes (zero dependencies to read); a QR
 * package encoded it once, off-repo, and the artifact is committed.
 */
function readPpm(rel) {
    const buf = fs.readFileSync(path.join(__dirname, rel));
    let i = 0;
    const token = () => {
        while (i < buf.length && buf[i] <= 0x20) i++;
        const s = i;
        while (i < buf.length && buf[i] > 0x20) i++;
        return buf.toString('latin1', s, i);
    };
    assert.equal(token(), 'P6', 'fixture magic');
    const width = Number(token());
    const height = Number(token());
    assert.equal(Number(token()), 255, 'fixture maxval');
    i++; // the single whitespace byte between the header and the pixel payload
    const rgb = buf.subarray(i);
    assert.equal(rgb.length, width * height * 3, 'pixel payload size');
    const rgba = new Uint8ClampedArray(width * height * 4);
    for (let p = 0; p < width * height; p++) {
        rgba[p * 4] = rgb[p * 3];
        rgba[p * 4 + 1] = rgb[p * 3 + 1];
        rgba[p * 4 + 2] = rgb[p * 3 + 2];
        rgba[p * 4 + 3] = 255;
    }
    return { width, height, rgba };
}

test('the vendored jsQR decodes the committed Pairing QR fixture', () => {
    const { width, height, rgba } = readPpm('testdata/qr-fixture.ppm');
    const text = qr.decodeQr(rgba, width, height);
    assert.equal(text, 'https://192.168.1.5:8080#123456');
    assert.equal(qr.parseScannedText(text), '123456');
});

test('decodeQr returns null when the frame holds no QR', () => {
    const { width, height } = readPpm('testdata/qr-fixture.ppm');
    const blank = new Uint8ClampedArray(width * height * 4).fill(255);
    assert.equal(qr.decodeQr(blank, width, height), null);
});

// ── captureErrorMessage ────────────────────────────────────────────────

test('camera denial maps to a manual-entry hint', () => {
    assert.match(qr.captureErrorMessage({ name: 'NotAllowedError' }), /enter the PIN manually/);
});

test('other camera failures map to retry hints that still name manual entry', () => {
    const cases = ['NotFoundError', 'NotReadableError', 'OverconstrainedError', 'TrackStartError', 'AbortError', 'BogusError'];
    for (const name of cases) {
        const msg = qr.captureErrorMessage({ name });
        assert.match(msg, /Try again/, `retry cue for ${name}`);
        assert.match(msg, /enter the PIN manually/, `manual escape for ${name}`);
    }
});

test('a non-error input falls back to the generic retry message', () => {
    for (const value of [null, undefined, {}]) {
        assert.match(qr.captureErrorMessage(value), /Try again/);
    }
});