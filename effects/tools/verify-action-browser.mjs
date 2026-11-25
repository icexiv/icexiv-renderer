// Usage: node verify-action-browser.mjs 'http://localhost:8000/examples/action.html?...' [frame.png]
// Supply a Stone action, dressed actor, both hands, and the preview from docs/action.md.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, output] = process.argv.slice(2);
if (!url) throw Error("expected a complete action example URL");
const profile = mkdtempSync(join(tmpdir(), "renderer-action-"));
const browser = spawn(process.env.CHROMIUM || "chromium", [
  "--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-vulkan=native",
  "--ignore-gpu-blocklist", "--disable-background-timer-throttling",
  "--disable-renderer-backgrounding", "--no-first-run", "--no-proxy-server",
  "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });
const exited = once(browser, "exit");
const timeout = setTimeout(() => browser.kill(), 90_000);
try {
  let port, log = "";
  for await (const chunk of browser.stderr) {
    log = (log + chunk).slice(-8192);
    port = log.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)/)?.[1];
    if (port) break;
  }
  if (!port) throw Error(`Chromium did not start: ${log}`);
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const ws = new WebSocket(targets.find(t => t.type === "page").webSocketDebuggerUrl);
  await once(ws, "open");
  const pending = new Map();
  const errors = [];
  let id = 0;
  ws.onclose = () => { for (const { reject } of pending.values()) reject(Error("Chromium closed")); };
  ws.onmessage = ({data}) => {
    const m = JSON.parse(data);
    if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails);
    const p = pending.get(m.id);
    if (!p) return;
    pending.delete(m.id);
    if (m.error) p.reject(Error(m.error.message)); else p.resolve(m.result);
  };
  const send = (method, params={}) => new Promise((resolve,reject) => {
    pending.set(++id,{resolve,reject}); ws.send(JSON.stringify({id,method,params}));
  });
  const evaluate = async expression => {
    const r=await send("Runtime.evaluate",{expression,awaitPromise:true,returnByValue:true});
    if(r.exceptionDetails) throw Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  await send("Runtime.enable");
  await send("Page.enable");
  await send("Page.navigate",{url});
  // Wait for the new document (not about:blank) and for the example's worker.
  for (;;) {
    const status = await evaluate("document.querySelector('#state')?.textContent");
    if (status?.startsWith("Action 119")) break;
    if (status && !status.includes("正在加载")) throw Error(status);
    await new Promise(r => setTimeout(r, 100));
  }
  await evaluate(`globalThis.v=document.querySelector("icexiv-equipment-canvas");v.pauseScene();true`);
  const replay = () => evaluate(`(async()=>{
    await v.replayEffect([1,2,3,4]);
    for(let i=0;i<5;i++)await v.stepScene(6);
    return v.randomState();
  })()`);
  const state = await replay();
  assert.equal(state.length,4);
  assert.notDeepEqual(state,[1,2,3,4]);
  const clip = await evaluate(`(()=>{let {x,y,width,height}=v.shadowRoot.querySelector("canvas").getBoundingClientRect();
    return {x,y,width,height,scale:1}})()`);
  const image = async () => (await send("Page.captureScreenshot",{clip})).data;
  const baseline = await image();
  await evaluate("v.stepScene(1)");
  const at31 = await image();
  assert.deepEqual(await replay(),state);
  await evaluate("v.setActorPlacement([0.3,0,0],[0,0.38268343,0,0.92387953])");
  await evaluate("v.stepScene(1)");
  assert.ok(await image() !== at31,"actor placement did not change pixels at tick 31");
  await evaluate("v.setActorPlacement([0,0,0])");
  assert.deepEqual(await replay(),state,"replay RNG differs");
  assert.ok(await image() === baseline, "replayed frame differs");
  assert.deepEqual(errors,[]);
  if (output) writeFileSync(output,Buffer.from(baseline,"base64"));
  console.log("Stone: 30 ticks; world placement, RNG replay and canvas readback passed");
  ws.close();
} finally {
  clearTimeout(timeout);
  browser.kill();
  await exited;
  rmSync(profile,{recursive:true,force:true});
}
