const assert = require("node:assert/strict");
const test = require("node:test");
const path = require("node:path");
const { checkVersion } = require("./check-version.js");

const root = path.resolve(__dirname, "..");
const { version } = require("../package.json");

test("all package versions and the release tag agree", () => {
  assert.equal(checkVersion(root, `v${version}`), version);
});

test("a mismatched release tag blocks publishing", () => {
  assert.throws(() => checkVersion(root, `v${version}-unexpected-tag`), /릴리스 태그/);
});
