function drawLineChart(canvas, series, { color, yMax, yMin = 0 }) {
  const ctx = canvas.getContext("2d");
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);

  if (series.length < 2) return;

  const xs = series.map((p) => p.x);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const spanX = maxX - minX || 1;
  const spanY = yMax - yMin || 1;

  ctx.strokeStyle = color;
  ctx.lineWidth = 2;
  ctx.beginPath();
  series.forEach((point, i) => {
    const px = ((point.x - minX) / spanX) * width;
    const py = height - ((point.y - yMin) / spanY) * height;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  });
  ctx.stroke();
}

async function loadBaselineCharts() {
  const target = document.getElementById("chart-target-select").value;
  const rangeHours = Number(document.getElementById("chart-range-select").value);
  const until = Math.floor(Date.now() / 1000);
  const since = until - rangeHours * 3600;

  const records = await window.__TAURI__.core.invoke("get_baseline_history", { target, since, until });

  // One record per hop per tick; the chart cares about the final
  // (highest-index) hop per timestamp, i.e. reachability of the target itself.
  const byTimestamp = new Map();
  for (const r of records) {
    const existing = byTimestamp.get(r.timestamp);
    if (!existing || r.hop_index > existing.hop_index) {
      byTimestamp.set(r.timestamp, r);
    }
  }
  const finalHopRecords = [...byTimestamp.values()].sort((a, b) => a.timestamp - b.timestamp);

  const rttSeries = finalHopRecords
    .filter((r) => r.rtt_avg_ms != null)
    .map((r) => ({ x: r.timestamp, y: r.rtt_avg_ms }));
  const lossSeries = finalHopRecords.map((r) => ({ x: r.timestamp, y: r.loss_pct }));

  const maxRtt = Math.max(10, ...rttSeries.map((p) => p.y));
  drawLineChart(document.getElementById("rtt-chart"), rttSeries, { color: "#1976d2", yMax: maxRtt });
  drawLineChart(document.getElementById("loss-chart"), lossSeries, { color: "#d32f2f", yMax: 100 });
}

window.loadBaselineCharts = loadBaselineCharts;

async function exportTrendReport() {
  const target = document.getElementById("chart-target-select").value;
  const rangeSelect = document.getElementById("chart-range-select");
  const rangeHours = Number(rangeSelect.value);
  const rangeLabel = rangeSelect.selectedOptions[0].textContent;
  const until = Math.floor(Date.now() / 1000);
  const since = until - rangeHours * 3600;

  const path = await window.__TAURI__.core.invoke("export_trend_report", { target, since, until, rangeLabel });
  alert(`Report saved to ${path}`);
}
window.exportTrendReport = exportTrendReport;
