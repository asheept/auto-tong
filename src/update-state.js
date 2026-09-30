function applyUpdateStatus(current, incoming) {
  if (!incoming || !Number.isSafeInteger(incoming.revision)) return current;
  if (current && incoming.revision < current.revision) return current;
  return {
    revision: incoming.revision,
    message: incoming.message || "",
    running: Boolean(incoming.running),
    buttonText: incoming.running ? "업데이트 중..." : "업데이트 확인",
  };
}

if (typeof module !== "undefined") module.exports = { applyUpdateStatus };
if (typeof window !== "undefined") window.applyUpdateStatus = applyUpdateStatus;
