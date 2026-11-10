import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const readJson = (relative) => JSON.parse(readFileSync(new URL(relative, root), "utf8"));
const manifest = readJson("package.json");
// Cargo generates these package blocks. Keep every version so a duplicated
// native package cannot accidentally make the JavaScript check pass.
const packages = readFileSync(new URL("Cargo.lock", root), "utf8")
  .split(/^\[\[package\]\]\r?$/m)
  .slice(1)
  .map((block) => ({
    name: block.match(/^name = "([^"]+)"\r?$/m)?.[1],
    version: block.match(/^version = "([^"]+)"\r?$/m)?.[1],
  }));
const pairs = [["@tauri-apps/api", "tauri"]];
for (const name of Object.keys(manifest.dependencies)) {
  const match = name.match(/^@tauri-apps\/plugin-([a-z-]+)$/);
  if (match) pairs.push([name, `tauri-plugin-${match[1]}`]);
}

let failures = 0;
for (const [jsName, rustName] of pairs) {
  const jsVersion = readJson(`node_modules/${jsName}/package.json`).version;
  const native = packages.filter((pkg) => pkg.name === rustName);
  const minor = (version) => version?.match(/^(\d+\.\d+)\./)?.[1];
  if (native.length !== 1 || !minor(jsVersion) || minor(native[0]?.version) !== minor(jsVersion)) {
    console.error(`${jsName} ${jsVersion} does not match ${rustName} ${native.map((pkg) => pkg.version).join(", ") || "missing"}`);
    failures += 1;
  }
}
if (failures) {
  console.error(`Update the npm and Cargo locks together in ${fileURLToPath(root)}.`);
  process.exitCode = 1;
} else {
  console.log(`Validated ${pairs.length} JavaScript/Rust Tauri version pairs.`);
}
