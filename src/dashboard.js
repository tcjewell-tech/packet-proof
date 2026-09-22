async function startSession(target) {
  await window.__TAURI__.core.invoke("start_session", { target });
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
