const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const path = require("node:path");

test("update notice only receives event listening and window closing", () => {
  const folder = path.join(__dirname, "../src-tauri/capabilities");
  const settings = JSON.parse(fs.readFileSync(path.join(folder, "default.json"), "utf8"));
  const notice = JSON.parse(fs.readFileSync(path.join(folder, "update-notice.json"), "utf8"));
  assert.deepEqual(settings.windows, ["settings"]);
  assert.deepEqual(notice.windows, ["update-blocked"]);
  assert.deepEqual(notice.permissions, ["core:event:allow-listen", "core:window:allow-close"]);
});
