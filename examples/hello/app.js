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

    const btnCopy = document.getElementById('btn-copy');
    const btnPaste = document.getElementById('btn-paste');
    const btnNotify = document.getElementById('btn-notify');
    const clipboardInput = document.getElementById('clipboard-input');

    // WP3: Invoke clipboard.write
    if (btnCopy) {
        btnCopy.addEventListener('click', async () => {
            output.className = 'output-box running';
            const textToCopy = clipboardInput ? clipboardInput.value : "Hello from Wavical!";
            output.textContent = 'Calling clipboard.write via window.wavical.invoke()...';
            try {
                const res = await window.wavical.invoke('clipboard.write', { text: textToCopy });
                output.className = 'output-box success';
                output.innerHTML = `
                    <div class="output-title">✔ clipboard.write succeeded:</div>
                    <div class="note">Copied text (${textToCopy.length} bytes) to system clipboard.</div>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ clipboard.write failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // WP3: Invoke clipboard.read
    if (btnPaste) {
        btnPaste.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Calling clipboard.read via window.wavical.invoke()...';
            try {
                const res = await window.wavical.invoke('clipboard.read');
                output.className = 'output-box success';
                if (clipboardInput && res && typeof res.text === 'string') {
                    clipboardInput.value = res.text;
                }
                output.innerHTML = `
                    <div class="output-title">✔ clipboard.read succeeded:</div>
                    <pre class="json-display">${JSON.stringify(res, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ clipboard.read failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // WP3: Invoke notify.send
    if (btnNotify) {
        btnNotify.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Calling notify.send via window.wavical.invoke()...';
            try {
                const res = await window.wavical.invoke('notify.send', {
                    title: 'Wavical Notification',
                    body: 'Hello from native desktop notification!'
                });
                output.className = 'output-box success';
                output.innerHTML = `
                    <div class="output-title">✔ notify.send succeeded:</div>
                    <pre class="json-display">${JSON.stringify(res, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ notify.send failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // =========================================================================
    // WP4: Scoped File Access (File Dialogs and Grants)
    // =========================================================================
    const btnOpenFile = document.getElementById('btn-open-file');
    const btnSaveFile = document.getElementById('btn-save-file');
    const btnCancelDialog = document.getElementById('btn-cancel-dialog');
    const btnAttackPath = document.getElementById('btn-attack-path');
    const fileContentInput = document.getElementById('file-content-input');

    // WP4: Open file via native dialog and read contents using opaque grant
    if (btnOpenFile) {
        btnOpenFile.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Opening native file dialog via dialog.openFile...';
            try {
                // Request native open file dialog with fixed filter ID "text"
                const dialogRes = await window.wavical.invoke('dialog.openFile', {
                    filterId: 'text',
                    title: 'Select a text file to read'
                });

                if (dialogRes.cancelled) {
                    output.className = 'output-box';
                    output.innerHTML = `
                        <div class="output-title">Dialog Cancelled:</div>
                        <div class="note">User closed or cancelled the open file dialog. Received clean "cancelled: true" result (not an error).</div>
                    `;
                    return;
                }

                const grantId = dialogRes.grantId;
                output.textContent = `Granted: ${grantId}. Reading file via fs.readGranted (path is never seen by page)...`;

                // Read file via opaque grant ID (never a path)
                const fileRes = await window.wavical.invoke('fs.readGranted', { grantId });
                output.className = 'output-box success';
                if (fileContentInput) {
                    fileContentInput.value = fileRes.content;
                }
                output.innerHTML = `
                    <div class="output-title">✔ File Read Succeeded (via Scoped Grant):</div>
                    <div class="note">Grant ID (128-bit opaque): <code>${grantId}</code></div>
                    <div class="note">File path was kept securely in Rust grant table and never exposed to JavaScript.</div>
                    <pre class="json-display">${escapeHtml(fileRes.content)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ File operation failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // WP4: Save file via native dialog and write contents using opaque grant
    if (btnSaveFile) {
        btnSaveFile.addEventListener('click', async () => {
            output.className = 'output-box running';
            const contentToWrite = fileContentInput ? fileContentInput.value : "Default Wavical saved content.";
            output.textContent = 'Opening native save dialog via dialog.saveFile...';
            try {
                // Request native save file dialog with fixed filter ID "text"
                const dialogRes = await window.wavical.invoke('dialog.saveFile', {
                    filterId: 'text',
                    title: 'Save text file',
                    defaultName: 'wavical_notes.txt'
                });

                if (dialogRes.cancelled) {
                    output.className = 'output-box';
                    output.innerHTML = `
                        <div class="output-title">Save Cancelled:</div>
                        <div class="note">User cancelled save file dialog. Received clean "cancelled: true" result.</div>
                    `;
                    return;
                }

                const grantId = dialogRes.grantId;
                output.textContent = `Granted: ${grantId}. Writing payload (${contentToWrite.length} bytes) via fs.writeGranted...`;

                const writeRes = await window.wavical.invoke('fs.writeGranted', {
                    grantId,
                    content: contentToWrite
                });

                output.className = 'output-box success';
                output.innerHTML = `
                    <div class="output-title">✔ File Write Succeeded (via Scoped Grant):</div>
                    <div class="note">Grant ID (128-bit opaque): <code>${grantId}</code></div>
                    <div class="note">Bytes written: ${writeRes.bytesWritten}</div>
                    <pre class="json-display">${JSON.stringify(writeRes, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ File save failed:</div><div>${err.message}</div>`;
            }
        });
    }

    // WP4: Test Dialog Cancellation
    if (btnCancelDialog) {
        btnCancelDialog.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Please click "Cancel" in the native dialog window...';
            try {
                const res = await window.wavical.invoke('dialog.openFile', {
                    title: 'Test: Please Click Cancel'
                });
                output.className = 'output-box success';
                output.innerHTML = `
                    <div class="output-title">✔ Dialog Cancellation Handled Cleanly:</div>
                    <div class="note">Received generic cancelled response: <code>${JSON.stringify(res)}</code></div>
                    <div class="note">No error was thrown, fulfilling Section 0.3 specifications.</div>
                `;
            } catch (err) {
                output.className = 'output-box error';
                output.innerHTML = `<div class="output-title">✖ Unexpected error on cancel:</div><div>${err.message}</div>`;
            }
        });
    }

    // WP4: Malicious Path Injection Attack (Page sends raw path string)
    if (btnAttackPath) {
        btnAttackPath.addEventListener('click', async () => {
            output.className = 'output-box running';
            output.textContent = 'Simulating attacker attempting to pass raw file path to fs.readGranted...';
            try {
                // Attacker attempts to inject raw path
                const res = await window.wavical.invoke('fs.readGranted', {
                    grantId: '0123456789abcdef0123456789abcdef',
                    path: 'C:\\Windows\\System32\\drivers\\etc\\hosts'
                });
                output.className = 'output-box error';
                output.innerHTML = `
                    <div class="output-title">✖ Critical Failure: Raw path was accepted!</div>
                    <pre class="json-display">${JSON.stringify(res, null, 2)}</pre>
                `;
            } catch (err) {
                output.className = 'output-box expected-rejection';
                output.innerHTML = `
                    <div class="output-title">🛡️ Raw Path Injection Rejected as Expected:</div>
                    <div class="error-msg">${escapeHtml(err.message)}</div>
                    <div class="note">Rust serde(deny_unknown_fields) and grant validation blocked the path injection attempt.</div>
                `;
            }
        });
    }

    function escapeHtml(str) {
        if (!str) return '';
        return String(str)
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;')
            .replace(/'/g, '&#039;');
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
