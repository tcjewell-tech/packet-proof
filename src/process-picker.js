async function loadProcessList() {
  const processes = await window.__TAURI__.core.invoke("list_candidate_processes");
  const select = document.getElementById("process-select");
  select.innerHTML = processes
    .map((p) => `<option value="${p.pid}">${p.name} (pid ${p.pid})</option>`)
    .join("");
}

async function loadConnectionsForSelectedProcess() {
  const pid = Number(document.getElementById("process-select").value);
  const result = await window.__TAURI__.core.invoke("list_connections_for_pid", { pid });
  const container = document.getElementById("connection-list");

  if (result.status === "none_detected") {
    container.innerHTML =
      "<p>No active connections detected. If this is a game using unconnected UDP sockets, " +
      "Packet Proof can't see its remote server through OS connection tables in this version " +
      "— enter the target IP manually instead.</p>";
    return;
  }

  container.innerHTML = result.endpoints
    .map(
      (e) => `
      <label>
        <input type="radio" name="candidate-target" value="${e.remote_ip}" />
        ${e.remote_ip}:${e.remote_port} (${e.protocol.toUpperCase()})
      </label>`
    )
    .join("<br/>");
}

window.loadProcessList = loadProcessList;
window.loadConnectionsForSelectedProcess = loadConnectionsForSelectedProcess;
