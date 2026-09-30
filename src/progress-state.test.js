const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const { createImportProgressController } = require("./progress-state.js");

test("fixture uses the terminal job contract", () => {
  const event = JSON.parse(fs.readFileSync(path.join(__dirname, "fixtures/import-progress.json"), "utf8"));
  assert.equal(event.phase, "completed");
  assert.equal(event.terminal, true);
  assert.equal(event.job_id, 7);
});

test("old timers and events cannot hide or replace a newer job", () => {
  const timers = new Map();
  let nextTimer = 0;
  const shown = [];
  let hides = 0;
  let terminalCount = 0;
  const controller = createImportProgressController({
    render: (event) => shown.push(event.job_id),
    hide: () => hides++,
    onTerminal: () => terminalCount++,
    setTimer: (fn) => { const id = ++nextTimer; timers.set(id, fn); return id; },
    clearTimer: (id) => timers.delete(id),
  });
  controller.receive({ job_id: 1, phase: "completed", terminal: true });
  const staleTimer = timers.get(1);
  controller.receive({ job_id: 2, phase: "downloading", terminal: false, percent: 100 });
  staleTimer();
  controller.receive({ job_id: 1, phase: "failed", terminal: true });
  assert.deepEqual(shown, [1, 2]);
  assert.equal(hides, 0);
  assert.equal(terminalCount, 1);
  controller.receive({ job_id: 2, phase: "completed", terminal: true });
  timers.get(2)();
  assert.equal(hides, 1);
});

test("recovery-required status remains visible until another job resolves it", () => {
  const timers = [];
  const shown = [];
  let hides = 0;
  const controller = createImportProgressController({
    render: (event) => shown.push(event.phase),
    hide: () => hides++,
    onTerminal: () => {},
    setTimer: (fn) => { timers.push(fn); return timers.length; },
    clearTimer: () => {},
  });
  controller.receive({ job_id: 10, phase: "recovery_required", terminal: true });
  assert.deepEqual(shown, ["recovery_required"]);
  assert.equal(timers.length, 0);
  assert.equal(hides, 0);
});

test("completed install with launcher restart error remains visible", () => {
  let timerCount = 0;
  const controller = createImportProgressController({
    render: () => {}, hide: () => {}, onTerminal: () => {},
    setTimer: () => { timerCount++; return timerCount; }, clearTimer: () => {},
  });
  controller.receive({ job_id: 11, phase: "completed", terminal: true, error: "launcher did not restart" });
  assert.equal(timerCount, 0);
});
