document.addEventListener("DOMContentLoaded", () => {
  const durationSelect = document.getElementById("duration-select");
  const customInput = document.getElementById("custom-duration-input");
  durationSelect?.addEventListener("change", () => {
    customInput.style.display = durationSelect.value === "custom" ? "inline" : "none";
  });
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
}

function currentTarget() {
  const picked = document.querySelector('input[name="candidate-target"]:checked');
  return picked ? picked.value : document.getElementById("target-input").value;
}

window.startSession = startSession;
window.stopSession = stopSession;
window.currentTarget = currentTarget;
