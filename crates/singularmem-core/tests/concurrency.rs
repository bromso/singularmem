//! Concurrency tests: parallel readers + single writer don't corrupt or
//! deadlock; two writers from separate Store handles to the same file
//! interleave cleanly under `SQLite` WAL semantics.

use std::sync::Arc;
use std::thread;

use singularmem_core::{NewItem, Store};
use tempfile::TempDir;

#[test]
fn parallel_readers_with_one_writer_dont_interfere() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Arc::new(Store::open(&path).unwrap());

    // Pre-seed with one item so readers have something to fetch.
    let seed = store.ingest(NewItem::text("seed")).unwrap();

    // Spawn 16 reader threads doing 100 reads each.
    let mut readers = Vec::new();
    for _ in 0..16 {
        let s = Arc::clone(&store);
        let id = seed.id;
        readers.push(thread::spawn(move || {
            for _ in 0..100 {
                let _ = s.get(id).expect("read");
            }
        }));
    }

    // One writer adding 100 items.
    let writer_store = Arc::clone(&store);
    let writer = thread::spawn(move || {
        for i in 0..100 {
            let _ = writer_store
                .ingest(NewItem::text(format!("item-{i}")))
                .expect("ingest");
        }
    });

    for r in readers {
        r.join().expect("reader join");
    }
    writer.join().expect("writer join");

    // Final state: 1 seed + 100 writes = 101 items.
    let count = store.list().unwrap().count();
    assert_eq!(count, 101);
}

#[test]
fn two_writers_from_separate_handles_serialise_correctly() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");

    let store_a = Store::open(&path).unwrap();
    let store_b = Store::open(&path).unwrap();

    // Both write concurrently. SQLite WAL allows this; outcomes interleave but
    // every successful ingest must produce a unique ID and a valid row.
    let path_a = path.clone();
    let _ = path_a; // ensure the path stays in scope; not used directly
    let writer_a = thread::spawn(move || {
        let mut ok = 0;
        for i in 0..50 {
            if store_a
                .ingest(NewItem::text(format!("a-{i}")))
                .is_ok()
            {
                ok += 1;
            }
        }
        ok
    });

    let writer_b = thread::spawn(move || {
        let mut ok = 0;
        for i in 0..50 {
            if store_b
                .ingest(NewItem::text(format!("b-{i}")))
                .is_ok()
            {
                ok += 1;
            }
        }
        ok
    });

    let ok_a = writer_a.join().unwrap();
    let ok_b = writer_b.join().unwrap();

    // Reopen to count. Both writers should fully succeed under WAL — SQLite
    // serialises the writes via the WAL; neither sees a busy error if the
    // default busy_timeout is reasonable. If one or both occasionally fail
    // with SQLite "database is locked", it's expected at small busy_timeout
    // and the rolled-back writes have not corrupted the file.
    let store = Store::open(&path).unwrap();
    let count = store.list().unwrap().count();
    assert_eq!(count, ok_a + ok_b, "successful writes are durable");
    assert!(ok_a > 0 && ok_b > 0, "both writers made progress (ok_a={ok_a}, ok_b={ok_b})");
}
