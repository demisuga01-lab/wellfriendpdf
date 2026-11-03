//! File access for the editor.
//!
//! Natively this is `std::fs`. On `wasm32` there is no file system, so the
//! same path-based requests run against an in-memory store: the browser
//! editor `put`s the input bytes under a path, runs the unchanged request
//! and `take`s the output. Only the operations the editing functions use are
//! provided; anything else is unavailable on `wasm32`, exactly as `std::fs`
//! would be.

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};

    pub use std::fs::{
        File, Metadata, metadata, read, read_dir, remove_file, rename, symlink_metadata,
    };

    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
        path.canonicalize()
    }

    pub fn load_document(
        path: &Path,
        options: lopdf::LoadOptions,
    ) -> lopdf::Result<lopdf::Document> {
        lopdf::Document::load_with_options(path, options)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod memory {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::io::{self, Read, Write};
    use std::path::{Component, Path, PathBuf};

    // Directory scans (the system font search) find nothing on wasm32.
    pub use std::fs::{read_dir, symlink_metadata};

    thread_local! {
        static FILES: RefCell<BTreeMap<PathBuf, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
    }

    fn normal(path: &Path) -> PathBuf {
        let mut out = PathBuf::from("/");
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    out.pop();
                }
                Component::Normal(part) => out.push(part),
                _ => {}
            }
        }
        out
    }

    fn missing() -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, "no such in-memory file")
    }

    /// Store `bytes` at `path`, replacing any earlier file.
    pub fn put(path: &Path, bytes: Vec<u8>) {
        FILES.with(|files| files.borrow_mut().insert(normal(path), bytes));
    }

    /// Remove and return the file at `path`.
    pub fn take(path: &Path) -> Option<Vec<u8>> {
        FILES.with(|files| files.borrow_mut().remove(&normal(path)))
    }

    /// Drop every in-memory file.
    pub fn clear() {
        FILES.with(|files| files.borrow_mut().clear());
    }

    pub struct Metadata {
        len: u64,
        directory: bool,
    }

    impl Metadata {
        pub fn len(&self) -> u64 {
            self.len
        }
        pub fn is_file(&self) -> bool {
            !self.directory
        }
        pub fn is_dir(&self) -> bool {
            self.directory
        }
    }

    pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
        let path = normal(path.as_ref());
        FILES.with(|files| {
            let files = files.borrow();
            if let Some(bytes) = files.get(&path) {
                return Ok(Metadata {
                    len: bytes.len() as u64,
                    directory: false,
                });
            }
            // A directory exists when some file lives below it.
            if files.keys().any(|key| key.starts_with(&path)) || path == Path::new("/") {
                return Ok(Metadata {
                    len: 0,
                    directory: true,
                });
            }
            Err(missing())
        })
    }

    pub fn exists(path: &Path) -> bool {
        metadata(path).is_ok()
    }

    pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
        let path = normal(path);
        if exists(&path) {
            Ok(path)
        } else {
            Err(missing())
        }
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = normal(path.as_ref());
        FILES.with(|files| files.borrow().get(&path).cloned().ok_or_else(missing))
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        take(path.as_ref()).map(|_| ()).ok_or_else(missing)
    }

    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let bytes = take(from.as_ref()).ok_or_else(missing)?;
        put(to.as_ref(), bytes);
        Ok(())
    }

    pub fn load_document(
        path: &Path,
        options: lopdf::LoadOptions,
    ) -> lopdf::Result<lopdf::Document> {
        let bytes = read(path).map_err(lopdf::Error::IO)?;
        lopdf::Document::load_mem_with_options(&bytes, options)
    }

    /// A file being read (whole contents) or written (committed on flush and
    /// on drop).
    pub struct File {
        path: PathBuf,
        data: Vec<u8>,
        position: usize,
        writing: bool,
    }

    #[derive(Default)]
    pub struct OpenOptions {
        create_new: bool,
    }

    impl OpenOptions {
        pub fn write(&mut self, _write: bool) -> &mut Self {
            self
        }
        pub fn create_new(&mut self, create_new: bool) -> &mut Self {
            self.create_new = create_new;
            self
        }
        pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
            let path = normal(path.as_ref());
            if self.create_new && exists(&path) {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file exists"));
            }
            put(&path, Vec::new());
            Ok(File {
                path,
                data: Vec::new(),
                position: 0,
                writing: true,
            })
        }
    }

    impl File {
        pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
            let path = normal(path.as_ref());
            let data = read(&path)?;
            Ok(File {
                path,
                data,
                position: 0,
                writing: false,
            })
        }
        pub fn options() -> OpenOptions {
            OpenOptions::default()
        }
        pub fn create(path: impl AsRef<Path>) -> io::Result<File> {
            OpenOptions::default().open(path)
        }
        fn commit(&self) {
            if self.writing {
                put(&self.path, self.data.clone());
            }
        }
    }

    impl Read for File {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let remaining = &self.data[self.position.min(self.data.len())..];
            let count = remaining.len().min(buffer.len());
            buffer[..count].copy_from_slice(&remaining[..count]);
            self.position += count;
            Ok(count)
        }
    }

    impl Write for File {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.data.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.commit();
            Ok(())
        }
    }

    impl Drop for File {
        fn drop(&mut self) {
            self.commit();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use memory::*;
