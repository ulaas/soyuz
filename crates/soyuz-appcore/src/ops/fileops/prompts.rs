//! Queue of jobs waiting for an answer. Several jobs may wait at once; the UI shows one
//! prompt at a time (oldest first) with a "1 of N" count. Dismissing a prompt (Esc /
//! click outside) defers it: the job keeps waiting and its activity card offers
//! "Resolve…" — other jobs' prompts come forward meanwhile.

use std::collections::{HashSet, VecDeque};

use soyuz_jobs::JobId;

#[derive(Clone, Debug, Default)]
pub struct PromptQueue {
    order: VecDeque<JobId>,
    deferred: HashSet<JobId>,
}

impl PromptQueue {
    /// A job raised a prompt (idempotent; keeps its place if already queued).
    pub fn push(&mut self, id: JobId) {
        if !self.order.contains(&id) {
            self.order.push_back(id);
        }
    }

    /// The job's prompt was answered or went away.
    pub fn remove(&mut self, id: JobId) {
        self.order.retain(|j| *j != id);
        self.deferred.remove(&id);
    }

    /// Hide the prompt until [`undefer`](Self::undefer).
    pub fn defer(&mut self, id: JobId) {
        if self.order.contains(&id) {
            self.deferred.insert(id);
        }
    }

    /// Bring a deferred prompt back to the front.
    pub fn undefer(&mut self, id: JobId) {
        if self.deferred.remove(&id) {
            self.order.retain(|j| *j != id);
            self.order.push_front(id);
        }
    }

    pub fn is_deferred(&self, id: JobId) -> bool {
        self.deferred.contains(&id)
    }

    /// The prompt to show now.
    pub fn current(&self) -> Option<JobId> {
        self.order.iter().copied().find(|id| !self.deferred.contains(id))
    }

    /// Prompts waiting to be shown (excluding deferred ones).
    pub fn visible_len(&self) -> usize {
        self.order.iter().filter(|id| !self.deferred.contains(id)).count()
    }

    /// All waiting jobs, deferred included.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Drop jobs that no longer wait (answered elsewhere, cancelled, finished).
    pub fn retain(&mut self, mut waiting: impl FnMut(JobId) -> bool) {
        self.order.retain(|id| waiting(*id));
        let order = &self.order;
        self.deferred.retain(|id| order.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_at_a_time_oldest_first() {
        let mut q = PromptQueue::default();
        assert_eq!(q.current(), None);
        q.push(JobId(3));
        q.push(JobId(1));
        q.push(JobId(3));
        assert_eq!(q.len(), 2, "a job is queued once");
        assert_eq!(q.current(), Some(JobId(3)));
        q.remove(JobId(3));
        assert_eq!(q.current(), Some(JobId(1)));
        assert_eq!(q.visible_len(), 1);
    }

    #[test]
    fn defer_and_resolve_later() {
        let mut q = PromptQueue::default();
        q.push(JobId(1));
        q.push(JobId(2));
        q.defer(JobId(1));
        assert!(q.is_deferred(JobId(1)));
        assert_eq!(q.current(), Some(JobId(2)), "the next job's prompt comes forward");
        assert_eq!(q.visible_len(), 1);
        assert_eq!(q.len(), 2);
        q.undefer(JobId(1));
        assert_eq!(q.current(), Some(JobId(1)), "resolve brings it to the front");
        q.defer(JobId(9));
        assert!(!q.is_deferred(JobId(9)), "unknown jobs cannot be deferred");
    }

    #[test]
    fn retain_drops_jobs_that_stopped_waiting() {
        let mut q = PromptQueue::default();
        for i in 1..=4 {
            q.push(JobId(i));
        }
        q.defer(JobId(2));
        q.retain(|id| id.0 % 2 == 1);
        assert_eq!(q.len(), 2);
        assert!(!q.is_deferred(JobId(2)));
        assert_eq!(q.current(), Some(JobId(1)));
        assert!(!q.is_empty());
    }
}
