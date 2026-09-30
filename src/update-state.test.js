const assert = require("node:assert/strict");
const test = require("node:test");
const { applyUpdateStatus } = require("./update-state.js");

test("the button follows the active update and ignores older status", () => {
  const running = applyUpdateStatus(null, { revision: 2, phase: "installing", message: "설치 중", running: true });
  assert.equal(running.running, true);
  assert.equal(running.buttonText, "업데이트 중...");
  assert.equal(applyUpdateStatus(running, { revision: 1, running: false }), running);
  const completed = applyUpdateStatus(running, { revision: 3, phase: "completed", message: "완료", running: false });
  assert.equal(completed.running, false);
  assert.equal(completed.buttonText, "업데이트 확인");
});
