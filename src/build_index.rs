// src/build_index.rs
// Offline tool: reads references.json.gz → builds VP-Tree → writes binary index
//
// Binary format:
//   u32le: n_points
//   u32le: n_nodes
//   u32le: root_node_idx
//   [n_points × (14×f32le + u8 label + 3×u8 pad)]
//   [n_nodes  × (u32le vp_idx + f32le thresh_sq + u32le left + u32le right)]

use flate2::read::GzDecoder;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::time::Instant;
use fraud_api::{Point, build_vp_tree, DIMS};

#[derive(Deserialize)]
struct RefRecord {
    vector: [f32; 14],
    label: String,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: build-index <references.json.gz> <output.bin>");
        std::process::exit(1);
    }

    let t0 = Instant::now();
    eprintln!("[build-index] Reading {}...", args[1]);

    let file = File::open(&args[1]).expect("Cannot open input");
    let gz = GzDecoder::new(BufReader::with_capacity(8 * 1024 * 1024, file));
    let records: Vec<RefRecord> = serde_json::from_reader(gz).expect("JSON parse failed");

    let n = records.len();
    eprintln!("[build-index] Parsed {} records in {:.1}s", n, t0.elapsed().as_secs_f64());

    let t1 = Instant::now();
    let points: Vec<Point> = records.into_iter().map(|r| Point {
        v: r.vector,
        label: if r.label == "fraud" { 1 } else { 0 },
        _pad: [0; 3],
    }).collect();

    eprintln!("[build-index] Building VP-Tree for {} points...", n);
    let tree = build_vp_tree(points);
    eprintln!(
        "[build-index] Tree built: {} nodes in {:.1}s",
        tree.nodes.len(), t1.elapsed().as_secs_f64()
    );

    // Write binary
    let t2 = Instant::now();
    let out = File::create(&args[2]).expect("Cannot create output");
    let mut w = BufWriter::with_capacity(64 * 1024 * 1024, out);

    // Header
    w.write_all(&(tree.points.len() as u32).to_le_bytes()).unwrap();
    w.write_all(&(tree.nodes.len()  as u32).to_le_bytes()).unwrap();
    w.write_all(&tree.root.to_le_bytes()).unwrap();

    // Points
    for p in &tree.points {
        for &x in &p.v { w.write_all(&x.to_le_bytes()).unwrap(); }
        w.write_all(&[p.label, 0, 0, 0]).unwrap();
    }

    // Nodes
    for nd in &tree.nodes {
        w.write_all(&nd.vp_idx.to_le_bytes()).unwrap();
        w.write_all(&nd.thresh_sq.to_le_bytes()).unwrap();
        w.write_all(&nd.left.to_le_bytes()).unwrap();
        w.write_all(&nd.right.to_le_bytes()).unwrap();
    }

    w.flush().unwrap();
    let sz = std::fs::metadata(&args[2]).unwrap().len();
    eprintln!(
        "[build-index] Wrote {:.1} MB in {:.1}s. Total: {:.1}s",
        sz as f64 / 1_048_576.0,
        t2.elapsed().as_secs_f64(),
        t0.elapsed().as_secs_f64()
    );
}
