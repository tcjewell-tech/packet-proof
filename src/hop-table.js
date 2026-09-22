const ZONE_LABELS = { home: "Home", isp_gateway: "ISP Gateway", beyond_isp: "Beyond ISP" };
const ZONE_CLASSES = { home: "zone-home", isp_gateway: "zone-gateway", beyond_isp: "zone-beyond" };

function cell(text) {
  const td = document.createElement("td");
  td.textContent = text;
  return td;
}

function renderHopTable(hops) {
  // Built with createElement/textContent rather than innerHTML, matching the
  // pattern process-picker.js already uses. Every value here comes from
  // backend-typed fields, so this is not currently exploitable -- it is the
  // one string-interpolated innerHTML site left in the frontend, and leaving
  // it would leave the habit.
  const body = document.getElementById("hop-table-body");
  body.textContent = "";

  for (const h of hops) {
    const tr = document.createElement("tr");
    const zoneClass = ZONE_CLASSES[h.zone];
    if (zoneClass) tr.className = zoneClass;

    tr.appendChild(cell(String(h.hop_index)));
    tr.appendChild(cell(h.addr ?? "—"));
    tr.appendChild(cell(h.rtt_best_ms?.toFixed(1) ?? "—"));
    tr.appendChild(cell(h.rtt_avg_ms?.toFixed(1) ?? "—"));
    tr.appendChild(cell(h.rtt_worst_ms?.toFixed(1) ?? "—"));
    tr.appendChild(cell(`${h.loss_pct.toFixed(0)}%`));
    tr.appendChild(cell(ZONE_LABELS[h.zone] ?? "—"));

    body.appendChild(tr);
  }
}

window.__TAURI__.event.listen("hop-update", (event) => {
  renderHopTable(event.payload);
});
