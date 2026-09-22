use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};

use crate::db::{Db, ProbeRecord};

pub fn spawn_db_writer(db_path: PathBuf) -> Sender<ProbeRecord> {
    let (tx, rx) = mpsc::channel::<ProbeRecord>();

    std::thread::spawn(move || {
        let db = Db::open(&db_path).expect("failed to open sqlite db");
        for record in rx {
            if let Err(e) = db.insert_record(&record) {
                eprintln!("db write failed: {e}");
            }
        }
    });

    tx
}
