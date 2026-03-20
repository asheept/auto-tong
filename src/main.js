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
      list.appendChild(li);
    });
  }
}

function showToast(message) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.remove("hidden");
  setTimeout(() => toast.classList.add("hidden"), 2000);
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
    showToast("저장 실패: " + err);
  }
});

$("#btn-check-now").addEventListener("click", async () => {
  try {
    const result = await invoke("check_now");
    showToast(result || "확인 완료");
    loadHistory();
  } catch (err) {
    showToast("오류: " + err);
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
    loadHistory();
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

// Initialize
loadConfig();
loadHistory();
