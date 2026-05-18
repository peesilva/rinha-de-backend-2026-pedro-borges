// src/index.rs — loads prebuilt VP-Tree via mmap (zero-copy, low RAM)
use fraud_api::{KnnHeap, VpNode, NULL_NODE, DIMS, euc_sq};
use memmap2::Mmap;
use std::fs::File;

pub struct Index {
    pub n_points: usize,
    pub n_nodes: usize,
    root: u32,
    // Pointers into the mmap — zero-copy, no Vec allocation
    points_ptr: *const [f32; DIMS],
    labels_ptr: *const u8,
    nodes_ptr:  *const VpNode,
    _mmap: Mmap, // keeps the mapping alive
}

// SAFETY: Index is read-only after construction; raw pointers point into
// the mmap which lives as long as the Index itself.
unsafe impl Send for Index {}
unsafe impl Sync for Index {}

impl Index {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let data: *const u8 = mmap.as_ptr();

        let mut cur = 0usize;

        // ── header ──────────────────────────────────────────────────────────
        let n_points = u32::from_le_bytes(unsafe {
            *(data.add(cur) as *const [u8; 4])
        }) as usize;
        cur += 4;

        let n_nodes = u32::from_le_bytes(unsafe {
            *(data.add(cur) as *const [u8; 4])
        }) as usize;
        cur += 4;

        let root = u32::from_le_bytes(unsafe {
            *(data.add(cur) as *const [u8; 4])
        });
        cur += 4;

        // ── points block ─────────────────────────────────────────────────────
        // Layout per point: 14×f32 (56 bytes) + u8 label + 3 pad = 60 bytes
        // We'll keep two separate pointers: one for the float vectors,
        // one for the label bytes (stride = 60, offset = 56 from point start).
        let points_base = cur;
        const POINT_STRIDE: usize = DIMS * 4 + 4; // 56 + 4 = 60 bytes

        // Build a thin index: store pointers to each point's vector + label.
        // This is just pointer arithmetic into the mmap — no data is copied.
        // We store them as raw pointers for O(1) access during KNN.
        let points_ptr = unsafe { data.add(points_base) as *const [f32; DIMS] };

        // Labels are at offset 56 within each 60-byte point record.
        // We'll access them via stride in vp_search.
        let labels_ptr = unsafe { data.add(points_base + DIMS * 4) as *const u8 };

        cur += n_points * POINT_STRIDE;

        // ── nodes block ──────────────────────────────────────────────────────
        // Layout: u32 vp_idx + f32 thresh_sq + u32 left + u32 right = 16 bytes
        let nodes_ptr = unsafe { data.add(cur) as *const VpNode };

        Ok(Index {
            n_points,
            n_nodes,
            root,
            points_ptr,
            labels_ptr,
            nodes_ptr,
            _mmap: mmap,
        })
    }

    #[inline]
    pub fn knn5_fraud_score(&self, query: &[f32; DIMS]) -> f32 {
        let mut heap = KnnHeap::new();
        self.vp_search(query, self.root, &mut heap);
        heap.fraud_score()
    }

    fn vp_search(&self, q: &[f32; DIMS], node_idx: u32, heap: &mut KnnHeap) {
        if node_idx == NULL_NODE { return; }

        // SAFETY: node_idx is always within bounds (built by build-index)
        let nd = unsafe { &*self.nodes_ptr.add(node_idx as usize) };

        // Points are stored with stride POINT_STRIDE (60 bytes).
        // points_ptr has type *const [f32; DIMS] but stride is 60, not 56.
        // We must use byte arithmetic.
        const POINT_STRIDE: usize = DIMS * 4 + 4;
        let vp_idx = nd.vp_idx as usize;

        let vp = unsafe {
            &*((self.points_ptr as *const u8).add(vp_idx * POINT_STRIDE) as *const [f32; DIMS])
        };

        let lbl = unsafe {
            *(self.labels_ptr as *const u8).add(vp_idx * POINT_STRIDE)
        };

        let d_sq = euc_sq(q, vp);
        heap.push(d_sq, lbl);

        let thresh_sq = nd.thresh_sq;
        if d_sq < thresh_sq {
            self.vp_search(q, nd.left, heap);
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