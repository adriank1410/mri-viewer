//! Pamięć podręczna zdekodowanych obrazów, wspólna dla okna i serwera MCP.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::imaging::{Frame, Pixels, load_frame};
use crate::library::Slice;

type Key = (PathBuf, u32);

pub struct FrameCache {
    inner: Mutex<Inner>,
    max_bytes: usize,
}

struct Inner {
    map: HashMap<Key, Arc<Frame>>,
    order: VecDeque<Key>,
    bytes: usize,
}

fn frame_bytes(f: &Frame) -> usize {
    match &f.pixels {
        Pixels::Gray(v) => v.len() * 4,
        Pixels::Rgb(v) => v.len(),
    }
}

impl FrameCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            }),
            max_bytes,
        }
    }

    pub fn peek(&self, slice: &Slice) -> Option<Arc<Frame>> {
        let key = (slice.path.clone(), slice.frame);
        self.inner.lock().unwrap().map.get(&key).cloned()
    }

    pub fn get(&self, slice: &Slice) -> Result<Arc<Frame>> {
        if let Some(f) = self.peek(slice) {
            return Ok(f);
        }
        let frame = Arc::new(load_frame(&slice.path, slice.frame)?);
        let key = (slice.path.clone(), slice.frame);
        let mut inner = self.inner.lock().unwrap();
        if !inner.map.contains_key(&key) {
            inner.bytes += frame_bytes(&frame);
            inner.map.insert(key.clone(), frame.clone());
            inner.order.push_back(key);
            while inner.bytes > self.max_bytes && inner.order.len() > 1 {
                if let Some(old) = inner.order.pop_front() {
                    if let Some(f) = inner.map.remove(&old) {
                        inner.bytes -= frame_bytes(&f);
                    }
                }
            }
        }
        Ok(frame)
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.map.clear();
        inner.order.clear();
        inner.bytes = 0;
    }
}
