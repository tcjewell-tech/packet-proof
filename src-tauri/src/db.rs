use rusqlite::Connection;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProbeRecord {
    pub session_type: String,
    pub target: String,
    pub session_id: Option<String>,
    pub timestamp: i64,
    pub hop_index: u8,
    pub hop_ip: Option<String>,
    pub rtt_best_ms: Option<f64>,
    pub rtt_avg_ms: Option<f64>,
    pub rtt_worst_ms: Option<f64>,
    pub loss_pct: f64,
    pub zone: String,
}

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS probe_records (
  id            INTEGER PRIMARY KEY,
  session_type  TEXT NOT NULL,
  target        TEXT NOT NULL,
  session_id    TEXT,
  timestamp     INTEGER NOT NULL,
  hop_index     INTEGER NOT NULL,
  hop_ip        TEXT,
  rtt_best_ms   REAL,
  rtt_avg_ms    REAL,
  rtt_worst_ms  REAL,
  loss_pct      REAL NOT NULL,
  zone          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_probe_records_target_time ON probe_records(target, timestamp);
CREATE INDEX IF NOT EXISTS idx_probe_records_session ON probe_records(session_id);
";

impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert_record(&self, record: &ProbeRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO probe_records
             (session_type, target, session_id, timestamp, hop_index, hop_ip,
              rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                record.session_type,
                record.target,
                record.session_id,
                record.timestamp,
                record.hop_index,
                record.hop_ip,
                record.rtt_best_ms,
                record.rtt_avg_ms,
                record.rtt_worst_ms,
                record.loss_pct,
                record.zone,
            ],
        )?;
        Ok(())
    }

    pub fn query_history(
        &self,
        target: &str,
        session_type_filter: Option<&str>,
        since: i64,
        until: i64,
    ) -> rusqlite::Result<Vec<ProbeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_type, target, session_id, timestamp, hop_index, hop_ip,
                    rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone
             FROM probe_records
             WHERE target = ?1
               AND timestamp BETWEEN ?2 AND ?3
               AND (?4 IS NULL OR session_type = ?4)
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![target, since, until, session_type_filter],
            |row| {
                Ok(ProbeRecord {
                    session_type: row.get(0)?,
                    target: row.get(1)?,
                    session_id: row.get(2)?,
                    timestamp: row.get(3)?,
                    hop_index: row.get(4)?,
                    hop_ip: row.get(5)?,
                    rtt_best_ms: row.get(6)?,
                    rtt_avg_ms: row.get(7)?,
                    rtt_worst_ms: row.get(8)?,
                    loss_pct: row.get(9)?,
                    zone: row.get(10)?,
                })
            },
        )?;

        rows.collect()
    }

    pub fn query_session(&self, session_id: &str) -> rusqlite::Result<Vec<ProbeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_type, target, session_id, timestamp, hop_index, hop_ip,
                    rtt_best_ms, rtt_avg_ms, rtt_worst_ms, loss_pct, zone
             FROM probe_records
             WHERE session_id = ?1
             ORDER BY timestamp ASC, hop_index ASC",
        )?;

        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok(ProbeRecord {
                session_type: row.get(0)?,
                target: row.get(1)?,
                session_id: row.get(2)?,
                timestamp: row.get(3)?,
                hop_index: row.get(4)?,
                hop_ip: row.get(5)?,
                rtt_best_ms: row.get(6)?,
                rtt_avg_ms: row.get(7)?,
                rtt_worst_ms: row.get(8)?,
                loss_pct: row.get(9)?,
                zone: row.get(10)?,
            })
        })?;

        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        Db::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn sample(session_type: &str, target: &str, ts: i64, hop: u8) -> ProbeRecord {
        ProbeRecord {
            session_type: session_type.to_string(),
            target: target.to_string(),
            session_id: None,
            timestamp: ts,
            hop_index: hop,
            hop_ip: Some("1.1.1.1".to_string()),
            rtt_best_ms: Some(5.0),
            rtt_avg_ms: Some(6.0),
            rtt_worst_ms: Some(8.0),
            loss_pct: 0.0,
            zone: "beyond_isp".to_string(),
        }
    }

    #[test]
    fn insert_and_query_round_trip() {
        let db = test_db();
        let rec = sample("baseline", "1.1.1.1", 1000, 1);
        db.insert_record(&rec).unwrap();

        let results = db.query_history("1.1.1.1", None, 0, 2000).unwrap();
        assert_eq!(results, vec![rec]);
    }

    #[test]
    fn query_filters_by_target() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 1000, 1)).unwrap();
        db.insert_record(&sample("baseline", "8.8.8.8", 1000, 1)).unwrap();

        let results = db.query_history("1.1.1.1", None, 0, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target, "1.1.1.1");
    }

    #[test]
    fn query_filters_by_time_range() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 500, 1)).unwrap();
        db.insert_record(&sample("baseline", "1.1.1.1", 1500, 1)).unwrap();
        db.insert_record(&sample("baseline", "1.1.1.1", 2500, 1)).unwrap();

        let results = db.query_history("1.1.1.1", None, 1000, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].timestamp, 1500);
    }

    #[test]
    fn query_filters_by_session_type() {
        let db = test_db();
        db.insert_record(&sample("baseline", "1.1.1.1", 1000, 1)).unwrap();
        db.insert_record(&sample("timed", "1.1.1.1", 1000, 1)).unwrap();

        let results = db.query_history("1.1.1.1", Some("timed"), 0, 2000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_type, "timed");
    }

    #[test]
    fn query_session_returns_only_that_sessions_records() {
        let db = test_db();
        let mut a = sample("timed", "1.1.1.1", 1000, 1);
        a.session_id = Some("session-a".to_string());
        let mut b = sample("timed", "1.1.1.1", 1000, 1);
        b.session_id = Some("session-b".to_string());
        db.insert_record(&a).unwrap();
        db.insert_record(&b).unwrap();

        let results = db.query_session("session-a").unwrap();
        assert_eq!(results, vec![a]);
    }
}
