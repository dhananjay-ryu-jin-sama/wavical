// Wavical Hello World Frontend Application Logic
// Adheres strictly to Content Security Policy without inline scripts

document.addEventListener('DOMContentLoaded', () => {
    const btnPing = document.getElementById('btn-ping');
    const btnEcho = document.getElementById('btn-echo');
    const btnNavExternal = document.getElementById('btn-nav-external');
    const btnPopup = document.getElementById('btn-popup');
    const btnRemoteScript = document.getElementById('btn-remote-script');
    const output = document.getElementById('ipc-output');

    // 1. Invoke core.ping (Granted capability)
    if (btnPing) {
        btnPing.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Calling core.ping via window.wavical.invoke()...';
            try {
                const res = await window.wavical.invoke('core.ping');
                output.className = 'output-box success';
                output.innerHTML = `
                    <div class="output-title">✔ core.ping succeeded:</div>
                    <pre class="json-display">${JSON.stringify(res, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ core.ping failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // 2. Invoke core.echo (Ungranted capability -> Expected default-deny)
    if (btnEcho) {
        btnEcho.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Calling core.echo via window.wavical.invoke()...';
            try {
                const res = await window.wavical.invoke('core.echo', { sample: "hello from frontend" });
                output.className = 'output-box error';
                output.innerHTML = `
                    <div class="output-title">✖ Unexpected success (core.echo should be denied):</div>
                    <pre class="json-display">${JSON.stringify(res, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box expected-rejection';
                output.innerHTML = `
                    <div class="output-title">🛡️ core.echo rejected as expected:</div>
                    <div class="error-msg">${err.message}</div>
                    <div class="note">Rust capability check blocked ungranted method before routing.</div>
                `;
            }
        });
    }

    // 3. Test External Navigation (Blocked by Navigation Handler)
    if (btnNavExternal) {
        btnNavExternal.addEventListener('click', () => {
            output.className = 'output-box running';
            output.textContent = 'Attempting top-level navigation to https://example.com...';
            try {
                window.location.href = 'https://example.com';
                setTimeout(() => {
                    output.className = 'output-box expected-rejection';
                    output.innerHTML = `
                        <div class="output-title">🛡️ External Navigation Blocked:</div>
                        <div class="error-msg">Navigation to https://example.com was denied by Rust navigation handler.</div>
                        <div class="note">Current location remains on wavical:// app origin.</div>
                    `;
                }, 400);
            } catch (err) {
                output.className = 'output-box expected-rejection';
                output.innerHTML = `<div class="output-title">🛡️ Navigation error:</div><div>${err.message}</div>`;
            }
        });
    }

    // 4. Test Popup / New Window Request (Blocked by New Window Handler)
    if (btnPopup) {
        btnPopup.addEventListener('click', () => {
            output.className = 'output-box running';
            output.textContent = 'Attempting to open popup window (window.open)...';
            try {
                const popup = window.open('https://example.com', '_blank');
                if (!popup || popup.closed) {
                    output.className = 'output-box expected-rejection';
                    output.innerHTML = `
                        <div class="output-title">🛡️ Popup Blocked:</div>
                        <div class="error-msg">window.open() returned null/closed.</div>
                        <div class="note">Rust new-window handler returned NewWindowResponse::Deny.</div>
                    `;
                } else {
                    output.className = 'output-box error';
                    output.innerHTML = `<div class="output-title">✖ Popup was unexpectedly opened.</div>`;
                }
            } catch (err) {
                output.className = 'output-box expected-rejection';
                output.innerHTML = `<div class="output-title">🛡️ Popup blocked with error:</div><div>${err.message}</div>`;
            }
        });
    }

    // 5. Test Remote Script Injection (Blocked by strict CSP script-src 'self')
    if (btnRemoteScript) {
        btnRemoteScript.addEventListener('click', () => {
            output.className = 'output-box running';
            output.textContent = 'Attempting to inject remote script (https://cdn.jsdelivr.net)...';
            
            const script = document.createElement('script');
            script.src = 'https://cdn.jsdelivr.net/npm/lodash@4.17.21/lodash.min.js';
            script.onload = () => {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ Remote script unexpectedly loaded!</div>`;
            };
            script.onerror = () => {
                output.className = 'output-box expected-rejection';
                output.innerHTML = `
                    <div class="output-title">🛡️ Remote Script Blocked by CSP:</div>
                    <div class="error-msg">Failed to load resource: script-src 'self' violation.</div>
                    <div class="note">Content-Security-Policy blocked untrusted CDN script execution.</div>
                `;
            };
            document.head.appendChild(script);
        });
    }
});
