// Port of: example_repo/fyrox/Fyrox-master/fyrox-core/src/pool/{handle.rs, mod.rs}
// Stripped of Fyrox reflection, visitor, PayloadContainer, and multi-borrow machinery.
// Only the generational arena semantics are preserved.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

pub const INVALID_GENERATION: u32 = 0;

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Handle<T> {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    _marker: PhantomData<T>,
}

impl<T> Handle<T> {
    pub const NONE: Self = Self {
        index: 0,
        generation: INVALID_GENERATION,
        _marker: PhantomData,
    };

    #[inline]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn is_none(&self) -> bool {
        self.generation == INVALID_GENERATION
    }

    #[inline]
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    #[inline]
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Reinterpret the handle as pointing to a different type.
    /// The index and generation are preserved; only the phantom marker changes.
    /// Used to bridge between opaque handle aliases (e.g. NodeHandle) and Pool<UiNode>.
    #[inline]
    pub fn transmute<U>(&self) -> Handle<U> {
        Handle {
            index: self.index,
            generation: self.generation,
            _marker: PhantomData,
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Handle<T> {}
impl<T> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> fmt::Display for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[idx={} gen={}]", self.index, self.generation)
    }
}

// ---------------------------------------------------------------------------
// PoolRecord (internal)
// ---------------------------------------------------------------------------

struct PoolRecord<T> {
    generation: u32,
    payload: Option<T>,
}

impl<T> Default for PoolRecord<T> {
    fn default() -> Self {
        Self {
            generation: INVALID_GENERATION,
            payload: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pool error
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum PoolError {
    InvalidIndex(u32),
    InvalidGeneration(u32),
    Empty(u32),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIndex(i) => write!(f, "pool: invalid index {i}"),
            Self::InvalidGeneration(g) => write!(f, "pool: stale generation {g}"),
            Self::Empty(i) => write!(f, "pool: empty record at {i}"),
        }
    }
}

impl std::error::Error for PoolError {}

// ---------------------------------------------------------------------------
// Pool
// ---------------------------------------------------------------------------

pub struct Pool<T> {
    records: Vec<PoolRecord<T>>,
    free_stack: Vec<u32>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Pool<T> {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            free_stack: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
            free_stack: Vec::new(),
        }
    }

    // --- mutation ---

    #[must_use]
    pub fn spawn(&mut self, value: T) -> Handle<T> {
        if let Some(free_index) = self.free_stack.pop() {
            let record = &mut self.records[free_index as usize];
            debug_assert!(record.payload.is_none());
            let generation = record.generation + 1;
            record.generation = generation;
            record.payload = Some(value);
            Handle::new(free_index, generation)
        } else {
            let index = self.records.len() as u32;
            let generation = 1;
            self.records.push(PoolRecord {
                generation,
                payload: Some(value),
            });
            Handle::new(index, generation)
        }
    }

    pub fn free(&mut self, handle: Handle<T>) -> T {
        self.try_free(handle).expect("free: invalid handle")
    }

    pub fn try_free(&mut self, handle: Handle<T>) -> Result<T, PoolError> {
        let index = handle.index as usize;
        let record = self
            .records
            .get_mut(index)
            .ok_or(PoolError::InvalidIndex(handle.index))?;
        if record.generation != handle.generation {
            return Err(PoolError::InvalidGeneration(handle.generation));
        }
        let payload = record
            .payload
            .take()
            .ok_or(PoolError::Empty(handle.index))?;
        self.free_stack.push(handle.index);
        Ok(payload)
    }

    // --- borrowing ---

    pub fn borrow(&self, handle: Handle<T>) -> &T {
        self.try_borrow(handle).expect("borrow: invalid handle")
    }

    pub fn borrow_mut(&mut self, handle: Handle<T>) -> &mut T {
        self.try_borrow_mut(handle)
            .expect("borrow_mut: invalid handle")
    }

    pub fn try_borrow(&self, handle: Handle<T>) -> Result<&T, PoolError> {
        let record = self
            .records
            .get(handle.index as usize)
            .ok_or(PoolError::InvalidIndex(handle.index))?;
        if record.generation != handle.generation {
            return Err(PoolError::InvalidGeneration(handle.generation));
        }
        record
            .payload
            .as_ref()
            .ok_or(PoolError::Empty(handle.index))
    }

    pub fn try_borrow_mut(&mut self, handle: Handle<T>) -> Result<&mut T, PoolError> {
        let index = handle.index;
        let record = self
            .records
            .get_mut(index as usize)
            .ok_or(PoolError::InvalidIndex(index))?;
        if record.generation != handle.generation {
            return Err(PoolError::InvalidGeneration(handle.generation));
        }
        record.payload.as_mut().ok_or(PoolError::Empty(index))
    }

    // --- queries ---

    #[inline]
    pub fn is_valid_handle(&self, handle: Handle<T>) -> bool {
        self.records
            .get(handle.index as usize)
            .map(|r| r.payload.is_some() && r.generation == handle.generation)
            .unwrap_or(false)
    }

    #[inline]
    pub fn alive_count(&self) -> u32 {
        self.records.iter().filter(|r| r.payload.is_some()).count() as u32
    }

    #[inline]
    pub fn capacity(&self) -> u32 {
        self.records.len() as u32
    }

    /// Returns the handle that *would* be assigned to the next `spawn` call.
    #[inline]
    pub fn next_free_handle(&self) -> Handle<T> {
        if let Some(&idx) = self.free_stack.last() {
            Handle::new(idx, self.records[idx as usize].generation + 1)
        } else {
            Handle::new(self.records.len() as u32, 1)
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.free_stack.clear();
    }

    // --- iteration ---

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.records.iter().filter_map(|r| r.payload.as_ref())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.records.iter_mut().filter_map(|r| r.payload.as_mut())
    }

    pub fn pair_iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.records.iter().enumerate().filter_map(|(i, r)| {
            r.payload
                .as_ref()
                .map(|p| (Handle::new(i as u32, r.generation), p))
        })
    }

    pub fn pair_iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.records.iter_mut().enumerate().filter_map(|(i, r)| {
            let g = r.generation;
            r.payload
                .as_mut()
                .map(move |p| (Handle::new(i as u32, g), p))
        })
    }

    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut pred: F) {
        for (i, record) in self.records.iter_mut().enumerate() {
            if let Some(payload) = &record.payload {
                if !pred(payload) {
                    record.payload = None;
                    self.free_stack.push(i as u32);
                }
            }
        }
    }
}
