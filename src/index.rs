// src/index.rs — loads prebuilt VP-Tree from binary file and runs KNN queries

use fraud_api::{KnnHeap, VpNode, NULL_NODE, DIMS, euc_sq};
use std::fs;

pub struct Index {
    pub n_points: usize,
    pub n_nodes: usize,
    root: u32,
    // points stored as flat f32 array: [14 floats, 1 label byte (as f32 placeholder), ...]
    // We store label separately to avoid struct padding math
    points: Vec<[f32; DIMS]>,
    labels: Vec<u8>,
    nodes: Vec<VpNode>,
}

impl Index {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let data = fs::read(path)?;
        let mut cur = 0usize;

        macro_rules! u32le {
            () => {{ let v = u32::from_le_bytes(data[cur..cur+4].try_into()?); cur += 4; v }};
        }
        macro_rules! f32le {
            () => {{ let v = f32::from_le_bytes(data[cur..cur+4].try_into()?); cur += 4; v }};
        }

        let n_points = u32le!() as usize;
        let n_nodes  = u32le!() as usize;
        let root     = u32le!();

        let mut points = Vec::with_capacity(n_points);
        let mut labels = Vec::with_capacity(n_points);

        for _ in 0..n_points {
            let mut v = [0.0f32; DIMS];
            for x in v.iter_mut() { *x = f32le!(); }
            let label = data[cur];
            cur += 4; // label + 3 pad
            points.push(v);
            labels.push(label);
        }

        let mut nodes = Vec::with_capacity(n_nodes);
        for _ in 0..n_nodes {
            let vp_idx   = u32le!();
            let thresh_sq = f32le!();
            let left     = u32le!();
            let right    = u32le!();
            nodes.push(VpNode { vp_idx, thresh_sq, left, right });
        }

        Ok(Index { n_points, n_nodes, root, points, labels, nodes })
    }

    #[inline]
    pub fn knn5_fraud_score(&self, query: &[f32; DIMS]) -> f32 {
        let mut heap = KnnHeap::new();
        self.vp_search(query, self.root, &mut heap);
        heap.fraud_score()
    }

    fn vp_search(&self, q: &[f32; DIMS], node_idx: u32, heap: &mut KnnHeap) {
        if node_idx == NULL_NODE { return; }

        let nd = unsafe { self.nodes.get_unchecked(node_idx as usize) };
        let vp = unsafe { self.points.get_unchecked(nd.vp_idx as usize) };
        let lbl = unsafe { *self.labels.get_unchecked(nd.vp_idx as usize) };

        let d_sq = euc_sq(q, vp);
        heap.push(d_sq, lbl);

        let thresh_sq = nd.thresh_sq;

        if d_sq < thresh_sq {
            self.vp_search(q, nd.left, heap);
            // Can right subtree contain closer points?
            let gap = thresh_sq.sqrt() - d_sq.sqrt();
            if gap * gap < heap.worst() {
                self.vp_search(q, nd.right, heap);
            }
        } else {
            self.vp_search(q, nd.right, heap);
            let gap = d_sq.sqrt() - thresh_sq.sqrt();
            if gap * gap < heap.worst() {
                self.vp_search(q, nd.left, heap);
            }
        }
    }
}
