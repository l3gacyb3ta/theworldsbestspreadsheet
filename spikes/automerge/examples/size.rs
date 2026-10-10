//! SPIKE (docs/design/automerge.md §9): rough size and speed numbers for a workbook-shaped
//! Automerge doc. Run with `cargo run --release -p automerge-spike --example size`.
//! Throwaway evidence, not product code.

use automerge::sync::{self, SyncDoc};
use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, ObjType, ROOT};
use std::time::Instant;

const CELLS: usize = 3000;
const SCRUB_FRAMES: usize = 600; // ten seconds of scrubbing at 60 fps

fn cell_value(i: usize) -> String {
    // about the size of a serialized formula with two id references
    format!("[{{\"T\":\"={i} \"}},{{\"R\":[\"{:016x}\",\"{:016x}\",0]}},{{\"T\":\" 2 * +\"}}]", i * 7919, i * 104729)
}

fn build() -> (AutoCommit, automerge::ObjId) {
    let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1u8; 16]));
    let sheet = doc.put_object(ROOT, "sheet", ObjType::Map).unwrap();
    let cells = doc.put_object(&sheet, "cells", ObjType::Map).unwrap();
    let pos = doc.put_object(&sheet, "row_pos", ObjType::Map).unwrap();
    for r in 0..300 {
        doc.put(&pos, format!("{:016x}", r * 31), format!("a{r:05}")).unwrap();
    }
    for i in 0..CELLS {
        doc.put(&cells, format!("{:016x}:{:016x}", (i / 10) * 31, i % 10), cell_value(i)).unwrap();
    }
    doc.commit();
    (doc, cells)
}

fn main() {
    let t = Instant::now();
    let (mut doc, cells) = build();
    println!("build {CELLS} cells: {:?}", t.elapsed());
    let bytes = doc.save();
    println!("saved size: {} KiB", bytes.len() / 1024);
    let t = Instant::now();
    let loaded = AutoCommit::load(&bytes).unwrap();
    println!("load: {:?} ({} keys)", t.elapsed(), automerge::ReadDoc::length(&loaded, &cells));

    // a peer that receives every scrub frame
    let mut peer = doc.fork().with_actor(ActorId::from(vec![2u8; 16]));
    let (mut s_out, mut s_in) = (sync::State::new(), sync::State::new());

    // one change per frame
    let key = format!("{:016x}:{:016x}", 0, 0);
    let t = Instant::now();
    let mut sync_time = std::time::Duration::ZERO;
    let mut wire = 0usize;
    for f in 0..SCRUB_FRAMES {
        doc.put(&cells, &key, format!("{f}")).unwrap();
        doc.commit();
        let ts = Instant::now();
        // exchange messages until both sides are quiet
        loop {
            let mut moved = false;
            if let Some(m) = doc.sync().generate_sync_message(&mut s_out) {
                let enc = m.encode();
                wire += enc.len();
                peer.sync().receive_sync_message(&mut s_in, sync::Message::decode(&enc).unwrap()).unwrap();
                moved = true;
            }
            if let Some(m) = peer.sync().generate_sync_message(&mut s_in) {
                let enc = m.encode();
                wire += enc.len();
                doc.sync().receive_sync_message(&mut s_out, sync::Message::decode(&enc).unwrap()).unwrap();
                moved = true;
            }
            if !moved {
                break;
            }
        }
        sync_time += ts.elapsed();
    }
    let total = t.elapsed();
    println!(
        "{SCRUB_FRAMES} per-frame changes: {:?} per frame total, of which sync {:?}; {} bytes on the wire per frame",
        total / SCRUB_FRAMES as u32,
        sync_time / SCRUB_FRAMES as u32,
        wire / SCRUB_FRAMES
    );
    println!("saved size after per-frame scrub: {} KiB", doc.save().len() / 1024);

    // the same scrub coalesced to one change per 100 ms
    let (mut doc2, cells2) = build();
    let before = doc2.save().len();
    for f in 0..SCRUB_FRAMES / 6 {
        doc2.put(&cells2, &key, format!("{f}")).unwrap();
        doc2.commit();
    }
    println!(
        "coalesced ({} changes): +{} KiB vs per-frame +{} KiB",
        SCRUB_FRAMES / 6,
        (doc2.save().len() - before) / 1024,
        (doc.save().len() - bytes.len()) / 1024
    );

    // applying one remote change and reading what it touched
    let mut a = doc.fork().with_actor(ActorId::from(vec![3u8; 16]));
    let mut b = doc.fork().with_actor(ActorId::from(vec![4u8; 16]));
    b.put(&cells, &key, "remote").unwrap();
    b.commit();
    a.update_diff_cursor();
    let t = Instant::now();
    a.merge(&mut b).unwrap();
    let patches = a.diff_incremental();
    println!("merge one remote change + patches: {:?} ({} patch)", t.elapsed(), patches.len());
}
