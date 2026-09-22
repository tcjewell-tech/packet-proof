let currentSessionId = null;

function setSessionRunning(running) {
  const start = document.getElementById("start-session");
  const stop = document.getElementById("stop-session");
  if (start) start.disabled = running;
  if (stop) stop.disabled = !running;
  const indicator = document.getElementById("session-status");
  if (indicator) indicator.textContent = running ? "Session running…" : "";
}

function setSessionError(message) {
  const el = document.getElementById("session-error");
  if (el) el.textContent = message;
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

/// True when "Custom" is selected but the box is empty, zero or not a number.
///
/// `currentDurationSecs()` returns null for that case, and the backend reads
/// a null duration as "run forever" — so without this check the user silently
/// gets an unbounded session instead of the one they asked for.
function customDurationIsInvalid() {
  const select = document.getElementById("duration-select");
  if (!select || select.value !== "custom") return false;
  const raw = document.getElementById("custom-duration-input")?.value ?? "";
  const minutes = Number(raw);
  return raw.trim() === "" || !Number.isFinite(minutes) || minutes <= 0;
}
window.customDurationIsInvalid = customDurationIsInvalid;

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

async function startSessionFromControls() {
  if (customDurationIsInvalid()) {
    setSessionError("Enter a custom duration of at least 1 minute, or choose a preset.");
    return;
  }
  setSessionError("");
  try {
    await startSession(currentTarget(), currentDurationSecs());
  } catch (err) {
    setSessionError(String(err));
  }
}

async function exportSessionReport() {
  if (!currentSessionId) {
    alert("Start a session first.");
    return;
  }
  const path = await window.__TAURI__.core.invoke("export_session_report", { sessionId: currentSessionId });
  alert(`Report saved to ${path}`);
}

// Wired here rather than with inline `onclick` attributes: those are inline
// scripts, which a `script-src 'self'` CSP blocks.
document.addEventListener("DOMContentLoaded", () => {
  const durationSelect = document.getElementById("duration-select");
  const customInput = document.getElementById("custom-duration-input");
  durationSelect?.addEventListener("change", () => {
    customInput.classList.toggle("hidden", durationSelect.value !== "custom");
    setSessionError("");
  });

  document.getElementById("start-session")?.addEventListener("click", startSessionFromControls);
  document.getElementById("stop-session")?.addEventListener("click", () => {
    stopSession().catch((err) => setSessionError(String(err)));
  });
  document.getElementById("export-session")?.addEventListener("click", () => {
    exportSessionReport().catch((err) => setSessionError(String(err)));
  });
});

window.startSession = startSession;
window.stopSession = stopSession;
window.currentTarget = currentTarget;
window.exportSessionReport = exportSessionReport;
