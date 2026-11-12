// Drive the web control surface in a real Chrome, over CDP.
//
// Not a substitute for the headless engine tests, which cover the screens far more
// thoroughly than anything here does. This covers the one thing they cannot: the
// browser half of `packages/nsh-app/src/backend.rs` — whether the wasm build that
// `nsh package --target web` produces actually talks to `packages/server` (the wire
// format, the session cookie, the guard, the error decoding) and renders what the
// server answers rather than an empty interface.
//
// Chrome's DevTools protocol rather than Playwright or Puppeteer because it needs
// no install: Node's own global `WebSocket` speaks it. `--no-proxy-server` is not
// cosmetic — this app's own system proxy is a loopback listener, and a machine
// configured to use it would otherwise have Chrome send the page request there.
//
// Run it through `scripts/e2e-web.sh`, which builds the site, starts a server on a
// scratch profile directory, waits for the listener and tears it all down.
import { spawn } from "node:child_process";

const chrome_path =
    process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const port = Number(process.env.CDP_PORT || 9333);
const url = process.env.E2E_URL;
if (!url) {
    console.error("E2E_URL is required; run scripts/e2e-web.sh");
    process.exit(2);
}
// Boot time for an 11 MB wasm module on a cold engine, plus the reads the boot
// effect makes before the overview can answer anything. Generous on purpose: a
// page read half-built fails these checks for a reason that looks like a bug.
const wait_ms = Number(process.env.E2E_WAIT || 20000);
// Joined rather than concatenated: `TMPDIR` ends in a separator and `/tmp` does
// not, and `"/tmp" + "kitty-…"` is a path no one meant.
const scratch = (name) =>
    (process.env.TMPDIR || "/tmp").replace(/\/?$/, "/") + name;

const chrome = spawn(
    chrome_path,
    [
        `--remote-debugging-port=${port}`,
        "--headless=new",
        "--no-first-run",
        "--no-default-browser-check",
        "--no-proxy-server",
        `--user-data-dir=${scratch(`kitty-e2e-chrome-${port}`)}`,
    ],
    { stdio: "ignore" },
);
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const bail = (code, message) => {
    chrome.kill();
    console.error(message);
    process.exit(code);
};

let targets = [];
for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
        targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
        if (targets.length) break;
    } catch {
        // Still starting.
    }
    await wait(300);
}
if (!targets.length) bail(1, "FAIL: Chrome's DevTools port never answered");

const page = targets.find((target) => target.type === "page");
const socket = new WebSocket(page.webSocketDebuggerUrl);
let sequence = 0;
const pending = new Map();
const consoleLines = [];
socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
        pending.get(message.id)(message);
        pending.delete(message.id);
    }
    if (message.method === "Runtime.consoleAPICalled") {
        consoleLines.push(
            message.params.args.map((arg) => arg.value ?? arg.description ?? "").join(" "),
        );
    }
    if (message.method === "Runtime.exceptionThrown") {
        consoleLines.push(
            `EXCEPTION: ${message.params.exceptionDetails?.exception?.description || ""}`,
        );
    }
});
const send = (method, params = {}) =>
    new Promise((resolve) => {
        sequence += 1;
        pending.set(sequence, resolve);
        socket.send(JSON.stringify({ id: sequence, method, params }));
    });
await new Promise((resolve) => socket.addEventListener("open", resolve));

await send("Runtime.enable");
await send("Page.enable");
await send("Page.navigate", { url });
await wait(wait_ms);

const evaluate = async (expression, awaitPromise = false) => {
    const answer = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise });
    if (answer.result?.exceptionDetails) {
        return `EVAL_ERROR: ${answer.result.exceptionDetails.text}`;
    }
    return answer.result?.result?.value;
};

// Three things, in the order a user meets them: does the page show what the server
// said, can it be interacted with, can it write. `.` in the regex below deliberately
// does not cross a line, so it captures one row of the rendered text.
const rendered = await evaluate(`JSON.stringify({
    title: document.title,
    core: /EMBEDDED.{0,24}/.exec(document.body?.innerText || "")?.[0] ?? null,
    systemProxy: (document.body?.innerText || "").includes("系统代理"),
    ready: (document.body?.innerText || "").includes("内核已就绪"),
    length: (document.body?.innerText || "").length,
})`);

const clicked = await evaluate(`(() => {
    const leaf = [...document.querySelectorAll("*")].find(
        (node) => node.children.length === 0
            && (node.textContent || "").trim() === "测试连通",
    );
    if (!leaf) return "NO_BUTTON";
    (leaf.closest("[role=button], button, [class*=button]") || leaf).click();
    return "CLICKED";
})()`);

// A write made from inside the page, which is the only way to prove the session
// cookie rides along and that the server's reply parses back into the client's own
// types rather than into a `serde_json::Value` nothing looks at.
const written = await evaluate(`(async () => {
    const profile = await (await fetch("/api/profile")).json();
    profile.subscriptions = [{
        id: 7, name: "E2E 订阅", source: "https://sub.example/link/abc",
        nodes: [], proxy_server_nameservers: [], rejected_count: 0,
    }];
    const saved = await fetch("/api/profile", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ profile }),
    });
    const back = await (await fetch("/api/profile")).json();
    return JSON.stringify({
        saveStatus: saved.status,
        names: (back.subscriptions || []).map((s) => s.name),
    });
})()`, true);

socket.close();
chrome.kill();

if (typeof rendered !== "string") bail(1, `FAIL: the page never rendered (${rendered})`);
const report = JSON.parse(rendered);
console.log(`RENDERED: ${rendered}`);
console.log(`CLICK:    ${clicked}`);
console.log(`WRITE:    ${written}`);
const errors = consoleLines.filter((line) => /error|EXCEPTION|refus|denied/i.test(line));
console.log(`CONSOLE:  ${consoleLines.length} line(s), ${errors.length} error(s)`);
for (const line of errors.slice(0, 5)) console.log(`  ${line}`);

const failures = [];
if (!report.core) failures.push("the core's version is not on screen: no data reached the page");
if (!report.ready) failures.push("the core's status is not on screen");
if (!report.systemProxy) failures.push("the system-proxy row is not on screen");
if (clicked !== "CLICKED") failures.push(`the 测试连通 button could not be pressed (${clicked})`);
if (typeof written !== "string") {
    failures.push(`the write could not be made (${written})`);
} else {
    const wrote = JSON.parse(written);
    if (wrote.saveStatus !== 200) failures.push(`saving a profile answered ${wrote.saveStatus}`);
    if (!wrote.names?.includes("E2E 订阅")) {
        failures.push(`the saved subscription did not come back (${JSON.stringify(wrote.names)})`);
    }
}
if (errors.length) failures.push(`${errors.length} console error(s)`);

if (failures.length) {
    for (const failure of failures) console.error(`FAIL: ${failure}`);
    process.exit(1);
}
console.log("\nOK: a browser drove the server end to end.");
