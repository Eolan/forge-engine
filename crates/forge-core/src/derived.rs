//! Derived data (#208, D-053): products made from sources on each machine, kept on disk so the
//! next start loads them instead of making them again.
//!
//! [`DerivedCache::get_or_make`] returns a product's stored copy when one is there for its
//! [`Key`], or makes it, stores it and returns it. A key is the digest of everything that
//! decides the product:
//! - its inputs: the parameters that make it, and the keys of the products it is made from, so
//!   a change upstream reaches every product downstream;
//! - its code: a digest of the source files that make it, computed at build time, never a
//!   version bumped by hand. A list too broad only costs a remake; too narrow serves a stale
//!   product.
//!
//! An entry goes when what it depends on changed: writing a product removes that
//! product's entries made by other code, while entries of other inputs (another seed) stay,
//! since switching back is then a load. They stay while used: an entry unused for
//! [`UNUSED_FOR`] (a month) is removed by the cache's first use in a process, and a size cap
//! drops the least recently used beyond it. An entry that cannot be read back whole
//! (truncated, another format, a checksum that fails) is removed and made again, never
//! trusted. [`sweep_unused`] keeps the shader and mesh caches the same way.
//!
//! Products are written and read through [`Stored`]: a plain little-endian layout, field by
//! field ([`crate::stored!`] writes the impl of a struct), with runs of numbers as one block.

use std::fmt::Debug;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use xxhash_rust::xxh3::Xxh3;

#[cfg(not(target_endian = "little"))]
compile_error!("derived data is stored little-endian, as the CPU holds it");

/// The container's layout; a product's own layout is part of its code digest.
const FORMAT: u32 = 1;
const MAGIC: &[u8; 4] = b"FDD1";
/// The file name's extension.
const EXTENSION: &str = "fdd";

/// What a stored product is written into: the file, with a running checksum and length.
pub struct Sink<'a> {
    out: &'a mut dyn Write,
    hasher: Xxh3,
    written: u64,
}

impl Sink<'_> {
    /// Writes raw bytes.
    pub fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.hasher.update(bytes);
        self.written += bytes.len() as u64;
        self.out.write_all(bytes)
    }
}

/// What a stored product is read from: the file, with a running checksum and the bytes left.
pub struct Source<'a> {
    input: &'a mut dyn Read,
    hasher: Xxh3,
    left: u64,
}

impl Source<'_> {
    /// Fills `bytes`, or fails if the payload has fewer left.
    pub fn bytes(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        if (bytes.len() as u64) > self.left {
            return Err(invalid("the stored product ends early"));
        }
        self.input.read_exact(bytes)?;
        self.hasher.update(bytes);
        self.left -= bytes.len() as u64;
        Ok(())
    }

    /// The payload's bytes not read yet.
    #[must_use]
    pub fn left(&self) -> u64 {
        self.left
    }

    /// A count read back, checked against what is left: each item takes at least
    /// `min_item_bytes`, so a damaged count fails here instead of allocating without bound.
    pub fn count(&mut self, min_item_bytes: u64) -> io::Result<usize> {
        let count = u64::take(self)?;
        if count.saturating_mul(min_item_bytes.max(1)) > self.left {
            return Err(invalid("a stored count larger than the payload"));
        }
        usize::try_from(count).map_err(|_| invalid("a stored count beyond this machine"))
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// A type written to and read from derived data. Use [`crate::stored!`] for a struct.
pub trait Stored: Sized {
    /// Writes it.
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()>;
    /// Reads one back.
    fn take(source: &mut Source<'_>) -> io::Result<Self>;
    /// Writes a run of them (the numbers' as one block).
    fn put_all(items: &[Self], sink: &mut Sink<'_>) -> io::Result<()> {
        items.iter().try_for_each(|item| item.put(sink))
    }
    /// Reads a run of `count` back.
    fn take_all(count: usize, source: &mut Source<'_>) -> io::Result<Vec<Self>> {
        (0..count).map(|_| Self::take(source)).collect()
    }
}

macro_rules! stored_numbers {
    ($($t:ty),*) => {$(
        impl Stored for $t {
            fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
                sink.bytes(&self.to_le_bytes())
            }
            fn take(source: &mut Source<'_>) -> io::Result<Self> {
                let mut bytes = [0; size_of::<$t>()];
                source.bytes(&mut bytes)?;
                Ok(<$t>::from_le_bytes(bytes))
            }
            fn put_all(items: &[Self], sink: &mut Sink<'_>) -> io::Result<()> {
                sink.bytes(bytemuck::cast_slice(items))
            }
            fn take_all(count: usize, source: &mut Source<'_>) -> io::Result<Vec<Self>> {
                if (count as u64).saturating_mul(size_of::<$t>() as u64) > source.left() {
                    return Err(invalid("a stored run larger than the payload"));
                }
                let mut items = vec![<$t>::default(); count];
                source.bytes(bytemuck::cast_slice_mut(&mut items))?;
                Ok(items)
            }
        }
    )*};
}

stored_numbers!(u8, u16, u32, u64, u128, i8, i16, i32, i64, f32, f64);

impl Stored for usize {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        (*self as u64).put(sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        usize::try_from(u64::take(source)?)
            .map_err(|_| invalid("a stored size beyond this machine"))
    }
}

impl Stored for bool {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        u8::from(*self).put(sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        match u8::take(source)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("a stored bool neither 0 nor 1")),
        }
    }
    fn put_all(items: &[Self], sink: &mut Sink<'_>) -> io::Result<()> {
        let bytes: Vec<u8> = items.iter().map(|&b| u8::from(b)).collect();
        sink.bytes(&bytes)
    }
    fn take_all(count: usize, source: &mut Source<'_>) -> io::Result<Vec<Self>> {
        u8::take_all(count, source)?
            .into_iter()
            .map(|b| match b {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(invalid("a stored bool neither 0 nor 1")),
            })
            .collect()
    }
}

impl<T: Stored, const N: usize> Stored for [T; N] {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        T::put_all(self, sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        T::take_all(N, source)?
            .try_into()
            .map_err(|_| invalid("a stored array of another length"))
    }
}

impl<T: Stored> Stored for Vec<T> {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.len().put(sink)?;
        T::put_all(self, sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        let count = source.count(1)?;
        T::take_all(count, source)
    }
}

impl<T: Stored> Stored for Arc<[T]> {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.len().put(sink)?;
        T::put_all(self, sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok(Vec::<T>::take(source)?.into())
    }
}

impl<T: Stored> Stored for Arc<T> {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        (**self).put(sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok(Arc::new(T::take(source)?))
    }
}

impl<T: Stored> Stored for Option<T> {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        match self {
            None => false.put(sink),
            Some(value) => {
                true.put(sink)?;
                value.put(sink)
            }
        }
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok(if bool::take(source)? {
            Some(T::take(source)?)
        } else {
            None
        })
    }
}

impl Stored for String {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.len().put(sink)?;
        sink.bytes(self.as_bytes())
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        let count = source.count(1)?;
        String::from_utf8(u8::take_all(count, source)?)
            .map_err(|_| invalid("a stored string that is not UTF-8"))
    }
}

impl<A: Stored, B: Stored> Stored for (A, B) {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.0.put(sink)?;
        self.1.put(sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok((A::take(source)?, B::take(source)?))
    }
}

impl<A: Stored, B: Stored, C: Stored> Stored for (A, B, C) {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.0.put(sink)?;
        self.1.put(sink)?;
        self.2.put(sink)
    }
    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok((A::take(source)?, B::take(source)?, C::take(source)?))
    }
}

/// Writes the [`Stored`] impl of a struct, its fields in the order named. Every field must be
/// named: a field added to the struct and not here fails to compile.
///
/// ```ignore
/// forge_core::stored!(Point { x, y });
/// forge_core::stored!([T: forge_core::derived::Stored] Grid<T> { size, data });
/// ```
#[macro_export]
macro_rules! stored {
    ([$($generics:tt)*] $ty:ty { $($field:ident),* $(,)? }) => {
        impl<$($generics)*> $crate::derived::Stored for $ty {
            fn put(&self, sink: &mut $crate::derived::Sink<'_>) -> ::std::io::Result<()> {
                $( $crate::derived::Stored::put(&self.$field, sink)?; )*
                Ok(())
            }
            fn take(source: &mut $crate::derived::Source<'_>) -> ::std::io::Result<Self> {
                Ok(Self { $( $field: $crate::derived::Stored::take(source)?, )* })
            }
        }
    };
    ($ty:ty { $($field:ident),* $(,)? }) => {
        $crate::stored!([] $ty { $($field),* });
    };
}

/// A product's key: what decides it ([`KeyHasher`]) and the digest of the code that makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The inputs: parameters and upstream keys.
    pub inputs: u64,
    /// The code: a digest of the source files that make the product.
    pub code: u64,
}

impl Key {
    /// One number for the whole key: what a product downstream adds to its inputs.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Xxh3::new();
        hasher.update(&self.inputs.to_le_bytes());
        hasher.update(&self.code.to_le_bytes());
        hasher.digest()
    }
}

/// Builds a [`Key`]'s inputs digest from the parameters and upstream keys.
#[derive(Default)]
pub struct KeyHasher(Xxh3);

impl KeyHasher {
    /// An empty one.
    #[must_use]
    pub fn new() -> Self {
        Self(Xxh3::new())
    }
    /// Adds a parameter by its `Debug` text, which names every field and value.
    #[must_use]
    pub fn debug(mut self, value: &impl Debug) -> Self {
        let text = format!("{value:?}");
        self.0.update(&(text.len() as u64).to_le_bytes());
        self.0.update(text.as_bytes());
        self
    }
    /// Adds a number (an upstream key's [`Key::digest`]).
    #[must_use]
    pub fn number(mut self, value: u64) -> Self {
        self.0.update(&value.to_le_bytes());
        self
    }
    /// The key, with `code`, the product's code digest.
    #[must_use]
    pub fn key(self, code: u64) -> Key {
        Key {
            inputs: self.0.digest(),
            code,
        }
    }
}

/// A product as [`DerivedCache::get_or_make`] returns it.
#[derive(Debug)]
pub struct Derived<T> {
    /// The product.
    pub value: T,
    /// Whether it was loaded rather than made.
    pub from_cache: bool,
    /// How long the load or the make took, with the write.
    pub ms: u128,
}

/// A directory of derived products.
#[derive(Clone, Debug)]
pub struct DerivedCache {
    dir: PathBuf,
    cap_bytes: u64,
}

impl DerivedCache {
    /// The default cap: about five islands' products.
    pub const DEFAULT_CAP_BYTES: u64 = 4 << 30;

    /// A cache in `dir` (made when first written), capped at [`Self::DEFAULT_CAP_BYTES`].
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            cap_bytes: Self::DEFAULT_CAP_BYTES,
        }
    }

    /// The same cache with another size cap.
    #[must_use]
    pub fn with_cap(mut self, cap_bytes: u64) -> Self {
        self.cap_bytes = cap_bytes;
        self
    }

    /// Its directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of `product`'s entry for `key`.
    #[must_use]
    pub fn path(&self, product: &str, key: Key) -> PathBuf {
        self.dir.join(format!(
            "{product}@{:016x}@{:016x}.{EXTENSION}",
            key.code, key.inputs
        ))
    }

    /// `product` for `key`: loaded if stored, else made with `make`, stored and returned. A
    /// failed write is logged and the product returned all the same.
    pub fn get_or_make<T: Stored>(
        &self,
        product: &str,
        key: Key,
        make: impl FnOnce() -> T,
    ) -> Derived<T> {
        let start = Instant::now();
        let path = self.path(product, key);
        sweep_once(&self.dir, &[EXTENSION], UNUSED_FOR);
        match self.load(product, key, &path) {
            Ok(Some(value)) => {
                // The last use, for the sweep and the size cap.
                touch(&path);
                return Derived {
                    value,
                    from_cache: true,
                    ms: start.elapsed().as_millis(),
                };
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(product, %error, path = %path.display(), "a derived entry that cannot be read: removed, made again");
                let _ = fs::remove_file(&path);
            }
        }
        let value = make();
        if let Err(error) = self.store(product, key, &path, &value) {
            tracing::warn!(product, %error, "the derived product could not be stored");
        }
        Derived {
            value,
            from_cache: false,
            ms: start.elapsed().as_millis(),
        }
    }

    /// The stored entry: none if there is no file, an error if there is one it cannot trust.
    fn load<T: Stored>(&self, product: &str, key: Key, path: &Path) -> io::Result<Option<T>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut input = BufReader::with_capacity(1 << 20, file);
        let mut header = Source {
            input: &mut input,
            hasher: Xxh3::new(),
            left: u64::MAX,
        };
        let mut magic = [0; 4];
        header.bytes(&mut magic)?;
        if &magic != MAGIC || u32::take(&mut header)? != FORMAT {
            return Err(invalid("not a derived entry of this format"));
        }
        if String::take(&mut header)? != product
            || u64::take(&mut header)? != key.code
            || u64::take(&mut header)? != key.inputs
        {
            return Err(invalid("a derived entry of another product or key"));
        }
        let length = u64::take(&mut header)?;
        let mut payload = Source {
            input: &mut input,
            hasher: Xxh3::new(),
            left: length,
        };
        let value = T::take(&mut payload)?;
        if payload.left != 0 {
            return Err(invalid("a derived entry longer than its product"));
        }
        let checksum = payload.hasher.digest();
        let mut trailer = [0; 8];
        input.read_exact(&mut trailer)?;
        if u64::from_le_bytes(trailer) != checksum {
            return Err(invalid("a derived entry whose checksum fails"));
        }
        let mut rest = [0; 1];
        if input.read(&mut rest)? != 0 {
            return Err(invalid("bytes after a derived entry's checksum"));
        }
        Ok(Some(value))
    }

    /// Writes `value` to a file of its own, renamed into place, then removes what nothing
    /// can match any more and keeps the cache under its cap.
    fn store<T: Stored>(&self, product: &str, key: Key, path: &Path, value: &T) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let partial = path.with_extension(format!("{EXTENSION}.{}.part", std::process::id()));
        let written = (|| -> io::Result<()> {
            let mut out = BufWriter::with_capacity(1 << 20, File::create(&partial)?);
            // The payload's length leads it: the product is written to a counter first, which
            // costs one more pass over it but no copy in memory.
            let mut counter = Counter(0);
            value.put(&mut Sink {
                out: &mut counter,
                hasher: Xxh3::new(),
                written: 0,
            })?;
            let mut header = Sink {
                out: &mut out,
                hasher: Xxh3::new(),
                written: 0,
            };
            header.bytes(MAGIC)?;
            FORMAT.put(&mut header)?;
            product.to_owned().put(&mut header)?;
            key.code.put(&mut header)?;
            key.inputs.put(&mut header)?;
            counter.0.put(&mut header)?;
            let mut payload = Sink {
                out: &mut out,
                hasher: Xxh3::new(),
                written: 0,
            };
            value.put(&mut payload)?;
            let checksum = payload.hasher.digest();
            out.write_all(&checksum.to_le_bytes())?;
            // No sync: an entry cut short by a crash fails its checksum and is made again.
            out.flush()
        })();
        if let Err(error) = written.and_then(|()| fs::rename(&partial, path)) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        self.remove_other_code(product, key);
        self.cap(path);
        Ok(())
    }

    /// Removes `product`'s entries of other code: no key can match them again.
    fn remove_other_code(&self, product: &str, key: Key) {
        let prefix = format!("{product}@");
        let keep = format!("{product}@{:016x}@", key.code);
        for (path, name) in self.entries() {
            if name.starts_with(&prefix) && !name.starts_with(&keep) {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// Removes the least recently used entries until the cache fits its cap; never `keep`.
    fn cap(&self, keep: &Path) {
        let mut entries: Vec<(SystemTime, u64, PathBuf)> = self
            .entries()
            .into_iter()
            .filter_map(|(path, _)| {
                let meta = fs::metadata(&path).ok()?;
                Some((meta.modified().ok()?, meta.len(), path))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        entries.sort_by_key(|e| e.0);
        for (_, len, path) in entries {
            if total <= self.cap_bytes {
                break;
            }
            if path != keep && fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }

    /// The entries' paths and file names.
    fn entries(&self) -> Vec<(PathBuf, String)> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        dir.filter_map(|e| {
            let path = e.ok()?.path();
            let name = path.file_name()?.to_str()?.to_owned();
            name.ends_with(&format!(".{EXTENSION}"))
                .then_some((path, name))
        })
        .collect()
    }
}

/// How long a cached file may go unused before a sweep removes it (#208): whatever nothing has
/// asked for in a month (a renamed product, an old prop, a removed shader, another seed tried
/// once) goes, so no cache directory grows with what is never used again.
pub const UNUSED_FOR: Duration = Duration::from_secs(30 * 24 * 3600);

/// How old a partial write must be before a sweep takes it for a crashed one and removes it.
const PARTIAL_FOR: Duration = Duration::from_secs(3600);

/// Marks a cached file used now: its modification time is what [`sweep_unused`] and the size
/// cap read as its last use.
pub fn touch(path: &Path) {
    if let Ok(file) = File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Removes, in `dir`, the files ending in one of `extensions` unused for `unused_for`, and the
/// partial writes (a name containing `.part`) older than an hour. Returns how many files and
/// bytes went.
pub fn sweep_unused(dir: &Path, extensions: &[&str], unused_for: Duration) -> (usize, u64) {
    let Ok(listing) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let now = SystemTime::now();
    let (mut files, mut bytes) = (0, 0);
    for entry in listing.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let limit = if name.contains(".part") {
            PARTIAL_FOR
        } else if extensions.iter().any(|e| name.ends_with(&format!(".{e}"))) {
            unused_for
        } else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let age = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        if meta.is_file() && age > limit && fs::remove_file(&path).is_ok() {
            files += 1;
            bytes += meta.len();
        }
    }
    if files > 0 {
        tracing::info!(dir = %dir.display(), files, mib = bytes >> 20, "cache: unused files removed");
    }
    (files, bytes)
}

/// Whether this process has swept `dir` already: each cache sweeps once a process.
fn first_sweep(dir: &Path) -> bool {
    static SWEPT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut swept = SWEPT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if swept.iter().any(|d| d == dir) {
        return false;
    }
    swept.push(dir.to_owned());
    true
}

/// [`sweep_unused`] once a process for `dir`.
pub fn sweep_once(dir: &Path, extensions: &[&str], unused_for: Duration) {
    if first_sweep(dir) {
        sweep_unused(dir, extensions, unused_for);
    }
}

/// A writer that only counts.
struct Counter(u64);

impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Sample {
        id: u32,
        heights: Vec<f32>,
        mask: Vec<bool>,
        name: String,
        corner: Option<[f64; 2]>,
        pairs: Vec<(u8, i64)>,
        shared: Arc<[u16]>,
    }
    crate::stored!(Sample {
        id,
        heights,
        mask,
        name,
        corner,
        pairs,
        shared
    });

    fn sample(id: u32) -> Sample {
        Sample {
            id,
            heights: (0..1000).map(|i| i as f32 * 0.5).collect(),
            mask: vec![true, false, true],
            name: "île".into(),
            corner: Some([1.5, -2.25]),
            pairs: vec![(1, -7), (255, i64::MAX)],
            shared: vec![3, 4, 5].into(),
        }
    }

    fn cache(name: &str) -> DerivedCache {
        let dir = std::env::temp_dir().join(format!("forge-derived-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        DerivedCache::new(dir)
    }

    #[test]
    fn a_product_is_made_once_then_loaded_whole() {
        let cache = cache("load");
        let key = KeyHasher::new().debug(&("seed", 7)).key(1);
        let first = cache.get_or_make("sample", key, || sample(7));
        assert!(!first.from_cache);
        let second = cache.get_or_make("sample", key, || -> Sample { unreachable!("stored") });
        assert!(second.from_cache);
        assert_eq!(second.value, sample(7));
        let _ = fs::remove_dir_all(cache.dir());
    }

    #[test]
    fn new_code_removes_the_old_entries_and_other_inputs_stay() {
        let cache = cache("clean");
        let key = |seed: u64, code: u64| KeyHasher::new().number(seed).key(code);
        cache.get_or_make("sample", key(1, 10), || sample(1));
        cache.get_or_make("sample", key(2, 10), || sample(2));
        cache.get_or_make("other", key(1, 99), || sample(9));
        // Another seed: both kept.
        assert!(cache.path("sample", key(1, 10)).exists());
        assert!(cache.path("sample", key(2, 10)).exists());
        // New code for "sample": its old entries go, "other"'s stay.
        cache.get_or_make("sample", key(1, 11), || sample(1));
        assert!(!cache.path("sample", key(1, 10)).exists());
        assert!(!cache.path("sample", key(2, 10)).exists());
        assert!(cache.path("sample", key(1, 11)).exists());
        assert!(cache.path("other", key(1, 99)).exists());
        let _ = fs::remove_dir_all(cache.dir());
    }

    #[test]
    fn a_damaged_entry_is_made_again() {
        let cache = cache("damaged");
        let key = KeyHasher::new().number(3).key(1);
        cache.get_or_make("sample", key, || sample(3));
        let path = cache.path("sample", key);
        let bytes = fs::read(&path).unwrap();
        // Truncated.
        fs::write(&path, &bytes[..bytes.len() - 20]).unwrap();
        let again = cache.get_or_make("sample", key, || sample(3));
        assert!(!again.from_cache);
        // A flipped bit in the payload: the checksum fails.
        let mut bytes = fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        fs::write(&path, &bytes).unwrap();
        let again = cache.get_or_make("sample", key, || sample(3));
        assert!(!again.from_cache);
        assert_eq!(again.value, sample(3));
        assert!(
            cache
                .get_or_make("sample", key, || -> Sample { unreachable!() })
                .from_cache
        );
        let _ = fs::remove_dir_all(cache.dir());
    }

    #[test]
    fn the_cap_drops_the_least_recently_used() {
        let cache = cache("cap");
        let key = |seed: u64| KeyHasher::new().number(seed).key(1);
        cache.get_or_make("sample", key(1), || sample(1));
        let one = fs::metadata(cache.path("sample", key(1))).unwrap().len();
        let cache = cache.with_cap(2 * one + one / 2);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.get_or_make("sample", key(2), || sample(2));
        std::thread::sleep(std::time::Duration::from_millis(20));
        // 1 used again: 2 is now the least recent.
        assert!(
            cache
                .get_or_make("sample", key(1), || -> Sample { unreachable!() })
                .from_cache
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.get_or_make("sample", key(3), || sample(3));
        assert!(cache.path("sample", key(1)).exists());
        assert!(!cache.path("sample", key(2)).exists());
        assert!(cache.path("sample", key(3)).exists());
        let _ = fs::remove_dir_all(cache.dir());
    }

    #[test]
    fn a_sweep_removes_what_went_unused_and_old_partial_writes() {
        let cache = cache("sweep");
        let key = |seed: u64| KeyHasher::new().number(seed).key(1);
        cache.get_or_make("sample", key(1), || sample(1));
        cache.get_or_make("sample", key(2), || sample(2));
        let partial = cache.dir().join("sample@x.fdd.7.part");
        fs::write(&partial, b"cut short").unwrap();
        let other = cache.dir().join("notes.txt");
        fs::write(&other, b"not the cache's").unwrap();
        // Entry 1 last used two months ago, the partial write two hours ago.
        let ago = |secs: u64| SystemTime::now() - Duration::from_secs(secs);
        let set = |path: &Path, at: SystemTime| {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        };
        set(&cache.path("sample", key(1)), ago(60 * 24 * 3600));
        set(&partial, ago(2 * 3600));
        set(&other, ago(60 * 24 * 3600));
        assert_eq!(sweep_unused(cache.dir(), &[EXTENSION], UNUSED_FOR).0, 2);
        assert!(!cache.path("sample", key(1)).exists());
        assert!(cache.path("sample", key(2)).exists());
        assert!(!partial.exists());
        assert!(other.exists());
        let _ = fs::remove_dir_all(cache.dir());
    }
}
