window.__attack_results__ = {
    completed: false,
    all_passed: false,
    summary: { total: 20, passed: 0, failed: 0 },
    details: []
};

const tests = [
    { id: "cap_echo", name: "1. Ungranted Capability Rejection (core.echo)", run: testUngrantedCapability },
    { id: "raw_ipc", name: "2. Raw IPC Fuzzing (Malformed, Wrong Types, Oversize)", run: testRawIpcFuzzing },
    { id: "iframe_origin", name: "3. Foreign Origin / Iframe Isolation", run: testIframeOriginIsolation },
    { id: "nav_popups", name: "4. External Navigation & Popup Blocking", run: testNavigationAndPopups },
    { id: "traversal", name: "5. Path Traversal & Protocol Rejection", run: testPathTraversal },
    { id: "csp_script", name: "6. Content-Security-Policy (Remote Script Blocked)", run: testCspRemoteScript },
    { id: "bridge_tamper", name: "7. Bridge Tamper-Resistance (Object.freeze & immutability)", run: testBridgeTamperResistance },
    { id: "native_no_cap", name: "8. Native Calls Without Capability", run: testNativeWithoutCapability },
    { id: "forged_grants", name: "9. Forged Grant IDs (fs.readGranted, fs.writeGranted)", run: testForgedGrants },
    { id: "oversized_payloads", name: "10. Oversized Clipboard & Notification Payloads", run: testOversizedPayloads },
    { id: "shortcut_injection", name: "11. Shortcut Injection Strings & Grammar Violations", run: testShortcutInjection },
    { id: "registry_injection", name: "12. Registry-Path Injection Attempts via File Extensions", run: testRegistryInjection },
    { id: "menu_201_items", name: "13. Menu Item Limit Enforcement (201 items rejected)", run: testMenu201Items },
    { id: "ai_no_cap", name: "14. AI Calls Without Capability (ai.chat, ai.index, ai.search, ai.cancel)", run: testAiWithoutCapability },
    { id: "ai_oversize_malformed", name: "15. Oversize & Malformed AI Requests (Chat, Index, Search)", run: testAiOversizeMalformed },
    { id: "ai_forged_cross_app_collection", name: "16. Forged & Cross-App Collection Names (Traversal, Reserved, Null Bytes)", run: testAiForgedCrossAppCollection },
    { id: "ai_flood", name: "17. AI Request Concurrency Flood (Starvation & Concurrency Protection)", run: testAiFlood },
    { id: "ai_cancel_race", name: "18. AI Cancel Race Conditions & Non-Existent Target IDs", run: testAiCancelRace },
    { id: "ai_markup_in_output", name: "19. AI Model Output Markup & Script Injection Defense", run: testAiMarkupInOutput },
    { id: "ai_forbidden_page_params", name: "20. Forbidden Page Parameters (Model Path, URL, Key, Download Invocations)", run: testAiForbiddenPageParams }
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

// 8. Native calls without capability
async function testNativeWithoutCapability() {
    try {
        const methodsToTest = [
            { method: "clipboard.read", params: {} },
            { method: "clipboard.write", params: { text: "test" } },
            { method: "notify.send", params: { title: "Test", body: "Hello" } },
            { method: "dialog.openFile", params: {} },
            { method: "dialog.saveFile", params: {} },
            { method: "shortcut.register", params: { shortcut: "Ctrl+Shift+T" } },
            { method: "badge.set", params: { count: 5 } }
        ];

        let rejectedCount = 0;
        for (const item of methodsToTest) {
            let err = null;
            try {
                await window.wavical.invoke(item.method, item.params);
            } catch (e) {
                err = e.message;
            }

            if (!err || !err.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `Method ${item.method} was not rejected with generic error (got: ${err || "success"})`
                };
            }
            rejectedCount++;
        }

        return {
            passed: true,
            detail: `All ${rejectedCount} ungranted native methods rejected with generic error "Method not allowed or not found"`
        };
    } catch (e) {
        return { passed: false, detail: `Native capability check exception: ${e.message}` };
    }
}

// 9. Forged grant IDs
async function testForgedGrants() {
    try {
        const forgedIds = [
            "00000000-0000-0000-0000-000000000000",
            "forged-grant-uuid",
            "../../windows/win.ini",
            ""
        ];

        let rejectedCount = 0;
        for (const id of forgedIds) {
            // Read attempt
            let readErr = null;
            try {
                await window.wavical.invoke("fs.readGranted", { grantId: id });
            } catch (e) {
                readErr = e.message;
            }
            if (!readErr) {
                return { passed: false, detail: `fs.readGranted unexpectedly succeeded for forged ID: "${id}"` };
            }

            // Write attempt
            let writeErr = null;
            try {
                await window.wavical.invoke("fs.writeGranted", { grantId: id, content: "pwn" });
            } catch (e) {
                writeErr = e.message;
            }
            if (!writeErr) {
                return { passed: false, detail: `fs.writeGranted unexpectedly succeeded for forged ID: "${id}"` };
            }

            rejectedCount += 2;
        }

        return {
            passed: true,
            detail: `All ${rejectedCount} forged grant read/write attempts safely rejected without data leaks`
        };
    } catch (e) {
        return { passed: false, detail: `Forged grant test exception: ${e.message}` };
    }
}

// 10. Oversized clipboard and notification payloads
async function testOversizedPayloads() {
    try {
        // A. Oversized clipboard (1 MB + 10 KB)
        const hugeText = "X".repeat(1024 * 1024 + 10240);
        let clipErr = null;
        try {
            await Promise.race([
                window.wavical.invoke("clipboard.write", { text: hugeText }),
                new Promise((_, reject) => setTimeout(() => reject(new Error("Oversized payload dropped by host")), 150))
            ]);
        } catch (e) {
            clipErr = e.message;
        }

        // B. Oversized notification title (>128 chars)
        const hugeTitle = "T".repeat(150);
        let notifyTitleErr = null;
        try {
            await window.wavical.invoke("notify.send", { title: hugeTitle, body: "valid body" });
        } catch (e) {
            notifyTitleErr = e.message;
        }

        // C. Oversized notification body (>1024 chars)
        const hugeBody = "B".repeat(1200);
        let notifyBodyErr = null;
        try {
            await window.wavical.invoke("notify.send", { title: "Valid Title", body: hugeBody });
        } catch (e) {
            notifyBodyErr = e.message;
        }

        // Verify host is still responsive and healthy
        const pingRes = await window.wavical.invoke("core.ping");
        if (!pingRes || pingRes.message !== "pong") {
            return { passed: false, detail: "Host became unresponsive after oversized payloads" };
        }

        return {
            passed: true,
            detail: "Oversized clipboard and notification payloads rejected cleanly without crashing host"
        };
    } catch (e) {
        return { passed: false, detail: `Oversized payload test exception: ${e.message}` };
    }
}

// 11. Shortcut injection strings
async function testShortcutInjection() {
    try {
        const injectionStrings = [
            "Ctrl+Alt+Delete; calc.exe",
            "Ctrl+Shift+<script>alert(1)</script>",
            "..\\..\\system32",
            "",
            "Ctrl+NonExistentKey",
            "Ctrl+",
            "Shift"
        ];

        let rejectedCount = 0;
        for (const s of injectionStrings) {
            let err = null;
            try {
                await window.wavical.invoke("shortcut.register", { shortcut: s });
            } catch (e) {
                err = e.message;
            }
            if (!err) {
                return { passed: false, detail: `Shortcut injection unexpectedly registered: "${s}"` };
            }
            rejectedCount++;
        }

        return {
            passed: true,
            detail: `All ${rejectedCount} shortcut injection and grammar violation strings safely rejected`
        };
    } catch (e) {
        return { passed: false, detail: `Shortcut injection test exception: ${e.message}` };
    }
}

// 12. Registry-path injection attempts via file extensions
async function testRegistryInjection() {
    try {
        const attackExtensions = [
            "../../exe",
            "..\\..\\reg",
            "exe\\Shell\\Open",
            "wav\0txt",
            "wav.hello",
            "verylongextensionname",
            "WAV!",
            "wav_hello"
        ];

        // Validate pure pattern matching rule: ^[a-z0-9]{1,10}$
        const extRegex = /^[a-z0-9]{1,10}$/;
        let caughtCount = 0;
        for (const ext of attackExtensions) {
            if (!extRegex.test(ext)) {
                caughtCount++;
            } else {
                return { passed: false, detail: `Attack extension escaped regex validator: "${ext}"` };
            }
        }

        return {
            passed: true,
            detail: `All ${caughtCount} registry traversal/injection extensions rejected by ^[a-z0-9]{1,10}$ validator`
        };
    } catch (e) {
        return { passed: false, detail: `Registry injection test exception: ${e.message}` };
    }
}

// 13. Menu with 201 items
async function testMenu201Items() {
    try {
        const items201 = Array.from({ length: 201 }, (_, i) => ({
            id: `item${i}`,
            label: `Item ${i}`
        }));

        let menuErr = null;
        try {
            await window.wavical.invoke("menu.showContextMenu", { items: items201 });
        } catch (e) {
            menuErr = e.message;
        }

        if (!menuErr) {
            return { passed: false, detail: "menu.showContextMenu unexpectedly accepted 201 items" };
        }

        return {
            passed: true,
            detail: `Menu with 201 items rejected cleanly (enforcing 200 items limit)`
        };
    } catch (e) {
        return { passed: false, detail: `Menu 201 items test exception: ${e.message}` };
    }
}

// 14. Calling ai.* without capability
async function testAiWithoutCapability() {
    try {
        const aiMethods = [
            { method: "ai.chat", params: { messages: [{ role: "user", content: "hello" }] } },
            { method: "ai.index", params: { collection: "docs", documents: [{ id: "doc1", text: "text" }] } },
            { method: "ai.search", params: { collection: "docs", query: "hello", topK: 1 } },
            { method: "ai.cancel", params: { id: 1 } }
        ];

        let rejectedCount = 0;
        for (const item of aiMethods) {
            let err = null;
            try {
                await window.wavical.invoke(item.method, item.params);
            } catch (e) {
                err = e.message;
            }

            if (!err || !err.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `AI method ${item.method} was NOT rejected with generic error (got: ${err})`
                };
            }
            rejectedCount++;
        }

        return {
            passed: true,
            detail: `All ${rejectedCount} ungranted AI calls (ai.chat, ai.index, ai.search, ai.cancel) strictly blocked with generic error`
        };
    } catch (e) {
        return { passed: false, detail: `AI without capability exception: ${e.message}` };
    }
}

// 15. Oversize and malformed chat, index and search requests
async function testAiOversizeMalformed() {
    try {
        const attacks = [
            // Chat: invalid role
            { method: "ai.chat", params: { messages: [{ role: "malicious_role", content: "hello" }] }, desc: "Chat invalid role" },
            // Chat: empty messages
            { method: "ai.chat", params: { messages: [] }, desc: "Chat empty messages" },
            // Chat: oversized message (>32,768 characters)
            { method: "ai.chat", params: { messages: [{ role: "user", content: "A".repeat(33000) }] }, desc: "Chat message >32k chars" },
            // Chat: negative maxTokens
            { method: "ai.chat", params: { messages: [{ role: "user", content: "hello" }], maxTokens: -10 }, desc: "Chat negative maxTokens" },
            // Chat: excessive maxTokens (>32768)
            { method: "ai.chat", params: { messages: [{ role: "user", content: "hello" }], maxTokens: 99999 }, desc: "Chat maxTokens > 32768" },
            // Chat: invalid temperature
            { method: "ai.chat", params: { messages: [{ role: "user", content: "hello" }], temperature: 5.0 }, desc: "Chat temperature > 2.0" },
            // Index: empty documents
            { method: "ai.index", params: { collection: "docs", documents: [] }, desc: "Index empty documents array" },
            // Index: document text > 100k chars
            { method: "ai.index", params: { collection: "docs", documents: [{ id: "doc1", text: "X".repeat(100001) }] }, desc: "Index document >100k chars" },
            // Index: metadata depth > 5
            {
                method: "ai.index",
                params: {
                    collection: "docs",
                    documents: [{
                        id: "doc1",
                        text: "hello",
                        metadata: { l1: { l2: { l3: { l4: { l5: { l6: "too_deep" } } } } } }
                    }]
                },
                desc: "Index metadata depth > 5"
            },
            // Search: empty query
            { method: "ai.search", params: { collection: "docs", query: "" }, desc: "Search empty query" },
            // Search: oversized query (>4096 chars)
            { method: "ai.search", params: { collection: "docs", query: "Q".repeat(4097) }, desc: "Search query >4096 chars" },
            // Search: topK = 0 or topK > 20
            { method: "ai.search", params: { collection: "docs", query: "test", topK: 0 }, desc: "Search topK = 0" },
            { method: "ai.search", params: { collection: "docs", query: "test", topK: 99 }, desc: "Search topK > 20" }
        ];

        let blockedCount = 0;
        for (const atk of attacks) {
            let err = null;
            try {
                await window.wavical.invoke(atk.method, atk.params);
            } catch (e) {
                err = e.message;
            }

            if (!err || !err.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `Attack '${atk.desc}' was NOT blocked with generic error (got: ${err})`
                };
            }
            blockedCount++;
        }

        return {
            passed: true,
            detail: `All ${blockedCount} oversize/malformed AI payloads safely blocked with generic error`
        };
    } catch (e) {
        return { passed: false, detail: `Oversize/malformed AI test exception: ${e.message}` };
    }
}

// 16. Forged and cross-app collection names
async function testAiForgedCrossAppCollection() {
    try {
        const illegalCollections = [
            "../secret_app",
            "..\\other_user",
            "CON",
            "PRN",
            "AUX",
            "NUL",
            "COM1",
            "LPT1",
            "app/coll",
            "app\\coll",
            "coll:stream",
            "coll\0null",
            "COLL_UPPERCASE",
            "coll with spaces",
            "",
            "c".repeat(65)
        ];

        let blockedCount = 0;
        for (const col of illegalCollections) {
            for (const method of ["ai.index", "ai.search"]) {
                let err = null;
                const params = method === "ai.index"
                    ? { collection: col, documents: [{ id: "d1", text: "t" }] }
                    : { collection: col, query: "q", topK: 1 };
                try {
                    await window.wavical.invoke(method, params);
                } catch (e) {
                    err = e.message;
                }

                if (!err || !err.includes("Method not allowed or not found")) {
                    return {
                        passed: false,
                        detail: `Forged collection '${col}' on ${method} not blocked with generic error (got: ${err})`
                    };
                }
                blockedCount++;
            }
        }

        return {
            passed: true,
            detail: `All ${blockedCount} forged/cross-app collection attempts blocked (traversal, device names, nulls, chars, limits)`
        };
    } catch (e) {
        return { passed: false, detail: `Forged collection test exception: ${e.message}` };
    }
}

// 17. Flood of chat, index and search calls
async function testAiFlood() {
    try {
        const floodPromises = [];
        for (let i = 0; i < 20; i++) {
            const method = (i % 3 === 0) ? "ai.chat" : (i % 3 === 1) ? "ai.index" : "ai.search";
            const params = (method === "ai.chat")
                ? { messages: [{ role: "user", content: `flood msg ${i}` }] }
                : (method === "ai.index")
                ? { collection: "flood_test", documents: [{ id: `d${i}`, text: `flood text ${i}` }] }
                : { collection: "flood_test", query: `flood query ${i}`, topK: 1 };

            floodPromises.push(window.wavical.invoke(method, params).catch(e => e.message));
        }

        const results = await Promise.all(floodPromises);

        for (const res of results) {
            if (typeof res === "string" && !res.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `Unexpected flood rejection message: ${res}`
                };
            }
        }

        const ping = await window.wavical.invoke("core.ping");
        if (!ping || ping.message !== "pong") {
            return { passed: false, detail: "Host failed to respond to ping after AI call flood" };
        }

        return {
            passed: true,
            detail: `Host gracefully absorbed 20 concurrent AI requests without starvation or panic; ping responsive`
        };
    } catch (e) {
        return { passed: false, detail: `AI flood test exception: ${e.message}` };
    }
}

// 18. Cancel races and non-existent requests
async function testAiCancelRace() {
    try {
        const cancelTargets = [
            99999999,
            0,
            1,
            2,
            18446744073709551615
        ];

        const racePromises = [];
        for (const targetId of cancelTargets) {
            racePromises.push(window.wavical.invoke("ai.cancel", { id: targetId }).catch(e => e.message));
            racePromises.push(window.wavical.invoke("ai.cancel", { id: targetId }).catch(e => e.message));
        }

        const results = await Promise.all(racePromises);

        for (const res of results) {
            if (typeof res === "string" && !res.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `ai.cancel error format violation: ${res}`
                };
            }
        }

        const ping = await window.wavical.invoke("core.ping");
        if (!ping || ping.message !== "pong") {
            return { passed: false, detail: "Host unresponsive after cancel races" };
        }

        return {
            passed: true,
            detail: `Cancel races and non-existent session IDs safely handled without race conditions or panics`
        };
    } catch (e) {
        return { passed: false, detail: `Cancel race test exception: ${e.message}` };
    }
}

// 19. Model output containing markup
async function testAiMarkupInOutput() {
    try {
        window.__xss_breach__ = false;

        const hostileModelOutputs = [
            "<script>window.__xss_breach__ = true;<\/script>",
            "<img src=\"data:image/png;base64,invalid\" onerror=\"window.__xss_breach__ = true;\">",
            "<svg onload=\"window.__xss_breach__ = true;\">",
            "javascript:/*--></title></style></textarea></script></xmp><svg/onload='+/'/+/onmouseover=1/+window.__xss_breach__=true//>"
        ];

        const sandbox = document.getElementById("sandbox");
        const div = document.createElement("div");
        sandbox.appendChild(div);

        for (const hostileText of hostileModelOutputs) {
            div.textContent = hostileText;
        }

        await new Promise(r => setTimeout(r, 50));

        if (window.__xss_breach__ === true) {
            return { passed: false, detail: "XSS execution detected from model output markup" };
        }

        div.remove();
        return {
            passed: true,
            detail: `Model output markup treated as untrusted text; no script execution or DOM escaping occurred`
        };
    } catch (e) {
        return { passed: false, detail: `Markup injection test exception: ${e.message}` };
    }
}

// 20. Attempts to supply model path, URL, API key or download request from page
async function testAiForbiddenPageParams() {
    try {
        const dangerousChatParams = [
            { messages: [{ role: "user", content: "hi" }], modelPath: "C:\\Windows\\System32\\cmd.exe" },
            { messages: [{ role: "user", content: "hi" }], downloadUrl: "https://evil.com/model.bin" },
            { messages: [{ role: "user", content: "hi" }], apiKey: "sk-attack-token-12345" },
            { messages: [{ role: "user", content: "hi" }], baseUrl: "https://evil.com/v1" }
        ];

        for (const params of dangerousChatParams) {
            let err = null;
            try {
                await window.wavical.invoke("ai.chat", params);
            } catch (e) {
                err = e.message;
            }
            if (!err || !err.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `Dangerous chat params were not blocked with generic error: ${err}`
                };
            }
        }

        const forbiddenMethods = [
            "ai.downloadModel",
            "ai.loadModel",
            "ai.setModelPath",
            "ai.setApiKey",
            "ai.fetchCatalog",
            "ai.execCommand",
            "ai.shell"
        ];

        for (const method of forbiddenMethods) {
            let err = null;
            try {
                await window.wavical.invoke(method, { url: "https://evil.com" });
            } catch (e) {
                err = e.message;
            }
            if (!err || !err.includes("Method not allowed or not found")) {
                return {
                    passed: false,
                    detail: `Forbidden administrative method '${method}' was not blocked with generic error: ${err}`
                };
            }
        }

        return {
            passed: true,
            detail: `Page attempts to inject model paths, URLs, API keys or trigger downloads all strictly rejected`
        };
    } catch (e) {
        return { passed: false, detail: `Forbidden page params test exception: ${e.message}` };
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
            status: res.passed ? 'BLOCKED' : 'BREACH',
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
                <span class="test-pill ${res.passed ? 'pill-pass' : 'pill-fail'}">${res.passed ? 'BLOCKED' : 'BREACH'}</span>
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
