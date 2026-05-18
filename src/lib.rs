// src/lib.rs — Shared types + VP-Tree builder (used by build-index)

pub const DIMS: usize = 14;

#[derive(Clone, Copy)]
pub struct Point {
    pub v: [f32; DIMS],
    pub label: u8,
    pub _pad: [u8; 3],
}

#[derive(Clone, Copy)]
pub struct VpNode {
    pub vp_idx: u32,
    pub thresh_sq: f32,
    pub left: u32,   // NULL_NODE = leaf
    pub right: u32,
}

pub const NULL_NODE: u32 = u32::MAX;

// ─── KNN max-heap (k=5) ───────────────────────────────────────────────────────
pub struct KnnHeap {
    pub items: [(f32, u8); 5],
    pub size: usize,
}

impl KnnHeap {
    #[inline] pub fn new() -> Self { KnnHeap { items: [(f32::INFINITY, 0); 5], size: 0 } }

    #[inline(always)]
    pub fn worst(&self) -> f32 {
        if self.size < 5 { f32::INFINITY } else { self.items[0].0 }
    }

    #[inline(always)]
    pub fn push(&mut self, d: f32, label: u8) {
        if self.size < 5 {
            self.items[self.size] = (d, label);
            self.size += 1;
            if self.size == 5 { self.heapify(); }
        } else if d < self.items[0].0 {
            self.items[0] = (d, label);
            self.sift(0);
        }
    }

    fn heapify(&mut self) { for i in (0..2).rev() { self.sift(i); } }
    fn sift(&mut self, mut i: usize) {
        loop {
            let mut m = i;
            let l = 2*i+1; let r = 2*i+2;
            if l < self.size && self.items[l].0 > self.items[m].0 { m = l; }
            if r < self.size && self.items[r].0 > self.items[m].0 { m = r; }
            if m == i { break; }
            self.items.swap(i, m); i = m;
        }
    }

    pub fn fraud_score(&self) -> f32 {
        if self.size == 0 { return 0.5; }
        self.items[..self.size].iter().filter(|(_, l)| *l == 1).count() as f32 / self.size as f32
    }
}

// ─── Distance ─────────────────────────────────────────────────────────────────
#[inline(always)]
pub fn euc_sq(a: &[f32; DIMS], b: &[f32; DIMS]) -> f32 {
    let mut s = 0.0f32;
    for i in 0..DIMS { let d = a[i] - b[i]; s += d * d; }
    s
}

// ─── VP-Tree builder ──────────────────────────────────────────────────────────
// Builds in O(N log N). Uses a work-stack instead of OS recursion.
// Indices slice is sorted in-place; node slots pre-allocated.

pub struct VpTree {
    pub points: Vec<Point>,
    pub nodes: Vec<VpNode>,
    pub root: u32,
}

pub fn build_vp_tree(points: Vec<Point>) -> VpTree {
    let n = points.len();
    // Worst case: n nodes
    let mut nodes: Vec<VpNode> = Vec::with_capacity(n);
    let mut indices: Vec<u32> = (0..n as u32).collect();

    // Shuffle for balance
    let mut rng = 0xcafe_babe_dead_beef_u64;
    for i in (1..n).rev() {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let j = (rng >> 33) as usize % (i + 1);
        indices.swap(i, j);
    }

    // Stack-based build: (slice_start, slice_len, parent_node_idx, is_left)
    // Each entry reserves a node slot upfront, then processes children.
    let root = build_iterative(&points, &mut indices, &mut nodes);

    VpTree { points, nodes, root }
}

fn build_iterative(
    points: &[Point],
    indices: &mut [u32],
    nodes: &mut Vec<VpNode>,
) -> u32 {
    // We use a Vec as a stack of work items.
    // Each item: (start, end, parent_slot, is_left_child)
    // Special sentinel: parent_slot = NULL_NODE means it's the root.

    enum Work {
        Build { start: usize, end: usize, parent_slot: u32, is_left: bool },
    }

    let mut stack = Vec::with_capacity(64);
    stack.push(Work::Build { start: 0, end: indices.len(), parent_slot: NULL_NODE, is_left: true });

    let mut root = NULL_NODE;

    while let Some(w) = stack.pop() {
        let Work::Build { start, end, parent_slot, is_left } = w;

        if start >= end {
            // Empty slice → set parent's child to NULL_NODE (already default)
            if parent_slot != NULL_NODE {
                if is_left { nodes[parent_slot as usize].left = NULL_NODE; }
                else       { nodes[parent_slot as usize].right = NULL_NODE; }
            }
            continue;
        }

        if end - start == 1 {
            let slot = nodes.len() as u32;
            nodes.push(VpNode { vp_idx: indices[start], thresh_sq: 0.0, left: NULL_NODE, right: NULL_NODE });
            set_parent(nodes, parent_slot, is_left, slot);
            if parent_slot == NULL_NODE { root = slot; }
            continue;
        }

        // Choose vantage point = first element
        let vp_raw = indices[start];
        let vp_vec = points[vp_raw as usize].v;
        let rest = &mut indices[start+1..end];

        // Compute distances from VP to each point in rest
        // Partition by median (nth_element)
        let mid = rest.len() / 2;

        // nth_element equivalent: select_nth_unstable_by_key on distances
        // We need to sort by distance but only partition, not full sort.
        // We'll compute distances inline and use select_nth_unstable.

        // Step 1: compute distances into a temp vec (unavoidable)
        let mut dist_idx: Vec<(f32, u32)> = rest.iter()
            .map(|&i| (euc_sq(&vp_vec, &points[i as usize].v), i))
            .collect();

        dist_idx.select_nth_unstable_by(mid, |a, b| {
            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
        });

        let thresh_sq = dist_idx[mid].0;

        // Write sorted indices back
        for (i, (_, idx)) in dist_idx.iter().enumerate() {
            rest[i] = *idx;
        }
        // rest[0..mid] = left (inside), rest[mid..] = right (outside)

        // Reserve node slot
        let slot = nodes.len() as u32;
        nodes.push(VpNode { vp_idx: vp_raw, thresh_sq, left: NULL_NODE, right: NULL_NODE });
        set_parent(nodes, parent_slot, is_left, slot);
        if parent_slot == NULL_NODE { root = slot; }

        let left_start  = start + 1;
        let left_end    = start + 1 + mid;
        let right_start = left_end;
        let right_end   = end;

        // Push right first (processed after left, giving DFS left-first order)
        stack.push(Work::Build { start: right_start, end: right_end, parent_slot: slot, is_left: false });
        stack.push(Work::Build { start: left_start,  end: left_end,  parent_slot: slot, is_left: true  });
    }

    root
}

fn set_parent(nodes: &mut [VpNode], parent_slot: u32, is_left: bool, child: u32) {
    if parent_slot == NULL_NODE { return; }
    if is_left { nodes[parent_slot as usize].left = child; }
    else       { nodes[parent_slot as usize].right = child; }
}
