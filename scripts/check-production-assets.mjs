import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
const assets = readdirSync("dist/assets").filter((file) => file.endsWith(".js"));
assert.ok(assets.length > 0, "Build the frontend first");
const found = assets.some((file) => readFileSync(path.join("dist/assets", file), "utf8").includes("MEMIRO_NATIVE_ACCEPTANCE_DIALOG_ADAPTER"));
assert.equal(found, process.argv.includes("--expect-native"), "Native picker adapter must appear only in the explicit CI build mode");
console.log(found ? "Native CI adapter present." : "Production assets exclude native CI adapter.");
