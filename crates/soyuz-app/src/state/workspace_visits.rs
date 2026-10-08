//! "Where was I": a per-tab graph of visited places (F7). Back/forward history is a
//! line that forgets the forward branch on every new navigation; the graph keeps it.
//! Each node is a place reached from its parent; navigating somewhere new after going
//! back starts a sibling branch. Every pane of the tab has its own cursor in the graph.

use std::collections::HashMap;

use soyuz_core::Location;

use super::pane::PaneId;

/// How a navigation reached its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisitKind {
    /// A new navigation (opens a branch from the current node).
    New,
    /// Back / forward / a history-graph jump: revisit an existing node when possible.
    Revisit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisitNode {
    pub location: Location,
    pub parent: Option<usize>,
    /// Unix seconds of the first and the latest visit.
    pub first: u64,
    pub last: u64,
    pub count: u32,
}

/// A row of the flattened graph (depth-first, children oldest first).
#[derive(Clone, Debug, PartialEq)]
pub struct VisitRow {
    pub node: usize,
    pub depth: usize,
    pub location: Location,
    pub first: u64,
    pub last: u64,
    pub count: u32,
    /// The pane cursor of the asking pane is here.
    pub current: bool,
    /// This node has more than one child (a branch point).
    pub branches: bool,
}

#[derive(Clone, Debug)]
pub struct VisitGraph {
    nodes: Vec<VisitNode>,
    cursors: HashMap<PaneId, usize>,
    limit: usize,
}

impl Default for VisitGraph {
    fn default() -> Self {
        VisitGraph { nodes: Vec::new(), cursors: HashMap::new(), limit: 200 }
    }
}

impl VisitGraph {
    pub fn with_limit(limit: usize) -> Self {
        VisitGraph { limit: limit.max(2), ..Default::default() }
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(2);
        self.prune();
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node(&self, i: usize) -> Option<&VisitNode> {
        self.nodes.get(i)
    }

    pub fn cursor(&self, pane: PaneId) -> Option<usize> {
        self.cursors.get(&pane).copied()
    }

    /// Record that `pane` now shows `loc`.
    pub fn visit(&mut self, pane: PaneId, loc: &Location, now: u64, kind: VisitKind) {
        let cur = self.cursor(pane);
        if let Some(c) = cur
            && self.nodes[c].location == *loc
        {
            self.touch(c, now);
            return;
        }
        let found = match (kind, cur) {
            (VisitKind::Revisit, Some(c)) => self.find_near(c, loc),
            _ => None,
        }
        .or_else(|| cur.and_then(|c| self.child_with(c, loc)));
        let target = match found {
            Some(n) => {
                self.touch(n, now);
                n
            }
            None => {
                self.nodes.push(VisitNode { location: loc.clone(), parent: cur, first: now, last: now, count: 1 });
                self.nodes.len() - 1
            }
        };
        self.cursors.insert(pane, target);
        self.prune();
    }

    /// A pane went away (closing a tab keeps nothing; this is for layout shrinking).
    pub fn forget_pane(&mut self, pane: PaneId) {
        self.cursors.remove(&pane);
    }

    fn touch(&mut self, n: usize, now: u64) {
        let node = &mut self.nodes[n];
        node.last = node.last.max(now);
        node.count = node.count.saturating_add(1);
    }

    fn child_with(&self, parent: usize, loc: &Location) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent == Some(parent) && n.location == *loc)
            .max_by_key(|(_, n)| n.last)
            .map(|(i, _)| i)
    }

    /// For back/forward: the parent chain first, then children, then anywhere.
    fn find_near(&self, from: usize, loc: &Location) -> Option<usize> {
        let mut p = self.nodes[from].parent;
        while let Some(i) = p {
            if self.nodes[i].location == *loc {
                return Some(i);
            }
            p = self.nodes[i].parent;
        }
        self.child_with(from, loc).or_else(|| {
            self.nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.location == *loc)
                .max_by_key(|(_, n)| n.last)
                .map(|(i, _)| i)
        })
    }

    /// Drop the oldest leaves (never a node a pane is on) until within the limit.
    fn prune(&mut self) {
        while self.nodes.len() > self.limit {
            let has_child = |i: usize| self.nodes.iter().any(|n| n.parent == Some(i));
            let victim = (0..self.nodes.len())
                .filter(|&i| !self.cursors.values().any(|&c| c == i) && !has_child(i))
                .min_by_key(|&i| self.nodes[i].last);
            let Some(v) = victim else { break };
            self.remove(v);
        }
    }

    fn remove(&mut self, v: usize) {
        self.nodes.remove(v);
        let fix = |i: usize| if i > v { i - 1 } else { i };
        for n in &mut self.nodes {
            n.parent = n.parent.map(fix);
        }
        for c in self.cursors.values_mut() {
            *c = fix(*c);
        }
    }

    /// Depth-first rows for the popup; `pane`'s cursor is marked.
    pub fn rows(&self, pane: Option<PaneId>) -> Vec<VisitRow> {
        let current = pane.and_then(|p| self.cursor(p));
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        let mut roots = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            match n.parent {
                Some(p) if p < self.nodes.len() => children[p].push(i),
                _ => roots.push(i),
            }
        }
        let by_first = |v: &mut Vec<usize>| v.sort_by_key(|&i| (self.nodes[i].first, i));
        by_first(&mut roots);
        for c in &mut children {
            by_first(c);
        }
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<(usize, usize)> = roots.iter().rev().map(|&r| (r, 0)).collect();
        while let Some((i, depth)) = stack.pop() {
            let n = &self.nodes[i];
            out.push(VisitRow {
                node: i,
                depth,
                location: n.location.clone(),
                first: n.first,
                last: n.last,
                count: n.count,
                current: current == Some(i),
                branches: children[i].len() > 1,
            });
            for &c in children[i].iter().rev() {
                stack.push((c, depth + 1));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Location {
        Location::Dir(s.into())
    }

    #[test]
    fn branches_at_back_and_forward() {
        let p = PaneId(1);
        let mut g = VisitGraph::default();
        g.visit(p, &d("/a"), 1, VisitKind::New);
        g.visit(p, &d("/a/b"), 2, VisitKind::New);
        g.visit(p, &d("/a/b/c"), 3, VisitKind::New);
        // Back twice, then somewhere new: a branch off /a.
        g.visit(p, &d("/a/b"), 4, VisitKind::Revisit);
        g.visit(p, &d("/a"), 5, VisitKind::Revisit);
        g.visit(p, &d("/x"), 6, VisitKind::New);
        assert_eq!(g.len(), 4, "revisits reuse nodes");
        let rows = g.rows(Some(p));
        let shape: Vec<(String, usize)> = rows.iter().map(|r| (r.location.display(), r.depth)).collect();
        assert_eq!(
            shape,
            [("/a".to_string(), 0), ("/a/b".into(), 1), ("/a/b/c".into(), 2), ("/x".into(), 1)],
            "the forward branch is kept next to the new one"
        );
        assert!(rows[0].branches);
        assert!(rows[3].current);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].last, 5);
        // Navigating to an existing child again reuses it.
        g.visit(p, &d("/a"), 7, VisitKind::Revisit);
        g.visit(p, &d("/a/b"), 8, VisitKind::New);
        assert_eq!(g.len(), 4);
    }

    #[test]
    fn panes_have_their_own_cursor_and_limit_prunes_old_leaves() {
        let (p, q) = (PaneId(1), PaneId(2));
        let mut g = VisitGraph::with_limit(3);
        g.visit(p, &d("/a"), 1, VisitKind::New);
        g.visit(q, &d("/q"), 2, VisitKind::New);
        g.visit(p, &d("/a/1"), 3, VisitKind::New);
        g.visit(p, &d("/a"), 4, VisitKind::Revisit);
        g.visit(p, &d("/a/2"), 5, VisitKind::New);
        assert_eq!(g.len(), 3, "the oldest leaf (/a/1) went");
        let names: Vec<String> = g.rows(None).iter().map(|r| r.location.display()).collect();
        assert_eq!(names, ["/a", "/a/2", "/q"]);
        assert_eq!(g.node(g.cursor(q).unwrap()).unwrap().location, d("/q"), "cursors survive pruning");
        assert_eq!(g.node(g.cursor(p).unwrap()).unwrap().location, d("/a/2"));
    }
}
