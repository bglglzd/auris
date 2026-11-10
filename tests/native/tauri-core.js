// Included only by Vite's explicit native-acceptance CI build mode.
// Keep the SDK intact; Tauri's injected invoke property is intentionally immutable.
export * from "../../node_modules/@tauri-apps/api/core.js";
import { invoke as nativeInvoke } from "../../node_modules/@tauri-apps/api/core.js";

const state = window.__nativeAcceptance = {
  marker: "MEMIRO_NATIVE_ACCEPTANCE_DIALOG_ADAPTER",
  dialogs: [], calls: {}, original: nativeInvoke,
};

export async function invoke(command, args, options) {
  state.calls[command] = (state.calls[command] ?? 0) + 1;
  if (command === "transcribe") {
    state.recognition = { model: args.options.model, language: args.options.language, cloud: !!args.options.cloud };
  }
  if (command === "plugin:dialog|open" || command === "plugin:dialog|save") {
    const next = state.dialogs.shift();
    if (!next || next.command !== command) throw new Error("Unexpected OS file picker");
    return next.value;
  }
  return nativeInvoke(command, args, options);
}
