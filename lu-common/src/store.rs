use crate::hash::ContentSignature;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("corrupt store entry: {0}")]
    Corrupt(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// On-disk content store using a nested directory structure.
///
/// Layout:
/// ```text
/// .lu-store/
///   ab/                  # first 2 hex chars of the file path hash
///     cdef01.json        # remaining hex chars
/// ```
///
/// Each entry stores signatures keyed by method:
/// ```json
/// {
///   "path": "/absolute/path/to/file",
///   "signatures": {
///     "blake3": "abc123...",
///     "crc32": "deadbeef"
///   }
/// }
/// ```
pub struct ContentStore {
    root: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoreEntry {
    path: String,
    signatures: HashMap<String, String>,
}

impl ContentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default_path() -> PathBuf {
        PathBuf::from(".lu-store")
    }

    /// Record a content signature for a file.
    pub fn record(&self, file_path: &Path, sig: &ContentSignature) -> Result<(), StoreError> {
        let entry_path = self.entry_path(file_path);
        fs::create_dir_all(entry_path.parent().unwrap())?;

        let mut entry = self.load_entry(&entry_path).unwrap_or_else(|| StoreEntry {
            path: file_path.to_string_lossy().into_owned(),
            signatures: HashMap::new(),
        });

        entry
            .signatures
            .insert(sig.method.clone(), sig.value.clone());

        let data = serde_json::to_string_pretty(&entry)?;
        fs::write(&entry_path, data)?;
        Ok(())
    }

    /// Query stored signatures for a file.
    pub fn query(&self, file_path: &Path) -> Result<Option<Vec<ContentSignature>>, StoreError> {
        let entry_path = self.entry_path(file_path);
        match self.load_entry(&entry_path) {
            Some(entry) => {
                let sigs: Vec<ContentSignature> = entry
                    .signatures
                    .into_iter()
                    .map(|(method, value)| ContentSignature { method, value })
                    .collect();
                Ok(Some(sigs))
            }
            None => Ok(None),
        }
    }

    /// Check if a file's current signature differs from stored.
    pub fn differs(
        &self,
        file_path: &Path,
        current: &ContentSignature,
    ) -> Result<bool, StoreError> {
        let entry_path = self.entry_path(file_path);
        match self.load_entry(&entry_path) {
            Some(entry) => match entry.signatures.get(&current.method) {
                Some(stored_value) => Ok(stored_value != &current.value),
                None => Ok(true), // No stored value for this method
            },
            None => Ok(true), // No entry at all
        }
    }

    /// Remove entries for files that no longer exist on disk.
    pub fn gc(&self) -> Result<u64, StoreError> {
        let mut removed = 0u64;
        if !self.root.exists() {
            return Ok(0);
        }
        for dir_entry in fs::read_dir(&self.root)? {
            let dir_entry = dir_entry?;
            if !dir_entry.file_type()?.is_dir() {
                continue;
            }
            for file_entry in fs::read_dir(dir_entry.path())? {
                let file_entry = file_entry?;
                let path = file_entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    if let Some(entry) = self.load_entry(&path) {
                        if !Path::new(&entry.path).exists() {
                            fs::remove_file(&path)?;
                            removed += 1;
                        }
                    }
                }
            }
        }
        Ok(removed)
    }

    fn entry_path(&self, file_path: &Path) -> PathBuf {
        let canonical = file_path.to_string_lossy();
        let hash = simple_path_hash(&canonical);
        let prefix = &hash[..2];
        let rest = &hash[2..];
        self.root.join(prefix).join(format!("{rest}.json"))
    }

    fn load_entry(&self, entry_path: &Path) -> Option<StoreEntry> {
        let data = fs::read_to_string(entry_path).ok()?;
        serde_json::from_str(&data).ok()
    }
}

/// Simple hash of a file path for store bucket placement.
/// Uses a fast non-cryptographic hash for directory distribution.
fn simple_path_hash(s: &str) -> String {
    // FNV-1a 64-bit for fast, well-distributed path hashing
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_store_record_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::new(dir.path().join(".lu-store"));

        let file_path = Path::new("/tmp/test-file.txt");
        let sig = ContentSignature {
            method: "blake3".into(),
            value: "abc123def456".into(),
        };

        store.record(file_path, &sig).unwrap();

        let result = store.query(file_path).unwrap().unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].method, "blake3");
        assert_eq!(result[0].value, "abc123def456");
    }

    #[test]
    fn test_store_differs() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::new(dir.path().join(".lu-store"));
        let file_path = Path::new("/tmp/test-file.txt");

        let sig1 = ContentSignature {
            method: "blake3".into(),
            value: "abc123".into(),
        };
        store.record(file_path, &sig1).unwrap();

        // Same value -> not different
        assert!(!store.differs(file_path, &sig1).unwrap());

        // Different value -> different
        let sig2 = ContentSignature {
            method: "blake3".into(),
            value: "xyz789".into(),
        };
        assert!(store.differs(file_path, &sig2).unwrap());
    }

    #[test]
    fn test_store_query_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::new(dir.path().join(".lu-store"));
        let result = store.query(Path::new("/no/such/file")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_gc() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::new(dir.path().join(".lu-store"));

        // Create a real file, record it, then delete it
        let real_file = dir.path().join("real.txt");
        fs::write(&real_file, "content").unwrap();
        let sig = ContentSignature {
            method: "crc32".into(),
            value: "deadbeef".into(),
        };
        store.record(&real_file, &sig).unwrap();
        fs::remove_file(&real_file).unwrap();

        let removed = store.gc().unwrap();
        assert_eq!(removed, 1);

        // Entry should be gone
        assert!(store.query(&real_file).unwrap().is_none());
    }

    #[test]
    fn test_simple_path_hash_deterministic() {
        let h1 = simple_path_hash("/foo/bar.txt");
        let h2 = simple_path_hash("/foo/bar.txt");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn test_simple_path_hash_distribution() {
        let h1 = simple_path_hash("/foo/bar.txt");
        let h2 = simple_path_hash("/foo/baz.txt");
        assert_ne!(h1, h2);
    }
}
