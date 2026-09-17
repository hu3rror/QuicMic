/**
 * QuicMic Client — pure in-app QR scan logic (ADR-0019).
 *
 * No DOM, no getUserMedia, no timers: everything here is a pure function over
 * plain inputs, so QR-text parsing, the vendored-decoder wrapper, and the
 * camera-error mapping can be unit-tested with Node's built-in test runner
 * (`node --test web/qr.test.js`) without a browser. app.js is the only caller
 * and keeps all the glue (getUserMedia, viewfinder loop, pairing).
 *
 * Loads both as a classic browser script (`window.QuicMicQr`, loaded after
 * jsqr.js and before app.js) and as a Node module (`module.exports`) for tests.
 */
(function (root, factory) {
    if (typeof module === 'object' && module.exports) {
        module.exports = factory(require('./jsqr.js'));
    } else {
        root.QuicMicQr = factory(root.jsQR);
    }
}(typeof self !== 'undefined' ? self : this, function (jsQR) {
    'use strict';

    // The Pairing QR encodes `https://<host>:<port>#<6-digit PIN>` (CONTEXT.md).
    // The scanner only needs the PIN, and the host in the QR is informational —
    // pair always targets the current page's origin (the PIN is machine-pinned
    // and does not rotate, ADR-0015). Any text that is not a trailing 6-digit
    // hash is a scan failure, never an auth event (ADR-0019).
    const PIN_RE = /#(\d{6})$/;

    /**
     * Extract the 6-digit pairing PIN from decoded QR text, or null when the
     * text is not the Pairing QR shape (no trailing `#<6 digits>`).
     */
    function parseScannedText(text) {
        if (typeof text !== 'string') return null;
        const m = PIN_RE.exec(text.trim());
        return m ? m[1] : null;
    }

    /**
     * Attempt to decode one RGBA frame with the vendored jsQR. Returns the
     * decoded text of any findable QR (not only the Pairing QR) or null when
     * the frame holds none — the caller decides whether the text is one of ours.
     */
    function decodeQr(rgba, width, height) {
        const result = jsQR(rgba, width, height);
        return result ? result.data : null;
    }

    // Two-tier mapping (ADR-0019): a hard denial degrades to manual entry; the
    // transient failures (camera in use, no camera, overconstrained) offer a
    // retry cue first. Both always name the manual escape hatch, so pairing
    // never dead-ends behind the camera.
    const CAPTURE_ERROR_MESSAGES = {
        NotAllowedError: 'Camera access denied — enter the PIN manually below.',
        NotFoundError: 'No camera found on this device. Try again, or enter the PIN manually below.',
        NotReadableError: 'The camera is in use by another app. Try again, or enter the PIN manually below.',
        TrackStartError: 'The camera is in use by another app. Try again, or enter the PIN manually below.',
        OverconstrainedError: 'The camera could not start. Try again, or enter the PIN manually below.',
        AbortError: 'The camera could not start. Try again, or enter the PIN manually below.',
    };

    /** Map a getUserMedia error to the message shown on the pairing card. */
    function captureErrorMessage(error) {
        const name = error && error.name;
        return CAPTURE_ERROR_MESSAGES[name] || 'Could not start the camera. Try again, or enter the PIN manually below.';
    }

    return {
        parseScannedText,
        decodeQr,
        captureErrorMessage,
    };
}));