// Real WebView2 UI + Tauri backend on a fresh GitHub-hosted Windows runner.
// Only OS file-picker results are supplied by the harness. No audio capture.
// CDP setup: https://playwright.dev/docs/webview2
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import net from "node:net";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { chromium } from "playwright-core";

assert.equal(process.platform, "win32");
assert.equal(process.env.GITHUB_ACTIONS, "true", "Use a disposable CI runner only");
assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
assert.ok(process.env.RUNNER_TEMP && process.env.APPDATA && process.env.LOCALAPPDATA);
const appData = path.join(process.env.APPDATA, "com.3uxo.app");
assert.ok(!existsSync(appData), "Refuse to open a pre-existing Memiro profile");
const installer = path.resolve(process.argv[2] ?? "");
assert.ok(installer.endsWith("-setup.exe") && existsSync(installer));
const manifest = JSON.parse(readFileSync(path.join(path.dirname(installer), "BUILD-MANIFEST.json"), "utf8").replace(/^\uFEFF/, ""));
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(sha256(readFileSync(installer)), manifest.sha256);
const sourceSha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
assert.equal(manifest.source_sha, sourceSha);
const launch = JSON.parse(readFileSync("target/native-acceptance-launch.json", "utf8"));
assert.equal(launch.source_sha, sourceSha);
assert.deepEqual(manifest.native_launch, launch);
assert.equal(launch.configuration_sha256, sha256(readFileSync("target/native-acceptance-config.json")));
const work = realpathSync(launch.work_directory);
const relative = path.relative(realpathSync(process.env.RUNNER_TEMP), work);
assert.ok(relative && !relative.startsWith("..") && !path.isAbsolute(relative));
assert.match(path.basename(work), /^memiro-native-/);
assert.equal(launch.window_label, "main");
assert.equal(launch.webview_data_directory, path.join(process.env.LOCALAPPDATA, launch.window_label, "MemiroNativeAcceptance", path.basename(work)));
assert.ok(!existsSync(launch.webview_data_directory), "WebView2 profile must be fresh");
const output = path.resolve("native-acceptance-results");
mkdirSync(work, { recursive: true });
mkdirSync(output, { recursive: true });
const report = {
  source_sha: sourceSha, installer_sha256: manifest.sha256,
  configuration: manifest.configuration, started_at: new Date().toISOString(),
  platform: process.platform, node: process.version, checks: [],
  boundary: "Real unsigned installed app, WebView2, SQLite, local model inference and file writes. OS open/save pickers return controlled paths. No microphone, paid AI, real call, upgrade or uninstall acceptance.",
};
const check = (name, evidence) => report.checks.push({ name, passed: true, evidence });
const fixture = path.resolve("tests/fixtures/synthetic-ru.wav");
const provenance = JSON.parse(readFileSync("tests/fixtures/synthetic-ru.json", "utf8"));
assert.equal(sha256(readFileSync(fixture)), provenance.sha256);
const invalid = path.join(work, "synthetic-invalid.wav");
writeFileSync(invalid, "Synthetic invalid audio for the import error path.");

async function run(executable, args) {
  const child = spawn(executable, args, { shell: false, windowsHide: true, stdio: "inherit" });
  await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => code === 0 ? resolve() : reject(new Error(`Owned installer failed: ${code}/${signal}`)));
  });
}
const server = net.createServer();
const port = launch.cdp_port;
assert.ok(Number.isInteger(port) && port > 1024 && port < 65536);
await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
await new Promise((resolve) => server.close(resolve));
const installed = path.join(work, "installed");
await run(installer, ["/S", `/D=${installed}`]);
assert.ok(!existsSync(appData), "Silent installation must not launch another app instance");
const executable = path.join(installed, "Memiro.exe");
assert.ok(existsSync(executable), "Silent installer must install the actual executable");
check("fresh silent NSIS installation", { executable_sha256: sha256(readFileSync(executable)) });
let app;
let browser;
let page;
let stdout = "", stderr = "";
try {
  app = spawn(executable, [], {
    shell: false, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env },
  });
  app.stdout.on("data", (bytes) => { stdout = (stdout + bytes.toString()).slice(-65536); });
  app.stderr.on("data", (bytes) => { stderr = (stderr + bytes.toString()).slice(-65536); });
  let startError;
  app.once("error", (error) => { startError = error; });
  for (let attempt = 0; attempt < 180; attempt++) {
    if (startError) throw startError;
    assert.equal(app.exitCode, null, "Installed app exited before WebView2 was ready");
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (response.ok) break;
    } catch { /* wait for the owned WebView2 process */ }
    await delay(500);
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const context = browser.contexts()[0];
  for (let attempt = 0; attempt < 120; attempt++) {
    page = context.pages().find((candidate) => /tauri|localhost/.test(candidate.url()));
    if (page) break;
    await delay(500);
  }
  assert.ok(page, "Actual Tauri page must be available");
  await page.getByRole("button", { name: "Импорт записи", exact: true }).waitFor();
  await page.evaluate(() => {
    const original = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
    window.__nativeAcceptance = { dialogs: [], calls: {}, original };
    window.__TAURI_INTERNALS__.invoke = async (command, args, options) => {
      const state = window.__nativeAcceptance;
      state.calls[command] = (state.calls[command] ?? 0) + 1;
      if (command === "transcribe") {
        state.recognition = { model: args.options.model, language: args.options.language, cloud: !!args.options.cloud };
      }
      if (command === "plugin:dialog|open" || command === "plugin:dialog|save") {
        const next = state.dialogs.shift();
        if (!next || next.command !== command) throw new Error("Unexpected OS file picker");
        return next.value;
      }
      return original(command, args, options);
    };
  });
  const picker = (command, value) => page.evaluate((item) => window.__nativeAcceptance.dialogs.push(item), { command, value });
  const invoke = (command, args) => page.evaluate(({ command, args }) => window.__nativeAcceptance.original(command, args), { command, args });
  const importFile = async (file) => {
    await page.getByRole("button", { name: "Импорт записи", exact: true }).click();
    await picker("plugin:dialog|open", file);
    await page.getByRole("button", { name: "Выбрать файл", exact: true }).click();
  };
  await importFile(invalid);
  await page.locator(".import-modal .ai-error").waitFor();
  assert.equal((await invoke("list_meetings")).length, 0);
  check("invalid import reports an error without a library row", true);
  await page.getByRole("button", { name: "Закрыть", exact: true }).click();
  await importFile(fixture);
  await page.getByRole("button", { name: "Расшифровать", exact: true }).waitFor();
  const [meeting] = await invoke("list_meetings");
  assert.ok(meeting && meeting.source === "imported");
  assert.equal(meeting.duration_secs, Math.floor(provenance.duration_seconds));
  check("real decoder import and SQLite library persistence", { source: meeting.source, duration_secs: meeting.duration_secs });
  await page.locator("select.speaker-count").selectOption("1");
  const started = Date.now();
  await page.getByRole("button", { name: "Расшифровать", exact: true }).click();
  await page.getByRole("button", { name: "Расшифровать заново", exact: true }).waitFor({ timeout: 20 * 60 * 1000 });
  const transcript = await invoke("get_transcript", { id: meeting.id });
  assert.ok(transcript?.segments?.length > 0);
  const text = transcript.segments.map((segment) => segment.text).join(" ");
  const words = new Set(text.toLocaleLowerCase("ru").match(/[\p{L}]+/gu));
  const expected = ["запись", "проверяем", "импорт", "распознавание", "сохранение", "текста"];
  const recognized = expected.filter((word) => words.has(word));
  assert.ok(recognized.length >= 5, `Expected at least five of six fixture keywords, got: ${text}`);
  for (const segment of transcript.segments) {
    assert.ok(Number.isFinite(segment.start_secs) && segment.start_secs >= 0);
    assert.ok(segment.end_secs > segment.start_secs && segment.end_secs <= provenance.duration_seconds + 0.5);
  }
  assert.ok((await page.locator(".transcript").innerText()).includes(transcript.segments[0].text));
  check("actual default local ASR through UI and persisted transcript", { text, recognized_keywords: recognized, first_run_seconds: (Date.now() - started) / 1000 });
  writeFileSync(path.join(output, "transcript.json"), JSON.stringify(transcript, null, 2));
  await page.screenshot({ path: path.join(output, "native-transcript.png") });
  const formats = [["docx", "Word"], ["md", "Markdown"], ["txt", "Текст"], ["srt", "Субтитры"]];
  for (const [extension, label] of formats) {
    await page.getByRole("button", { name: "⬇ Экспорт", exact: true }).click();
    const modal = page.locator(".export-modal");
    await modal.getByRole("button", { name: new RegExp(`^${label}`) }).click();
    if (extension === "txt") {
      await picker("plugin:dialog|save", path.join(work, "nonexistent-parent", "failed.txt"));
      await modal.getByRole("button", { name: "Сохранить файл", exact: true }).click();
      await modal.locator(".ai-error").waitFor();
      check("failed native export remains visible and can be retried", true);
    }
    const destination = path.join(output, `synthetic-transcript.${extension}`);
    await picker("plugin:dialog|save", destination);
    await modal.getByRole("button", { name: "Сохранить файл", exact: true }).click();
    await modal.waitFor({ state: "hidden" });
    const bytes = readFileSync(destination);
    assert.ok(bytes.length > 30);
    let exported = bytes.toString("utf8");
    if (extension === "docx") {
      exported = execFileSync("python", ["-c", "import sys,zipfile,xml.etree.ElementTree as E; z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; r=E.fromstring(z.read('word/document.xml')); print(' '.join(r.itertext()))", destination], { encoding: "utf8", env: { ...process.env, PYTHONIOENCODING: "utf-8" } });
    }
    for (const segment of transcript.segments) assert.ok(exported.includes(segment.text), `${extension} must contain every recognized segment`);
    if (extension === "srt") assert.match(exported, /\d\d:\d\d:\d\d,\d{3} --> \d\d:\d\d:\d\d,\d{3}/);
    check(`real ${extension} export file`, { bytes: bytes.length, sha256: sha256(bytes) });
  }
  const calls = await page.evaluate(() => window.__nativeAcceptance.calls);
  assert.deepEqual(await page.evaluate(() => window.__nativeAcceptance.recognition), {
    model: "parakeet-tdt-0.6b-v3", language: "ru", cloud: false,
  });
  for (const forbidden of ["start_recording", "generate_report", "ai_correct_transcript", "cloud_asr_check"]) assert.ok(!calls[forbidden], `Unexpected ${forbidden}`);
  assert.ok(calls.import_recording >= 2 && calls.transcribe === 1 && calls.save_binary_file === 1 && calls.save_text_file >= 4);
  assert.equal((await page.evaluate(() => window.__nativeAcceptance.dialogs.length)), 0);
  check("only OS file pickers substituted; real backend commands observed", calls);
  report.passed = true;
} catch (error) {
  report.passed = false;
  report.error = String(error.stack ?? error);
  if (page) await page.screenshot({ path: path.join(output, "failure.png") }).catch(() => {});
  throw error;
} finally {
  report.finished_at = new Date().toISOString();
  writeFileSync(path.join(output, "app-stdout.log"), stdout);
  writeFileSync(path.join(output, "app-stderr.log"), stderr);
  if (app?.pid) {
    report.owned_app = { pid: app.pid, exit_code: app.exitCode, cdp_port: port };
    try {
      const diagnostics = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-File", path.resolve("scripts/native-owned-processes.ps1"), "-OwnedProcessId", String(app.pid), "-ExpectedExecutable", executable], {
        windowsHide: true, encoding: "utf8", timeout: 15000, env: { ...process.env },
      });
      writeFileSync(path.join(output, "owned-processes.json"), diagnostics);
    } catch (error) { report.process_diagnostics_error = String(error); }
  }
  if (existsSync(path.join(appData, "3uxo.log"))) {
    writeFileSync(path.join(output, "app-backend.log"), readFileSync(path.join(appData, "3uxo.log")));
  }
  writeFileSync(path.join(output, "result.json"), JSON.stringify(report, null, 2));
  if (browser) await browser.close().catch(() => {});
  if (app?.pid && app.exitCode === null) {
    // Exact owned process tree only; never target processes by name or port.
    try { execFileSync("taskkill.exe", ["/PID", String(app.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" }); } catch { /* runner teardown also owns this disposable VM */ }
  }
}
