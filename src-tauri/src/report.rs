use crate::db::ProbeRecord;

#[derive(Debug, PartialEq)]
pub struct SessionSummary {
    pub target: String,
    pub session_id: String,
    pub hop_count: usize,
    pub avg_loss_pct: f64,
    pub worst_hop_rtt_ms: Option<f64>,
}

pub fn summarize_session(records: &[ProbeRecord]) -> SessionSummary {
    if records.is_empty() {
        return SessionSummary {
            target: String::new(),
            session_id: String::new(),
            hop_count: 0,
            avg_loss_pct: 0.0,
            worst_hop_rtt_ms: None,
        };
    }

    let target = records[0].target.clone();
    let session_id = records[0].session_id.clone().unwrap_or_default();
    let hop_count = records.iter().map(|r| r.hop_index).collect::<std::collections::HashSet<_>>().len();
    let avg_loss_pct = records.iter().map(|r| r.loss_pct).sum::<f64>() / records.len() as f64;
    let worst_hop_rtt_ms = records.iter().filter_map(|r| r.rtt_worst_ms).fold(None, |acc: Option<f64>, v| {
        Some(acc.map_or(v, |a| a.max(v)))
    });

    SessionSummary { target, session_id, hop_count, avg_loss_pct, worst_hop_rtt_ms }
}

use genpdf::elements::{Break, Paragraph, TableLayout};
use genpdf::{Document, SimplePageDecorator};

pub fn generate_session_report(records: &[ProbeRecord], out_path: &std::path::Path) -> Result<(), String> {
    let summary = summarize_session(records);

    let font_family = genpdf::fonts::from_files("./fonts", "LiberationSans", None)
        .map_err(|e| format!("failed to load fonts: {e}"))?;
    let mut doc = Document::new(font_family);
    doc.set_title("Packet Proof — Session Report");
    let mut decorator = SimplePageDecorator::new();
    decorator.set_margins(10);
    doc.set_page_decorator(decorator);

    doc.push(Paragraph::new(format!("Session Report — {}", summary.target)));
    doc.push(Paragraph::new(format!("Session ID: {}", summary.session_id)));
    doc.push(Paragraph::new(format!("Hops: {}", summary.hop_count)));
    doc.push(Paragraph::new(format!("Average loss: {:.1}%", summary.avg_loss_pct)));
    doc.push(Paragraph::new(format!(
        "Worst hop RTT: {}",
        summary.worst_hop_rtt_ms.map(|v| format!("{v:.1}ms")).unwrap_or_else(|| "—".to_string())
    )));
    doc.push(Break::new(1));

    let mut table = TableLayout::new(vec![1, 2, 1, 1, 1, 1, 1]);
    table.set_cell_decorator(genpdf::elements::FrameCellDecorator::new(true, true, false));
    let header = ["Hop", "IP", "Best", "Avg", "Worst", "Loss %", "Zone"];
    let mut row = table.row();
    for h in header {
        row.push_element(Paragraph::new(h));
    }
    row.push().map_err(|e| format!("{e}"))?;

    let mut by_hop: Vec<&ProbeRecord> = records.iter().collect();
    by_hop.sort_by_key(|r| r.hop_index);
    by_hop.dedup_by_key(|r| r.hop_index);

    for r in by_hop {
        let mut row = table.row();
        row.push_element(Paragraph::new(r.hop_index.to_string()));
        row.push_element(Paragraph::new(r.hop_ip.clone().unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_best_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_avg_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(r.rtt_worst_ms.map(|v| format!("{v:.1}")).unwrap_or_else(|| "—".to_string())));
        row.push_element(Paragraph::new(format!("{:.0}%", r.loss_pct)));
        row.push_element(Paragraph::new(r.zone.clone()));
        row.push().map_err(|e| format!("{e}"))?;
    }
    doc.push(table);

    doc.render_to_file(out_path).map_err(|e| format!("failed to write pdf: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hop: u8, loss: f64, worst: Option<f64>) -> ProbeRecord {
        ProbeRecord {
            session_type: "timed".to_string(),
            target: "1.1.1.1".to_string(),
            session_id: Some("s1".to_string()),
            timestamp: 1000,
            hop_index: hop,
            hop_ip: Some("1.1.1.1".to_string()),
            rtt_best_ms: Some(1.0),
            rtt_avg_ms: Some(2.0),
            rtt_worst_ms: worst,
            loss_pct: loss,
            zone: "beyond_isp".to_string(),
        }
    }

    #[test]
    fn summarizes_hop_count_avg_loss_and_worst_rtt() {
        let records = vec![record(1, 0.0, Some(5.0)), record(2, 10.0, Some(40.0)), record(3, 20.0, Some(15.0))];
        let summary = summarize_session(&records);
        assert_eq!(summary.target, "1.1.1.1");
        assert_eq!(summary.session_id, "s1");
        assert_eq!(summary.hop_count, 3);
        assert!((summary.avg_loss_pct - 10.0).abs() < 0.001);
        assert_eq!(summary.worst_hop_rtt_ms, Some(40.0));
    }

    #[test]
    fn handles_no_records() {
        let summary = summarize_session(&[]);
        assert_eq!(summary.hop_count, 0);
        assert_eq!(summary.avg_loss_pct, 0.0);
        assert_eq!(summary.worst_hop_rtt_ms, None);
    }

    // Not run by default (`generate_session_report` renders a real PDF via
    // genpdf, which isn't meaningfully assertable at the byte level — this
    // is a manual smoke check, not a regression test). Run explicitly with
    // `cargo test report::tests -- --ignored --nocapture` from `src-tauri`
    // (relies on the relative `./fonts` path resolving against the crate
    // root, which is cargo's cwd for `cargo test`).
    #[test]
    #[ignore]
    fn generates_a_real_pdf_from_synthetic_records() {
        let records = vec![
            record(1, 0.0, Some(5.0)),
            record(2, 10.0, Some(40.0)),
            record(3, 20.0, Some(15.0)),
        ];
        let out_path = std::env::temp_dir().join("packet-proof-report-smoke-test.pdf");
        let _ = std::fs::remove_file(&out_path);

        generate_session_report(&records, &out_path).expect("PDF generation should succeed");

        let bytes = std::fs::read(&out_path).expect("output file should exist");
        assert!(!bytes.is_empty(), "PDF file should not be empty");
        assert!(bytes.starts_with(b"%PDF-"), "output should start with the PDF magic header");
        println!("wrote {} bytes to {}", bytes.len(), out_path.display());
    }
}
