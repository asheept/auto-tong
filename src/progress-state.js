function createImportProgressController({ render, hide, onTerminal, setTimer = setTimeout, clearTimer = clearTimeout }) {
  let activeJobId = null;
  let terminal = false;
  let timer = null;

  return {
    receive(event) {
      if (!event || !Number.isSafeInteger(event.job_id) || event.job_id < 1) return;
      if (activeJobId !== null && event.job_id < activeJobId) return;
      if (event.job_id === activeJobId && terminal && !event.terminal) return;

      if (timer !== null) {
        clearTimer(timer);
        timer = null;
      }
      if (event.job_id !== activeJobId) {
        activeJobId = event.job_id;
        terminal = false;
      }
      render(event);
      if (event.terminal) {
        terminal = true;
        onTerminal(event);
        if (event.phase !== "recovery_required" && !event.error) {
          const completedJobId = event.job_id;
          timer = setTimer(() => {
            if (activeJobId === completedJobId && terminal) hide();
            timer = null;
          }, 3000);
        }
      }
    },
  };
}

if (typeof module !== "undefined") module.exports = { createImportProgressController };
if (typeof window !== "undefined") window.createImportProgressController = createImportProgressController;
