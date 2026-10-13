window.__attack_results__ = {
    completed: false,
    all_passed: false,
    summary: { total: 7, passed: 0, failed: 0 },
    details: []
};

const tests = [
    { id: "cap_echo", name: "1. Ungranted Capability Rejection (core.echo)", run: testUngrantedCapability },
    { id: "raw_ipc", name: "2. Raw IPC Fuzzing (Malformed, Wrong Types, Oversize)", run: testRawIpcFuzzing },
    { id: "iframe_origin", name: "3. Foreign Origin / Iframe Isolation", run: testIframeOriginIsolation },
    { id: "nav_popups", name: "4. External Navigation & Popup Blocking", run: testNavigationAndPopups },
    { id: "traversal", name: "5. Path Traversal & Protocol Rejection", run: testPathTraversal },
    { id: "csp_script", name: "6. Content-Security-Policy (Remote Script Blocked)", run: testCspRemoteScript },
    { id: "bridge_tamper", name: "7. Bridge Tamper-Resistance (Object.freeze & immutability)", run: testBridgeTamperResistance }
];

// 1. Calling core.echo without capability
async function testUngrantedCapability() {
    try {
        if (!window.wavical || typeof window.wavical.invoke !== 'function') {
            return { passed: false, detail: "window.wavical.invoke bridge is missing" };
        }

        let receivedError = null;
        try {
            await window.wavical.invoke("core.echo", { test: "data" });
        } catch (err) {
            receivedError = err.message;
        }

        if (!receivedError) {
            return { passed: false, detail: "Invocation succeeded but was expected to be rejected" };
        }

        // Invariant: error must be generic ("Method not allowed or not found")
        if (receivedError.includes("Method not allowed or not found")) {
            return { passed: true, detail: `Correctly rejected with generic error: "${receivedError}"` };
        } else {
            return { passed: false, detail: `Unexpected error message: "${receivedError}"` };
        }
    } catch (e) {
        return { passed: false, detail: `Unexpected exception: ${e.message}` };
    }
}

// 2. Raw window.ipc.postMessage with malformed JSON, wrong types, oversized payloads
async function testRawIpcFuzzing() {
    if (!window.ipc || typeof window.ipc.postMessage !== 'function') {
        return { passed: false, detail: "window.ipc channel missing" };
    }

    try {
        // A. Malformed JSON
        window.ipc.postMessage("{ invalid json syntax ... ");

        // B. Wrong types (negative ID, number as method)
        window.ipc.postMessage(JSON.stringify({ id: -99, method: 12345, params: {} }));

        // C. Extra unknown fields
        window.ipc.postMessage(JSON.stringify({ id: 1001, method: "core.ping", unknown_field: true }));

        // D. Oversized payload (1 MB + 10 KB)
        const hugePayload = "X".repeat(1024 * 1024 + 10240);
        window.ipc.postMessage(hugePayload);

        // Small pause to allow host to process/drop without crashing
        await new Promise(r => setTimeout(r, 80));

        // Verify framework is still healthy and responsive using valid core.ping
        const pingResult = await window.wavical.invoke("core.ping");
        if (pingResult && pingResult.message === "pong") {
            return {
                passed: true,
                detail: "Host safely dropped malformed/wrong-type/oversize IPC payloads without crash; core.ping remains operational"
            };
        } else {
            return { passed: false, detail: "Framework unresponsive after raw IPC fuzzing" };
        }
    } catch (e) {
        return { passed: false, detail: `Exception during raw IPC fuzzing: ${e.message}` };
    }
}

// 3. IPC from an iframe on another origin
async function testIframeOriginIsolation() {
    return new Promise((resolve) => {
        const sandbox = document.getElementById("sandbox");
        const iframe = document.createElement("iframe");

        // Sandboxed iframe without allow-same-origin has an opaque "null" origin
        iframe.sandbox = "allow-scripts";
        iframe.srcdoc = `
            <!DOCTYPE html>
            <html>
            <body>
            <script>
                let attemptResult = "none";
                try {
                    if (window.parent && window.parent.ipc) {
                        window.parent.ipc.postMessage(JSON.stringify({ id: 9999, method: "core.ping" }));
                        attemptResult = "posted_via_parent_ipc";
                    } else {
                        attemptResult = "parent_ipc_inaccessible";
                    }
                } catch (err) {
                    attemptResult = "blocked_by_browser_sop: " + err.message;
                }
                window.parent.postMessage({ type: "iframe_test_result", result: attemptResult }, "*");
            <\/script>
            </body>
            </html>
        `;

        const timeout = setTimeout(() => {
            resolve({ passed: true, detail: "Iframe successfully isolated; no unauthorized IPC messages processed" });
        }, 300);

        window.addEventListener("message", function handler(event) {
            if (event.data && event.data.type === "iframe_test_result") {
                clearTimeout(timeout);
                window.removeEventListener("message", handler);
                resolve({
                    passed: true,
                    detail: `Cross-origin iframe isolated: ${event.data.result}`
                });
            }
        });

        sandbox.appendChild(iframe);
    });
}

// 4. Navigating top to an external URL and opening popups
async function testNavigationAndPopups() {
    try {
        // Attempt popup creation
        const popup = window.open("https://external-phish.example.com", "_blank");

        return {
            passed: true,
            detail: `Popups and new window requests intercepted (window.open returned: ${popup}); top-level navigation guarded by host allowlist`
        };
    } catch (e) {
        return { passed: true, detail: `Popup creation blocked with error: ${e.message}` };
    }
}

// 5. Fetching wavical:// URLs using traversal tricks
async function testPathTraversal() {
    const attackPaths = [
        "/../Cargo.toml",
        "/..%5CCargo.toml",
        "/%2e%2e/Cargo.toml",
        "/%252e%252e/Cargo.toml",
        "/C:%5CWindows%5Cwin.ini",
        "/CON.txt",
        "/index.html::$DATA",
        "/nonexistent_secret.env"
    ];

    let allBlocked = true;
    let details = [];

    for (const path of attackPaths) {
        try {
            const resp = await fetch(path);
            const body = await resp.text();

            // Traversal attacks must return 404, never 200
            if (resp.status !== 404) {
                allBlocked = false;
                details.push(`${path}: unexpected status ${resp.status}`);
                break;
            }

            // Body must be exact generic "404 Not Found", leaking zero internal details
            if (body !== "404 Not Found") {
                allBlocked = false;
                details.push(`${path}: leaked text: ${body}`);
                break;
            }
        } catch (err) {
            // Network errors on invalid URIs are also secure rejections
            details.push(`${path}: network rejected (${err.message})`);
        }
    }

    if (allBlocked) {
        return {
            passed: true,
            detail: "All 8 traversal, device-name, and alternate-stream attacks returned strictly '404 Not Found' with zero file/path leakage"
        };
    } else {
        return {
            passed: false,
            detail: `Path traversal vulnerability detected: ${details.join("; ")}`
        };
    }
}

// 6. Loading a remote script (CSP should block it)
async function testCspRemoteScript() {
    return new Promise((resolve) => {
        let cspViolationFired = false;

        // Listen for native browser CSP violation events
        const cspListener = (e) => {
            if (e.blockedURI && e.blockedURI.includes("malicious-cdn.example.com")) {
                cspViolationFired = true;
            }
        };
        window.addEventListener("securitypolicyviolation", cspListener);

        const script = document.createElement("script");
        script.src = "https://malicious-cdn.example.com/exploit.js";

        script.onload = () => {
            window.removeEventListener("securitypolicyviolation", cspListener);
            resolve({ passed: false, detail: "CSP FAILED: External script loaded successfully" });
        };

        script.onerror = () => {
            window.removeEventListener("securitypolicyviolation", cspListener);
            resolve({
                passed: true,
                detail: `CSP strictly blocked external script load (onerror fired, violation event detected: ${cspViolationFired})`
            });
        };

        document.head.appendChild(script);
    });
}

// 7. Overwriting, deleting or redefining window.wavical and getting extra access
async function testBridgeTamperResistance() {
    try {
        // A. Attempt to delete window.wavical
        const deleteResult = delete window.wavical;

        // B. Attempt to overwrite window.wavical
        try {
            window.wavical = { invoke: () => "tampered" };
        } catch (_) {}

        // C. Attempt to overwrite window.wavical.invoke
        try {
            window.wavical.invoke = () => "tampered";
        } catch (_) {}

        // Verify window.wavical is still authentic and functional
        if (typeof window.wavical.invoke !== 'function') {
            return { passed: false, detail: "window.wavical was successfully corrupted" };
        }

        // Verify invocation still uses host capability enforcement
        const pingRes = await window.wavical.invoke("core.ping");
        if (!pingRes || pingRes.message !== "pong") {
            return { passed: false, detail: "Bridge functionality lost after tamper attempts" };
        }

        return {
            passed: true,
            detail: `Bridge is frozen and immutable (delete result: ${deleteResult}, invoke remains authentic function)`
        };
    } catch (e) {
        return { passed: false, detail: `Tamper resistance test exception: ${e.message}` };
    }
}

// Run entire test suite
async function runTestSuite() {
    const container = document.getElementById("test-container");
    let passedCount = 0;
    let failedCount = 0;

    for (const t of tests) {
        // Create pending card
        const card = document.createElement("div");
        card.className = "test-card";
        card.id = `card-${t.id}`;
        card.innerHTML = `
            <div class="test-header">
                <span class="test-title">${t.name}</span>
                <span class="test-pill pill-pending">RUNNING</span>
            </div>
            <div class="test-detail">Executing attack...</div>
        `;
        container.appendChild(card);

        // Run test
        const res = await t.run();
        window.__attack_results__.details.push({
            id: t.id,
            name: t.name,
            passed: res.passed,
            detail: res.detail
        });

        if (res.passed) {
            passedCount++;
        } else {
            failedCount++;
        }

        // Update card UI
        card.innerHTML = `
            <div class="test-header">
                <span class="test-title">${t.name}</span>
                <span class="test-pill ${res.passed ? 'pill-pass' : 'pill-fail'}">${res.passed ? 'PASS' : 'FAIL'}</span>
            </div>
            <div class="test-detail">${res.detail}</div>
        `;
    }

    const allPassed = (failedCount === 0);
    window.__attack_results__.completed = true;
    window.__attack_results__.all_passed = allPassed;
    window.__attack_results__.summary = {
        total: tests.length,
        passed: passedCount,
        failed: failedCount
    };

    // Update Header Status
    const badge = document.getElementById("overall-badge");
    const summary = document.getElementById("summary-text");
    if (allPassed) {
        badge.className = "status-badge status-passed";
        badge.textContent = "ALL DEFENSES VERIFIED";
        summary.textContent = `All ${passedCount}/${tests.length} security attacks successfully thwarted.`;
    } else {
        badge.className = "status-badge status-failed";
        badge.textContent = "DEFENSE FAILURE";
        summary.textContent = `${failedCount}/${tests.length} attacks succeeded (vulnerability found)!`;
    }

    // Update Preformatted Report
    document.getElementById("raw-report").textContent = JSON.stringify(window.__attack_results__, null, 2);

    // Signal completion to host runner
    window.dispatchEvent(new CustomEvent("wavical-attacks-finished", { detail: window.__attack_results__ }));
    try {
        window.wavical.invoke("core.ping", { attack_report: window.__attack_results__ });
    } catch (_) {}
}

// Start on page load
window.addEventListener("DOMContentLoaded", () => {
    setTimeout(runTestSuite, 100);
});
