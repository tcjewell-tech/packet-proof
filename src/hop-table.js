function renderHopTable(hops) {
  const rows = hops
    .map(
      (h) => `
    <tr>
      <td>${h.hop_index}</td>
      <td>${h.addr ?? "—"}</td>
      <td>${h.rtt_best_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_avg_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.rtt_worst_ms?.toFixed(1) ?? "—"}</td>
      <td>${h.loss_pct.toFixed(0)}%</td>
    </tr>`
    )
    .join("");

  document.getElementById("hop-table-body").innerHTML = rows;
}

window.__TAURI__.event.listen("hop-update", (event) => {
  renderHopTable(event.payload);
});

window.__TAURI__.core.invoke("start_demo_session").catch((err) => {
  console.error("start_demo_session failed:", err);
});
