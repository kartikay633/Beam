//! The shared chunk queue that data streams take work from. Each stream
//! pulls the next chunk as soon as it is free (work stealing), so fast
//! streams naturally do more of the work.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ChunkRef {
    /// Index into the transfer's file list (not the wire `file_id`).
    pub file: usize,
    pub chunk: u64,
}

pub(crate) struct ChunkQueue {
    chunks: Mutex<VecDeque<ChunkRef>>,
    notify: Notify,
    closed: AtomicBool,
}

impl ChunkQueue {
    pub(crate) fn new(chunks: VecDeque<ChunkRef>) -> ChunkQueue {
        ChunkQueue {
            chunks: Mutex::new(chunks),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }

    /// The next chunk to send. Waits while the queue is empty (a failed chunk
    /// may still be put back) and returns `None` once the queue is closed.
    pub(crate) async fn next(&self) -> Option<ChunkRef> {
        loop {
            // Register interest before checking, so a push between the check
            // and the wait isn't missed.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.closed.load(Ordering::Acquire) {
                return None;
            }
            if let Some(chunk) = self.lock().pop_front() {
                return Some(chunk);
            }
            notified.await;
        }
    }

    /// Puts a chunk back at the front, so it is sent next.
    pub(crate) fn push_front(&self, chunk: ChunkRef) {
        self.lock().push_front(chunk);
        self.notify.notify_one();
    }

    /// Wakes every waiting stream and makes them stop.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<ChunkRef>> {
        self.chunks.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn chunk(file: usize, chunk: u64) -> ChunkRef {
        ChunkRef { file, chunk }
    }

    #[tokio::test]
    async fn chunks_come_out_in_order_and_retries_jump_the_line() {
        let queue = ChunkQueue::new(VecDeque::from([chunk(0, 0), chunk(0, 1), chunk(1, 0)]));
        assert_eq!(queue.next().await, Some(chunk(0, 0)));
        queue.push_front(chunk(0, 0));
        assert_eq!(queue.next().await, Some(chunk(0, 0)));
        assert_eq!(queue.next().await, Some(chunk(0, 1)));
        assert_eq!(queue.next().await, Some(chunk(1, 0)));
        assert_eq!(queue.len(), 0);
    }

    #[tokio::test]
    async fn waiting_streams_wake_for_retries_and_for_close() {
        let queue = Arc::new(ChunkQueue::new(VecDeque::new()));
        let waiter = {
            let queue = queue.clone();
            tokio::spawn(async move { queue.next().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        queue.push_front(chunk(3, 7));
        assert_eq!(waiter.await.unwrap(), Some(chunk(3, 7)));

        let waiters: Vec<_> = (0..4)
            .map(|_| {
                let queue = queue.clone();
                tokio::spawn(async move { queue.next().await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(20)).await;
        queue.close();
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn many_streams_share_the_work_without_duplicates() {
        let all: VecDeque<_> = (0..1_000).map(|i| chunk(0, i)).collect();
        let queue = Arc::new(ChunkQueue::new(all));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let queue = queue.clone();
                tokio::spawn(async move {
                    let mut got = Vec::new();
                    while let Some(c) = queue.next().await {
                        got.push(c.chunk);
                        if queue.len() == 0 {
                            queue.close();
                        }
                        tokio::task::yield_now().await;
                    }
                    got
                })
            })
            .collect();
        let mut seen = Vec::new();
        for worker in workers {
            seen.extend(worker.await.unwrap());
        }
        seen.sort();
        assert_eq!(seen, (0..1_000).collect::<Vec<_>>());
    }
}
