async function startSession(target) {
  await window.__TAURI__.core.invoke("start_session", { target });
}

async function stopSession() {
  await window.__TAURI__.core.invoke("stop_session");
}

window.startSession = startSession;
window.stopSession = stopSession;
