// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam::queue::ArrayQueue;
use tracing::debug;

pub struct RingBuffer<T> {
    buffer: Arc<ArrayQueue<T>>,
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
    capacity: usize,
    high_watermark: f64,
    low_watermark: f64,
    refill_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    has_been_filled: std::sync::atomic::AtomicBool,
}

impl<T> std::fmt::Debug for RingBuffer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingBuffer")
            .field("capacity", &self.capacity)
            .field("high_watermark", &self.high_watermark)
            .field("low_watermark", &self.low_watermark)
            .field("len", &self.buffer.len())
            .field("is_full", &(self.buffer.len() >= self.capacity))
            .finish_non_exhaustive()
    }
}

impl<T: Clone> RingBuffer<T> {
    pub fn new(capacity: usize, high_watermark: f64, low_watermark: f64) -> Self {
        assert!(capacity > 0, "Capacity must be greater than 0");
        assert!(
            high_watermark > low_watermark,
            "High watermark must be greater than low watermark"
        );

        let buffer = Arc::new(ArrayQueue::new(capacity));

        Self {
            buffer,
            write_idx: AtomicUsize::new(0),
            read_idx: AtomicUsize::new(0),
            capacity,
            high_watermark,
            low_watermark,
            refill_callback: None,
            has_been_filled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_refill_callback(
        capacity: usize,
        high_watermark: f64,
        low_watermark: f64,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let mut buffer = Self::new(capacity, high_watermark, low_watermark);
        buffer.refill_callback = Some(Arc::new(callback));
        buffer
    }

    pub fn push(&self, item: T) -> bool {
        if self.buffer.push(item).is_ok() {
            self.has_been_filled.store(true, Ordering::Relaxed);
            self.check_watermark();
            true
        } else {
            false
        }
    }

    pub fn push_batch(&self, items: &[T]) -> usize {
        let mut pushed = 0;
        for item in items {
            if self.buffer.push(item.clone()).is_ok() {
                pushed += 1;
            } else {
                break;
            }
        }
        if pushed > 0 {
            self.has_been_filled.store(true, Ordering::Relaxed);
            self.check_watermark();
        }
        pushed
    }

    pub fn pop(&self) -> Option<T> {
        let item = self.buffer.pop();
        if item.is_some() {
            self.read_idx.fetch_add(1, Ordering::Relaxed);
        }
        item
    }

    pub fn try_pop(&self) -> Option<T> {
        self.pop()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.capacity
    }

    pub fn fill_level(&self) -> f64 {
        self.buffer.len() as f64 / self.capacity as f64
    }

    pub fn write_idx(&self) -> usize {
        self.write_idx.load(Ordering::Relaxed)
    }

    pub fn read_idx(&self) -> usize {
        self.read_idx.load(Ordering::Relaxed)
    }

    pub fn remaining_capacity(&self) -> usize {
        self.capacity - self.buffer.len()
    }

    fn check_watermark(&self) {
        let fill_level = self.fill_level();

        if fill_level >= self.high_watermark {
            debug!(
                "RingBuffer fill level {}% reached high watermark {}%",
                fill_level * 100.0,
                self.high_watermark * 100.0
            );

            if let Some(ref callback) = self.refill_callback {
                callback();
            }
        }
    }

    pub fn need_refill(&self) -> bool {
        self.has_been_filled.load(Ordering::Relaxed) && self.fill_level() <= self.low_watermark
    }

    pub fn clear(&self) {
        while self.pop().is_some() {}
    }

    pub fn drain(&self) -> Vec<T> {
        let mut items = Vec::with_capacity(self.len());
        while let Some(item) = self.pop() {
            items.push(item);
        }
        items
    }

    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        RingBufferIterator { buffer: self }
    }
}

impl<T: Clone> Clone for RingBuffer<T> {
    fn clone(&self) -> Self {
        Self {
            buffer: Arc::clone(&self.buffer),
            write_idx: AtomicUsize::new(0),
            read_idx: AtomicUsize::new(0),
            capacity: self.capacity,
            high_watermark: self.high_watermark,
            low_watermark: self.low_watermark,
            refill_callback: None,
            has_been_filled: AtomicBool::new(false),
        }
    }
}

pub struct RingBufferIterator<'a, T> {
    buffer: &'a RingBuffer<T>,
}

impl<'a, T: Clone> Iterator for RingBufferIterator<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.buffer.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_basic_operations() {
        let buffer: RingBuffer<u64> = RingBuffer::new(10, 0.8, 0.2);

        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        assert_eq!(buffer.capacity(), 10);
        assert_eq!(buffer.len(), 0);

        assert!(buffer.push(1));
        assert!(buffer.push(2));
        assert_eq!(buffer.len(), 2);

        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), None);
    }

    #[test]
    fn test_ring_buffer_full() {
        let buffer: RingBuffer<u64> = RingBuffer::new(3, 0.8, 0.2);

        assert!(buffer.push(1));
        assert!(buffer.push(2));
        assert!(buffer.push(3));
        assert!(!buffer.push(4));

        assert_eq!(buffer.pop(), Some(1));
        assert!(buffer.push(4));
        assert_eq!(buffer.pop(), Some(2));
    }

    #[test]
    fn test_ring_buffer_batch_operations() {
        let buffer: RingBuffer<u64> = RingBuffer::new(10, 0.8, 0.2);

        let items: Vec<u64> = (1..=5).collect();
        assert_eq!(buffer.push_batch(&items), 5);
        assert_eq!(buffer.len(), 5);

        let collected: Vec<u64> = buffer.drain();
        assert_eq!(collected, vec![1, 2, 3, 4, 5]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_ring_buffer_fill_level() {
        let buffer: RingBuffer<u64> = RingBuffer::new(100, 0.8, 0.2);

        assert_eq!(buffer.fill_level(), 0.0);
        assert!(!buffer.need_refill());

        for i in 0..25 {
            buffer.push(i);
        }
        assert!((buffer.fill_level() - 0.25).abs() < 0.01);
        assert!(!buffer.need_refill());

        for _ in 0..20 {
            let _ = buffer.pop();
        }
        assert!((buffer.fill_level() - 0.05).abs() < 0.01);
        assert!(buffer.need_refill());
    }

    #[test]
    fn test_ring_buffer_clear() {
        let buffer: RingBuffer<u64> = RingBuffer::new(10, 0.8, 0.2);

        for i in 0..5 {
            buffer.push(i);
        }
        assert_eq!(buffer.len(), 5);

        buffer.clear();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_ring_buffer_refill_callback() {
        let callback_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let callback_called_clone = callback_called.clone();

        let buffer: RingBuffer<u64> = RingBuffer::with_refill_callback(100, 0.8, 0.2, move || {
            callback_called_clone.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        for i in 0..80 {
            buffer.push(i);
        }
        assert!(callback_called.load(std::sync::atomic::Ordering::Relaxed));
    }
}
