import assert from "node:assert/strict";
import test from "node:test";

test("CI adapter substitutes only picker paths with immutable native IPC intact", async () => {
  const calls = [];
  const original = async (...args) => { calls.push(args); return "synthetic backend result"; };
  const internals = {};
  Object.defineProperty(internals, "invoke", { value: original });
  globalThis.window = { __TAURI_INTERNALS__: internals };
  try {
    const { invoke } = await import("./tauri-core.js");
    const state = window.__nativeAcceptance;
    assert.equal(state.marker, "MEMIRO_NATIVE_ACCEPTANCE_DIALOG_ADAPTER");
    state.dialogs.push({ command: "plugin:dialog|open", value: "synthetic.wav" });
    assert.equal(await invoke("plugin:dialog|open", { multiple: false }), "synthetic.wav");
    assert.equal(calls.length, 0);
    state.dialogs.push({ command: "plugin:dialog|save", value: "synthetic.txt" });
    assert.equal(await invoke("plugin:dialog|save"), "synthetic.txt");
    await assert.rejects(invoke("plugin:dialog|open"), /Unexpected OS file picker/);
    const args = { id: "synthetic", options: { model: "parakeet-tdt-0.6b-v3", language: "ru" } };
    const options = { headers: { "synthetic-header": "synthetic-value" } };
    assert.equal(await invoke("transcribe", args, options), "synthetic backend result");
    assert.deepEqual(calls, [["transcribe", args, options]]);
    assert.deepEqual(state.recognition, { model: "parakeet-tdt-0.6b-v3", language: "ru", cloud: false });
    assert.equal(internals.invoke, original);
    assert.equal(Object.getOwnPropertyDescriptor(internals, "invoke").writable, false);
  } finally { delete globalThis.window; }
});
