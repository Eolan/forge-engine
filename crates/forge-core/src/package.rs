//! Packages (D-053's step 5, #214): a cooked world as a plain directory, so that a game loads
//! its content without making it.
//!
//! A package is `manifest.toml` and its entries' files beside it. The manifest lists each entry
//! with these fields:
//! - its id, unique in the package (`cell/4/12/-3`, `mesh/lamp_post_a`);
//! - its kind (`cell`, `mesh`, `texture`) and its format version, which a reader checks against
//!   the one it reads;
//! - its file, relative to the directory;
//! - its size and the xxh3 digest of the whole file, checked on every read.
//!
//! A [`Stored`] entry's file is a header (magic, kind, format) and the payload packed as the
//! derived-data cache packs its entries (LZ4 frames and their checksum, [`crate::derived`]).
//! A file entry (a texture's DDS, say) is copied as it is. A wrong format or digest is refused
//! with an error that names the entry.
//!
//! It is unzipped for now (the owner's call). A container file, or a zip, would be this same
//! manifest over another file system, a choice for a release.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::derived::{Stored, decode_body, encode_body};

/// The manifest's own format; a package of another is refused.
pub const FORMAT: u32 = 1;
/// The manifest's file name.
pub const MANIFEST: &str = "manifest.toml";
/// A [`Stored`] entry's file starts with this.
const MAGIC: &[u8; 4] = b"FPK1";

/// The manifest as written.
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    package: Header,
    #[serde(default, rename = "entry")]
    entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    name: String,
    format: u32,
}

/// An entry of a package, as its manifest lists it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Its id, unique in the package.
    pub id: String,
    /// What it holds.
    pub kind: String,
    /// The version of its kind's layout it was written in.
    pub format: u32,
    /// Its file, relative to the package's directory, with `/` between folders.
    pub file: String,
    /// Its file's size in bytes.
    pub bytes: u64,
    /// The xxh3 digest of its whole file, in hex.
    pub digest: String,
}

/// An error that names the entry.
fn invalid(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// An id as a file name: what isn't a letter, a digit, `-` or `.` becomes `_`.
fn file_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Writes a package: its entries as they come, its manifest at [`PackageWriter::finish`].
pub struct PackageWriter {
    dir: PathBuf,
    name: String,
    entries: Vec<Entry>,
}

impl PackageWriter {
    /// A package named `name` into `dir`, made if missing. Files of an earlier package there are
    /// overwritten as entries of the same ids are written; the manifest lists only this one's.
    pub fn create(dir: impl Into<PathBuf>, name: &str) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            name: name.to_owned(),
            entries: Vec::new(),
        })
    }

    /// Writes `value` as entry `id` of `kind` in layout `format`; its file's size.
    pub fn put<T: Stored>(
        &mut self,
        id: &str,
        kind: &str,
        format: u32,
        value: &T,
    ) -> io::Result<u64> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(kind.len() as u32).to_le_bytes());
        bytes.extend_from_slice(kind.as_bytes());
        bytes.extend_from_slice(&format.to_le_bytes());
        bytes.extend_from_slice(&encode_body(value)?);
        self.add(id, kind, format, "fpk", &bytes)
    }

    /// Copies the file at `from` as entry `id` of `kind` in layout `format`; its size.
    pub fn put_file(&mut self, id: &str, kind: &str, format: u32, from: &Path) -> io::Result<u64> {
        let bytes = fs::read(from)?;
        let extension = from
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_owned();
        self.add(id, kind, format, &extension, &bytes)
    }

    fn add(
        &mut self,
        id: &str,
        kind: &str,
        format: u32,
        extension: &str,
        bytes: &[u8],
    ) -> io::Result<u64> {
        if self.entries.iter().any(|e| e.id == id) {
            return Err(invalid(format!("package entry {id} written twice")));
        }
        let file = format!("{}/{}.{extension}", file_name(kind), file_name(id));
        let path = self.dir.join(&file);
        fs::create_dir_all(path.parent().unwrap_or(&self.dir))?;
        fs::write(&path, bytes)?;
        self.entries.push(Entry {
            id: id.to_owned(),
            kind: kind.to_owned(),
            format,
            file,
            bytes: bytes.len() as u64,
            digest: format!("{:016x}", xxh3_64(bytes)),
        });
        Ok(bytes.len() as u64)
    }

    /// Writes the manifest, its entries sorted by id, renamed into place; the package's size.
    pub fn finish(mut self) -> io::Result<u64> {
        self.entries.sort_by(|a, b| a.id.cmp(&b.id));
        let total = self.entries.iter().map(|e| e.bytes).sum();
        let manifest = Manifest {
            package: Header {
                name: self.name,
                format: FORMAT,
            },
            entries: self.entries,
        };
        let text =
            toml::to_string(&manifest).map_err(|e| invalid(format!("package manifest: {e}")))?;
        let part = self
            .dir
            .join(format!("{MANIFEST}.{}.part", std::process::id()));
        fs::write(&part, text)?;
        fs::rename(&part, self.dir.join(MANIFEST))?;
        Ok(total)
    }
}

/// An opened package: its manifest read and checked, its entries read on demand.
pub struct Package {
    dir: PathBuf,
    name: String,
    entries: HashMap<String, Entry>,
}

impl Package {
    /// Opens the package in `dir`. Refused when its manifest is missing, unreadable or of
    /// another format.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        let text = fs::read_to_string(dir.join(MANIFEST)).map_err(|e| {
            io::Error::new(e.kind(), format!("{}: {e}", dir.join(MANIFEST).display()))
        })?;
        let manifest: Manifest = toml::from_str(&text)
            .map_err(|e| invalid(format!("{}: {e}", dir.join(MANIFEST).display())))?;
        if manifest.package.format != FORMAT {
            return Err(invalid(format!(
                "package {} is of format {}; this engine reads {FORMAT}",
                manifest.package.name, manifest.package.format
            )));
        }
        let entries = manifest
            .entries
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();
        Ok(Self {
            dir,
            name: manifest.package.name,
            entries,
        })
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entry `id`, as listed.
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Its entries of `kind`, in no order.
    pub fn entries_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.values().filter(move |e| e.kind == kind)
    }

    /// Entry `id`'s file read whole, refused unless it is of `kind` in layout `format` and its
    /// size and digest are the manifest's.
    pub fn read_bytes(&self, id: &str, kind: &str, format: u32) -> io::Result<Vec<u8>> {
        let entry = self
            .entries
            .get(id)
            .ok_or_else(|| invalid(format!("package {} has no entry {id}", self.name)))?;
        if entry.kind != kind || entry.format != format {
            return Err(invalid(format!(
                "package {} entry {id} is a {} of format {}; asked for a {kind} of format {format}",
                self.name, entry.kind, entry.format
            )));
        }
        let bytes = fs::read(self.dir.join(&entry.file))?;
        let digest = format!("{:016x}", xxh3_64(&bytes));
        if bytes.len() as u64 != entry.bytes || digest != entry.digest {
            return Err(invalid(format!(
                "package {} entry {id}: its file ({} bytes, digest {digest}) isn't the manifest's ({} bytes, {})",
                self.name,
                bytes.len(),
                entry.bytes,
                entry.digest
            )));
        }
        Ok(bytes)
    }

    /// Entry `id` read back as a `T`, checked as [`Self::read_bytes`] checks it.
    pub fn read<T: Stored>(&self, id: &str, kind: &str, format: u32) -> io::Result<T> {
        let bytes = self.read_bytes(id, kind, format)?;
        let bad = || {
            invalid(format!(
                "package {} entry {id}: not a stored entry",
                self.name
            ))
        };
        if bytes.len() < 12 || &bytes[..4] != MAGIC {
            return Err(bad());
        }
        let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or_default()) as usize;
        let body = bytes.get(8 + n + 4..).ok_or_else(bad)?;
        decode_body(body).map_err(|e| invalid(format!("package {} entry {id}: {e}", self.name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("forge-package-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// Entries written come back as they were, through a manifest read anew.
    #[test]
    fn entries_come_back_as_written() {
        let dir = temp("round");
        let mut w = PackageWriter::create(&dir, "test").unwrap();
        let cell: Vec<u32> = (0..10_000).collect();
        w.put("cell/4/12/-3", "cell", 2, &cell).unwrap();
        w.put("name", "text", 1, &"a district".to_owned()).unwrap();
        let raw = dir.join("raw.dds");
        fs::write(&raw, b"DDS not really").unwrap();
        w.put_file("texture/wall", "texture", 1, &raw).unwrap();
        assert!(w.finish().unwrap() > 0);
        let p = Package::open(&dir).unwrap();
        assert_eq!(p.name(), "test");
        assert_eq!(p.read::<Vec<u32>>("cell/4/12/-3", "cell", 2).unwrap(), cell);
        assert_eq!(p.read::<String>("name", "text", 1).unwrap(), "a district");
        assert_eq!(
            p.read_bytes("texture/wall", "texture", 1).unwrap(),
            b"DDS not really"
        );
        assert_eq!(p.entries_of("cell").count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A changed file, another format or kind, a missing entry and a manifest of another format
    /// are refused, naming what is wrong.
    #[test]
    fn what_doesnt_match_is_refused() {
        let dir = temp("refuse");
        let mut w = PackageWriter::create(&dir, "test").unwrap();
        w.put("a", "cell", 1, &vec![1u32, 2, 3]).unwrap();
        w.put("b", "cell", 1, &vec![4u32]).unwrap();
        assert!(
            w.put("b", "cell", 1, &vec![5u32]).is_err(),
            "an id written twice"
        );
        w.finish().unwrap();
        let p = Package::open(&dir).unwrap();
        let wrong = |r: io::Result<Vec<u32>>, what: &str| {
            let e = r.unwrap_err().to_string();
            assert!(e.contains(what), "{e}");
        };
        wrong(p.read("a", "cell", 2), "format 1");
        wrong(p.read("a", "mesh", 1), "a cell");
        wrong(p.read("c", "cell", 1), "no entry c");
        // One byte changed in b's file.
        let file = dir.join(&p.entry("b").unwrap().file);
        let mut bytes = fs::read(&file).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&file, bytes).unwrap();
        wrong(p.read("b", "cell", 1), "isn't the manifest's");
        // A manifest of another format.
        let text = fs::read_to_string(dir.join(MANIFEST)).unwrap();
        // The package's own format comes first in the manifest.
        fs::write(
            dir.join(MANIFEST),
            text.replacen("format = 1", "format = 9", 1),
        )
        .unwrap();
        let e = Package::open(&dir).err().expect("refused").to_string();
        assert!(e.contains("format 9"), "{e}");
        let _ = fs::remove_dir_all(&dir);
    }
}
