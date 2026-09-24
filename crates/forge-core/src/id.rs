//! Generational handles: stable, cheap, typed references into engine tables.
//!
//! A [`Handle`] is a 32-bit slot index plus a 32-bit generation. Freeing a slot bumps its
//! generation, so a stale handle can never alias a newer occupant (the Bitsquid / kyren
//! pattern, see `docs/RESEARCH.md` §6).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

/// Typed generational handle. `T` is a marker only; the handle carries no data.
pub struct Handle<T> {
    index: u32,
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// A handle that never refers to a live slot.
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: 0,
        _marker: PhantomData,
    };

    /// Builds a handle from its parts.
    pub const fn from_parts(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            _marker: PhantomData,
        }
    }

    /// Slot index.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Generation of the slot when the handle was issued.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// Whether this is [`Self::INVALID`].
    pub const fn is_invalid(self) -> bool {
        self.index == u32::MAX
    }

    /// Packs the handle into 64 bits (network, GPU buffers).
    pub const fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.index as u64
    }

    /// Unpacks a handle from [`Self::to_bits`].
    pub const fn from_bits(bits: u64) -> Self {
        Self::from_parts(bits as u32, (bits >> 32) as u32)
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
        self.to_bits().hash(state);
    }
}
impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}v{})", self.index, self.generation)
    }
}
impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self::INVALID
    }
}

/// Issues and recycles handles. Storage of the values themselves lives elsewhere (tables).
pub struct HandleAllocator<T> {
    generations: Vec<u32>,
    free: Vec<u32>,
    live: usize,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Default for HandleAllocator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> HandleAllocator<T> {
    /// An empty allocator.
    pub const fn new() -> Self {
        Self {
            generations: Vec::new(),
            free: Vec::new(),
            live: 0,
            _marker: PhantomData,
        }
    }

    /// Issues a fresh handle, reusing a freed slot when one exists.
    pub fn allocate(&mut self) -> Handle<T> {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            return Handle::from_parts(index, self.generations[index as usize]);
        }
        let index = u32::try_from(self.generations.len()).expect("handle space exhausted");
        self.generations.push(1);
        Handle::from_parts(index, 1)
    }

    /// Releases a handle. Returns `false` if it was stale or already freed.
    pub fn free(&mut self, handle: Handle<T>) -> bool {
        if !self.is_live(handle) {
            return false;
        }
        let slot = &mut self.generations[handle.index as usize];
        // Generation 0 is reserved for "never issued", so wrap to 1.
        *slot = slot.wrapping_add(1).max(1);
        self.free.push(handle.index);
        self.live -= 1;
        true
    }

    /// Whether the handle refers to a slot that is currently allocated.
    pub fn is_live(&self, handle: Handle<T>) -> bool {
        self.generations
            .get(handle.index as usize)
            .is_some_and(|&g| g == handle.generation)
            && !self.free.contains(&handle.index)
    }

    /// Number of live handles.
    pub const fn live_count(&self) -> usize {
        self.live
    }

    /// Number of slots ever issued (the size a dense table needs).
    pub fn capacity(&self) -> usize {
        self.generations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Thing;

    #[test]
    fn freed_handles_go_stale_and_slots_are_reused() {
        let mut alloc = HandleAllocator::<Thing>::new();
        let a = alloc.allocate();
        assert!(alloc.is_live(a));
        assert!(alloc.free(a));
        assert!(!alloc.is_live(a));
        assert!(!alloc.free(a));
        let b = alloc.allocate();
        assert_eq!(a.index(), b.index());
        assert_ne!(a.generation(), b.generation());
        assert!(alloc.is_live(b));
        assert_eq!(Handle::<Thing>::from_bits(b.to_bits()), b);
    }
}
