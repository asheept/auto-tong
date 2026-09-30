const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);

async function loadConfig() {
  const config = await invoke("get_config");
  $("#drive_sync_folder").value = config.drive_sync_folder || "";
  $("#prismlauncher_exe").value = config.prismlauncher_exe || "";
  $("#prismlauncher_data_dir").value = config.prismlauncher_data_dir || "";
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
      const isFailed = item.status === "failed" || item.status === "retry_failed" || item.status === "migration_pending" || item.status === "cancelled";

      if (item.status === "launcher_warning") {
        li.classList.add("history-warning");
        li.textContent = item.path;
        const warning = document.createElement("span");
        warning.className = "history-retained";
        warning.textContent = " · 설치 완료, 런처 재실행 확인 필요";
        li.appendChild(warning);
      } else if (isFailed) {
        li.classList.add("history-failed");
        const xSpan = document.createElement("span");
        xSpan.className = "history-x";
        xSpan.textContent = "X";
        li.appendChild(xSpan);
        li.appendChild(document.createTextNode(" " + item.path));
        if (item.status === "retry_failed" && item.installed) {
          const retained = document.createElement("span");
          retained.className = "history-retained";
          retained.textContent = " · 재시도 실패, 기존 설치 유지";
          li.appendChild(retained);
        } else if (item.status === "migration_pending") {
          const pending = document.createElement("span");
          pending.className = "history-retained";
          pending.textContent = " · 이전 이력 연결 확인 필요";
          li.appendChild(pending);
        } else if (item.status === "cancelled") {
          const cancelled = document.createElement("span");
          cancelled.className = "history-retained";
          cancelled.textContent = item.installed ? " · 사용자가 취소함, 기존 설치 유지" : " · 사용자가 취소함";
          li.appendChild(cancelled);
        }
      } else {
        li.textContent = item.path;
      }

      li.tabIndex = 0;
      li.setAttribute("role", "button");
      li.addEventListener("click", () => showReimportModal(item.path, li));
      li.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          showReimportModal(item.path, li);
        }
      });
      list.appendChild(li);
    });
  }
}

let toastTimer = null;
function showToast(message, isError = false, duration = 2000) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.remove("hidden", "toast-error", "toast-success");
  toast.classList.add(isError ? "toast-error" : "toast-success");
  clearTimeout(toastTimer);
  toastTimer = duration > 0 ? setTimeout(() => toast.classList.add("hidden"), duration) : null;
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

$("#btn-pick-prism-data").addEventListener("click", async () => {
  const current = $("#prismlauncher_data_dir").value;
  const selected = await open({ directory: true, title: "PrismLauncher 데이터 폴더 선택", defaultPath: current || undefined });
  if (selected) $("#prismlauncher_data_dir").value = selected;
});

$("#settings-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const saveButton = $("#btn-save");
  if (saveButton.disabled) return;

  const tags = $("#subscribed_tags")
    .value.split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  if (tags.some((tag) => !tag.replace(/^@+/, "").trim())) {
    showToast("구독 태그에는 @ 또는 공백만 입력할 수 없습니다", true);
    return;
  }

  const config = {
    drive_sync_folder: $("#drive_sync_folder").value,
    prismlauncher_exe: $("#prismlauncher_exe").value,
    prismlauncher_data_dir: $("#prismlauncher_data_dir").value,
    subscribed_tags: tags,
    poll_interval_secs: parseInt($("#poll_interval_secs").value) || 60,
    autostart: $("#autostart").checked,
  };

  try {
    saveButton.disabled = true;
    showToast("설정 저장 및 적용 확인 중...", false, 0);
    const applied = await invoke("save_config", { newConfig: config });
    showToast(`설정 ${applied.revision} 적용 완료 (${applied.watchMode === "event" ? "파일 감시" : "주기 확인"})`);
  } catch (err) {
    showToast("설정 처리 실패: " + err, true);
  } finally {
    saveButton.disabled = false;
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
let cancelRequestedJobId = null;
let activeImportJobId = null;
const progressController = window.createImportProgressController({
  render: ({ job_id, file_name, percent, status, phase, terminal }) => {
  const section = $("#progress-section");
  const fill = $("#progress-fill");
  const nameEl = $("#progress-filename");
  const percentEl = $("#progress-percent");
  const statusEl = $("#progress-status");

  activeImportJobId = job_id;

  section.classList.remove("hidden");
  nameEl.textContent = file_name;
  percentEl.textContent = percent + "%";
  fill.style.width = percent + "%";
  statusEl.textContent = status;
  const cancelButton = $("#btn-cancel-import");
  cancelButton.classList.toggle("hidden", terminal || phase === "recording");
  cancelButton.disabled = cancelRequestedJobId === job_id;
  },
  hide: () => $("#progress-section").classList.add("hidden"),
  onTerminal: () => {
    loadHistory().catch((err) => console.error("이력 갱신 실패:", err));
  },
});
listen("import-progress", (event) => progressController.receive(event.payload));

$("#btn-cancel-import").addEventListener("click", async () => {
  const button = $("#btn-cancel-import");
  cancelRequestedJobId = activeImportJobId;
  button.disabled = true;
  try {
    const result = await invoke("cancel_import");
    showToast(result);
  } catch (error) {
    button.disabled = false;
    showToast("취소 요청 실패: " + error, true);
  }
});

// Update check
let currentUpdateStatus = null;
function renderUpdateStatus(incoming) {
  const next = window.applyUpdateStatus(currentUpdateStatus, incoming);
  if (next === currentUpdateStatus) return;
  currentUpdateStatus = next;
  $("#btn-update").disabled = next.running;
  $("#btn-update").textContent = next.buttonText;
  $("#update-status").textContent = next.message;
}
listen("update-status", (event) => renderUpdateStatus(event.payload));
invoke("get_update_status").then(renderUpdateStatus).catch((error) => console.error("업데이트 상태 조회 실패:", error));

$("#btn-update").addEventListener("click", async () => {
  const btn = $("#btn-update");
  btn.disabled = true;
  btn.textContent = "업데이트 중...";

  try {
    await invoke("check_update");
  } catch (err) {
    showToast("업데이트: " + err, true);
  } finally {
    invoke("get_update_status").then(renderUpdateStatus).catch((error) => console.error("업데이트 상태 조회 실패:", error));
  }
});

// Reimport modal
let reimportTarget = null;
let reimportTrigger = null;

function showReimportModal(relativePath, trigger) {
  reimportTarget = relativePath;
  reimportTrigger = trigger;
  $("#reimport-modal-msg").textContent = `"${relativePath}" 을(를) 다시 가져오시겠습니까?`;
  $("#reimport-modal").classList.remove("hidden");
  $("#reimport-cancel").focus();
}

function closeReimportModal() {
  $("#reimport-modal").classList.add("hidden");
  reimportTarget = null;
  reimportTrigger?.focus();
  reimportTrigger = null;
}

$("#reimport-cancel").addEventListener("click", () => {
  closeReimportModal();
});

$("#reimport-confirm").addEventListener("click", async () => {
  if (!reimportTarget) return;
  const target = reimportTarget;
  closeReimportModal();

  try {
    const result = await invoke("reimport", { relativePath: target });
    showToast(result);
  } catch (err) {
    showToast("실패: " + err, true);
  }
});

// Close modal on overlay click
$("#reimport-modal").addEventListener("click", (e) => {
  if (e.target === e.currentTarget) {
    closeReimportModal();
  }
});

$("#reimport-modal").addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    closeReimportModal();
  } else if (event.key === "Tab") {
    const cancel = $("#reimport-cancel");
    const confirm = $("#reimport-confirm");
    if (event.shiftKey && document.activeElement === cancel) {
      event.preventDefault();
      confirm.focus();
    } else if (!event.shiftKey && document.activeElement === confirm) {
      event.preventDefault();
      cancel.focus();
    }
  }
});

// Initialize
loadConfig().catch((err) => showToast("설정 로드 실패: " + err, true));
loadHistory().catch((err) => showToast("이력 로드 실패: " + err, true));
