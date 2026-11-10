// CI-only build overlay: elevated WebView2 >=150 ignores environment overrides.
// https://github.com/MicrosoftEdge/WebView2Feedback/issues/5645
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import net from "node:net";
import path from "node:path";
assert.equal(process.platform, "win32");
assert.equal(process.env.GITHUB_ACTIONS, "true", "Use a disposable CI runner only");
assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
assert.ok(process.env.RUNNER_TEMP && process.env.APPDATA && process.env.LOCALAPPDATA);
assert.ok(!existsSync(path.join(process.env.APPDATA, "com.3uxo.app")), "Refuse an existing Memiro profile");
const config = JSON.parse(readFileSync("src-tauri/tauri.ci.conf.json", "utf8"));
const normal = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
assert.equal(normal.app.windows.length, 1);
const server = net.createServer();
await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
const port = server.address().port;
await new Promise((resolve) => server.close(resolve));
const work = mkdtempSync(path.join(process.env.RUNNER_TEMP, "memiro-native-"));
const relativeProfile = path.join("MemiroNativeAcceptance", path.basename(work));
const label = normal.app.windows[0].label ?? "main";
const profile = path.join(process.env.LOCALAPPDATA, label, relativeProfile);
assert.ok(!existsSync(profile));
config.app = { windows: normal.app.windows.map((window) => ({
  ...window,
  dataDirectory: relativeProfile,
  // Explicit API arguments survive elevated-host hardening. Preserve Wry defaults.
  additionalBrowserArgs: `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
})) };
mkdirSync("target", { recursive: true });
const bytes = JSON.stringify(config, null, 2);
writeFileSync("target/native-acceptance-config.json", bytes);
writeFileSync("target/native-acceptance-launch.json", JSON.stringify({
  source_sha: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  cdp_port: port, work_directory: work, webview_data_directory: profile, window_label: label,
  configuration_sha256: createHash("sha256").update(bytes).digest("hex"),
}, null, 2));
console.log("Prepared a fresh CI-only WebView2 profile and loopback CDP build overlay.");
