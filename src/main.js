const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);

async function loadConfig() {
  const config = await invoke("get_config");
  $("#drive_sync_folder").value = config.drive_sync_folder || "";
  $("#prismlauncher_exe").value = config.prismlauncher_exe || "";
  $("#subscribed_tags").value = (config.subscribed_tags || []).join(", ");
  $("#poll_interval_secs").value = config.poll_interval_secs || 60;
  $("#autostart").checked = config.autostart ?? true;
}

async function loadHistory() {
  const history = await invoke("get_import_history");
  const list = $("#history-list");
  const empty = $("#history-empty");

  list.innerHTML = "";
  if (history.length === 0) {
    empty.style.display = "block";
  } else {
    empty.style.display = "none";
    history.slice(-5).reverse().forEach((item) => {
      const li = document.createElement("li");
      li.textContent = item;
      li.addEventListener("click", () => showReimportModal(item));
      list.appendChild(li);
    });
  }
}

let toastTimer = null;
function showToast(message, isError = false) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.remove("hidden", "toast-error", "toast-success");
  toast.classList.add(isError ? "toast-error" : "toast-success");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.add("hidden"), 2000);
}

$("#btn-pick-drive").addEventListener("click", async () => {
  const current = $("#drive_sync_folder").value;
  const selected = await open({ directory: true, title: "Drive 동기화 폴더 선택", defaultPath: current || undefined });
  if (selected) {
    $("#drive_sync_folder").value = selected;
  }
});

$("#btn-pick-prism").addEventListener("click", async () => {
  const current = $("#prismlauncher_exe").value;
  const selected = await open({
    filters: [{ name: "실행파일", extensions: ["exe"] }],
    title: "PrismLauncher 선택",
    defaultPath: current || undefined,
  });
  if (selected) {
    $("#prismlauncher_exe").value = selected;
  }
});

$("#settings-form").addEventListener("submit", async (e) => {
  e.preventDefault();

  const tags = $("#subscribed_tags")
    .value.split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);

  const config = {
    drive_sync_folder: $("#drive_sync_folder").value,
    prismlauncher_exe: $("#prismlauncher_exe").value,
    subscribed_tags: tags,
    poll_interval_secs: parseInt($("#poll_interval_secs").value) || 60,
    autostart: $("#autostart").checked,
  };

  try {
    await invoke("save_config", { newConfig: config });
    showToast("저장되었습니다");
  } catch (err) {
    showToast("저장 실패: " + err, true);
  }
});

$("#btn-check-now").addEventListener("click", async () => {
  const btn = $("#btn-check-now");
  btn.disabled = true;
  try {
    const result = await invoke("check_now");
    showToast(result || "확인 완료");
    await loadHistory();
  } catch (err) {
    showToast("오류: " + err, true);
  } finally {
    btn.disabled = false;
  }
});

// Progress bar
let progressTimer = null;

listen("import-progress", (event) => {
  const { file_name, percent, status } = event.payload;
  const section = $("#progress-section");
  const fill = $("#progress-fill");
  const nameEl = $("#progress-filename");
  const percentEl = $("#progress-percent");
  const statusEl = $("#progress-status");

  section.classList.remove("hidden");
  nameEl.textContent = file_name;
  percentEl.textContent = percent + "%";
  fill.style.width = percent + "%";
  statusEl.textContent = status;

  if (percent >= 100) {
    loadHistory().catch((err) => console.error("이력 갱신 실패:", err));
    clearTimeout(progressTimer);
    progressTimer = setTimeout(() => {
      section.classList.add("hidden");
    }, 3000);
  } else if (status === "실패") {
    clearTimeout(progressTimer);
    progressTimer = setTimeout(() => {
      section.classList.add("hidden");
    }, 3000);
  }
});

// Update check
$("#btn-update").addEventListener("click", async () => {
  const btn = $("#btn-update");
  const status = $("#update-status");
  btn.disabled = true;
  btn.textContent = "확인 중...";
  status.textContent = "";

  try {
    const result = await invoke("check_update");
    status.textContent = result;
  } catch (err) {
    status.textContent = "오류: " + err;
  }

  btn.disabled = false;
  btn.textContent = "업데이트 확인";
});

// Reimport modal
let reimportTarget = null;

function showReimportModal(relativePath) {
  reimportTarget = relativePath;
  $("#reimport-modal-msg").textContent = `"${relativePath}" 을(를) 다시 가져오시겠습니까?`;
  $("#reimport-modal").classList.remove("hidden");
}

$("#reimport-cancel").addEventListener("click", () => {
  $("#reimport-modal").classList.add("hidden");
  reimportTarget = null;
});

$("#reimport-confirm").addEventListener("click", async () => {
  $("#reimport-modal").classList.add("hidden");
  if (!reimportTarget) return;

  try {
    const result = await invoke("reimport", { relativePath: reimportTarget });
    showToast(result);
  } catch (err) {
    showToast("실패: " + err, true);
  }
  reimportTarget = null;
});

// Close modal on overlay click
$("#reimport-modal").addEventListener("click", (e) => {
  if (e.target === e.currentTarget) {
    $("#reimport-modal").classList.add("hidden");
    reimportTarget = null;
  }
});

// Initialize
loadConfig().catch((err) => showToast("설정 로드 실패: " + err, true));
loadHistory().catch((err) => showToast("이력 로드 실패: " + err, true));
