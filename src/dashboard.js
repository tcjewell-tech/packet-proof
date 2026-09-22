document.addEventListener("DOMContentLoaded", () => {
  const durationSelect = document.getElementById("duration-select");
  const customInput = document.getElementById("custom-duration-input");
  durationSelect?.addEventListener("change", () => {
    customInput.style.display = durationSelect.value === "custom" ? "inline" : "none";
  });
});

let currentSessionId = null;

function setSessionRunning(running) {
  const start = document.getElementById("start-session");
  const stop = document.getElementById("stop-session");
  if (start) start.disabled = running;
  if (stop) stop.disabled = !running;
  const indicator = document.getElementById("session-status");
  if (indicator) indicator.textContent = running ? "Session running…" : "";
}

window.__TAURI__.event.listen("session-started", (event) => {
  currentSessionId = event.payload.session_id;
  setSessionRunning(true);
});

// A timed session stops itself; the backend has already cleared its state, so
// the controls have to come back exactly as they would after a manual stop.
window.__TAURI__.event.listen("session-ended", () => {
  setSessionRunning(false);
});

function currentDurationSecs() {
  const select = document.getElementById("duration-select");
  if (select.value === "custom") {
    const minutes = Number(document.getElementById("custom-duration-input").value || 0);
    return minutes > 0 ? minutes * 60 : null;
  }
  return Number(select.value);
}
window.currentDurationSecs = currentDurationSecs;

async function startSession(target, durationSecs) {
  await window.__TAURI__.core.invoke("start_session", { target, durationSecs });
}

async function stopSession() {
  await window.__TAURI__.core.invoke("stop_session");
  setSessionRunning(false);
}

function currentTarget() {
  const picked = document.querySelector('input[name="candidate-target"]:checked');
  return picked ? picked.value : document.getElementById("target-input").value;
}

async function exportSessionReport() {
  if (!currentSessionId) {
    alert("Start a session first.");
    return;
  }
  const path = await window.__TAURI__.core.invoke("export_session_report", { sessionId: currentSessionId });
  alert(`Report saved to ${path}`);
}

window.startSession = startSession;
window.stopSession = stopSession;
window.currentTarget = currentTarget;
window.exportSessionReport = exportSessionReport;
