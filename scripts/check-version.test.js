const assert = require("node:assert/strict");
const test = require("node:test");
const path = require("node:path");
const { checkVersion } = require("./check-version.js");

const root = path.resolve(__dirname, "..");

test("all package versions and the release tag agree", () => {
  assert.equal(checkVersion(root, "v0.2.9"), "0.2.9");
});

test("a mismatched release tag blocks publishing", () => {
  assert.throws(() => checkVersion(root, "v0.2.10"), /릴리스 태그/);
});
