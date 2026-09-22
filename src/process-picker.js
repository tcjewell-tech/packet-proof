async function loadProcessList() {
  const processes = await window.__TAURI__.core.invoke("list_candidate_processes");
  const select = document.getElementById("process-select");
  select.innerHTML = "";
  for (const p of processes) {
    const opt = document.createElement("option");
    opt.value = String(p.pid);
    opt.textContent = `${p.name} (pid ${p.pid})`;
    select.appendChild(opt);
  }
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

  container.innerHTML = "";
  result.endpoints.forEach((e, index) => {
    if (index > 0) {
      container.appendChild(document.createElement("br"));
    }

    const label = document.createElement("label");

    const input = document.createElement("input");
    input.type = "radio";
    input.name = "candidate-target";
    input.value = e.remote_ip;
    label.appendChild(input);

    label.appendChild(
      document.createTextNode(` ${e.remote_ip}:${e.remote_port} (${e.protocol.toUpperCase()})`)
    );

    container.appendChild(label);
  });
}

window.loadProcessList = loadProcessList;
window.loadConnectionsForSelectedProcess = loadConnectionsForSelectedProcess;
