//! Content-addressed blob storage on the local filesystem.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use elrond_application::ports::{BlobError, BlobStore, ContentInspector, StoredBlob};
use elrond_domain::{BlobClass, MediaType, Sha256Checksum, StorageKey};
use sha2::{Digest, Sha256};

/// Blobs stored as files under a data directory.
#[derive(Debug, Clone)]
pub struct FilesystemBlobStore {
    root: PathBuf,
    max_bytes: u64,
}

impl FilesystemBlobStore {
    /// Creates a store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_bytes,
        }
    }

    /// Resolves a key to an absolute path.
    ///
    /// Joins segment by segment rather than joining the key wholesale. `StorageKey`
    /// already guarantees no traversal, so this is belt-and-braces: a future
    /// refactor that loosened the key type would still not be able to escape the
    /// root through this function.
    fn path_for(&self, key: &StorageKey) -> Result<PathBuf, BlobError> {
        let mut path = self.root.clone();
        for segment in key.segments() {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(BlobError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "storage key contained a traversal segment",
                ))));
            }
            path.push(segment);
        }
        Ok(path)
    }

    /// Computes the checksum of a buffer.
    fn checksum_of(bytes: &[u8]) -> Sha256Checksum {
        let digest = Sha256::digest(bytes);
        let mut out = [0_u8; 32];
        out.copy_from_slice(&digest);
        Sha256Checksum::from_bytes(out)
    }

    /// Writes bytes to `path` via a temporary file in the same directory.
    ///
    /// A rename within a directory is atomic, so a crash mid-write leaves either
    /// nothing or the complete file — never a truncated blob that would then be
    /// trusted because its path implies its checksum.
    async fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), BlobError> {
        let parent = path.parent().ok_or_else(|| {
            BlobError::Backend(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "blob path has no parent directory",
            )))
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(BlobError::backend)?;

        // The temporary name includes the process id and a counter so two
        // concurrent writes of the same content cannot collide on it.
        let unique = NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = parent.join(format!(".tmp-{}-{unique}", std::process::id()));

        if let Err(error) = tokio::fs::write(&temporary, bytes).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(BlobError::backend(error));
        }

        match tokio::fs::rename(&temporary, path).await {
            Ok(()) => Ok(()),
            Err(error) => {
                // On Windows a rename onto an existing file fails. Content is
                // immutable and addressed by checksum, so if the destination now
                // exists it already holds exactly these bytes.
                let _ = tokio::fs::remove_file(&temporary).await;
                if tokio::fs::try_exists(path).await.unwrap_or(false) {
                    Ok(())
                } else {
                    Err(BlobError::backend(error))
                }
            }
        }
    }
}

/// Counter used to make temporary filenames unique within a process.
static NEXT_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[async_trait]
impl BlobStore for FilesystemBlobStore {
    async fn put(&self, class: BlobClass, bytes: Vec<u8>) -> Result<StoredBlob, BlobError> {
        let byte_size = bytes.len() as u64;
        if byte_size > self.max_bytes {
            return Err(BlobError::TooLarge {
                limit: self.max_bytes,
            });
        }

        // Hashing before writing is what makes the location derivable from the
        // content rather than from anything the client controls.
        let checksum = Self::checksum_of(&bytes);
        let key = match class {
            BlobClass::Derivative => StorageKey::derive_derivative(checksum),
            other => StorageKey::derive(other, checksum),
        };
        let path = self.path_for(&key)?;

        // Identical content is already identical bytes. Rewriting could only
        // produce the same file or, if interrupted, a worse one.
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(StoredBlob {
                key,
                checksum,
                byte_size,
                deduplicated: true,
            });
        }

        Self::write_atomically(&path, &bytes).await?;

        Ok(StoredBlob {
            key,
            checksum,
            byte_size,
            deduplicated: false,
        })
    }

    async fn get(&self, key: &StorageKey) -> Result<Vec<u8>, BlobError> {
        let path = self.path_for(key)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(BlobError::NotFound {
                    key: key.as_str().to_owned(),
                })
            }
            Err(error) => Err(BlobError::backend(error)),
        }
    }

    async fn exists(&self, key: &StorageKey) -> Result<bool, BlobError> {
        let path = self.path_for(key)?;
        tokio::fs::try_exists(&path)
            .await
            .map_err(BlobError::backend)
    }

    async fn get_verified(&self, key: &StorageKey) -> Result<Vec<u8>, BlobError> {
        let bytes = self.get(key).await?;

        // Derivative keys carry a `.pdf` suffix, so the digest is the last segment
        // with that suffix removed.
        let expected = key
            .as_str()
            .rsplit('/')
            .next()
            .map(|name| name.trim_end_matches(".pdf"))
            .and_then(|digest| digest.parse::<Sha256Checksum>().ok())
            .ok_or_else(|| BlobError::IntegrityFailure {
                key: key.as_str().to_owned(),
            })?;

        if Self::checksum_of(&bytes) != expected {
            return Err(BlobError::IntegrityFailure {
                key: key.as_str().to_owned(),
            });
        }
        Ok(bytes)
    }

    async fn delete(&self, key: &StorageKey) -> Result<(), BlobError> {
        let path = self.path_for(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            // Cleanup has to be idempotent: a retried sweep must not fail because
            // the previous attempt already succeeded.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(BlobError::backend(error)),
        }
    }
}

/// Identifies file types from their leading bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct MagicByteInspector;

impl ContentInspector for MagicByteInspector {
    fn detect(&self, bytes: &[u8]) -> Option<MediaType> {
        // `infer` reads magic numbers. Its answer is a MIME string, which is then
        // narrowed to the closed set Elrond supports; anything outside that set is
        // reported as unrecognized rather than accepted.
        let kind = infer::get(bytes)?;
        let detected = MediaType::from_mime(kind.mime_type());

        // OOXML files are ZIP containers, so `infer` may report a plain archive for
        // a .docx whose internal ordering it does not recognize. Reporting `None`
        // lets the extension decide, rather than storing it as an unsupported ZIP.
        if detected.is_none() && kind.mime_type() == "application/zip" {
            return None;
        }
        detected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store rooted in a unique temporary directory.
    fn store(label: &str) -> FilesystemBlobStore {
        let root = std::env::temp_dir().join(format!("elrond-blobs-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        FilesystemBlobStore::new(root, 16 * 1024 * 1024)
    }

    #[tokio::test]
    async fn content_round_trips() {
        let store = store("round-trip");
        let stored = store
            .put(BlobClass::Original, b"hello elrond".to_vec())
            .await
            .expect("stored");

        assert_eq!(stored.byte_size, 12);
        assert!(!stored.deduplicated);
        assert_eq!(
            store.get(&stored.key).await.expect("read back"),
            b"hello elrond"
        );
    }

    #[tokio::test]
    async fn the_key_is_derived_from_the_content() {
        let store = store("derived-key");
        let stored = store
            .put(BlobClass::Original, b"hello elrond".to_vec())
            .await
            .expect("stored");

        // SHA-256 of "hello elrond", verified against the checksum the store
        // computed while writing.
        assert!(stored.key.as_str().starts_with("originals/"));
        assert!(stored.key.as_str().ends_with(&stored.checksum.to_hex()));
    }

    #[tokio::test]
    async fn identical_content_is_deduplicated() {
        let store = store("dedup");
        let first = store
            .put(BlobClass::Original, b"same bytes".to_vec())
            .await
            .expect("stored");
        let second = store
            .put(BlobClass::Original, b"same bytes".to_vec())
            .await
            .expect("stored");

        assert_eq!(first.key, second.key);
        assert!(!first.deduplicated);
        assert!(
            second.deduplicated,
            "the second write should have been skipped"
        );
    }

    #[tokio::test]
    async fn different_content_lands_in_different_places() {
        let store = store("distinct");
        let a = store
            .put(BlobClass::Original, b"first".to_vec())
            .await
            .expect("stored");
        let b = store
            .put(BlobClass::Original, b"second".to_vec())
            .await
            .expect("stored");
        assert_ne!(a.key, b.key);
    }

    #[tokio::test]
    async fn classes_are_stored_separately() {
        let store = store("classes");
        let original = store
            .put(BlobClass::Original, b"shared".to_vec())
            .await
            .expect("stored");
        let derivative = store
            .put(BlobClass::Derivative, b"shared".to_vec())
            .await
            .expect("stored");

        assert_ne!(original.key, derivative.key);
        assert_eq!(original.checksum, derivative.checksum);
        // Derived keys are generated, so the exact expected value is asserted
        // rather than probing for a suffix.
        assert_eq!(
            derivative.key.as_str(),
            format!(
                "derivatives/{}/{}/{}.pdf",
                derivative.checksum.hex_prefix(2),
                &derivative.checksum.to_hex()[2..4],
                derivative.checksum.to_hex()
            )
        );
    }

    #[tokio::test]
    async fn a_missing_blob_reports_not_found() {
        let store = store("missing");
        let key = StorageKey::derive(
            BlobClass::Original,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .expect("valid digest"),
        );

        assert!(matches!(
            store.get(&key).await.expect_err("absent"),
            BlobError::NotFound { .. }
        ));
        assert!(!store.exists(&key).await.expect("query succeeds"));
    }

    #[tokio::test]
    async fn oversized_content_is_refused_before_it_is_written() {
        let store =
            FilesystemBlobStore::new(std::env::temp_dir().join("elrond-blobs-too-large"), 8);
        let error = store
            .put(BlobClass::Original, vec![0_u8; 9])
            .await
            .expect_err("over the limit");
        assert!(matches!(error, BlobError::TooLarge { limit: 8 }));
    }

    #[tokio::test]
    async fn verification_accepts_intact_content() {
        let store = store("verify-ok");
        let stored = store
            .put(BlobClass::Original, b"intact".to_vec())
            .await
            .expect("stored");
        assert_eq!(
            store.get_verified(&stored.key).await.expect("verified"),
            b"intact"
        );
    }

    #[tokio::test]
    async fn verification_detects_tampered_content() {
        let store = store("verify-tampered");
        let stored = store
            .put(BlobClass::Original, b"intact".to_vec())
            .await
            .expect("stored");

        // Overwrite the file behind the store's back, the way disk corruption or a
        // manual edit would.
        let mut path = store.root.clone();
        for segment in stored.key.segments() {
            path.push(segment);
        }
        std::fs::write(&path, b"tampered").expect("overwritten");

        assert!(matches!(
            store.get_verified(&stored.key).await.expect_err("detected"),
            BlobError::IntegrityFailure { .. }
        ));
        // The plain read still succeeds; verification is the opt-in check.
        assert_eq!(store.get(&stored.key).await.expect("read"), b"tampered");
    }

    #[tokio::test]
    async fn deleting_is_idempotent() {
        let store = store("delete");
        let stored = store
            .put(BlobClass::Original, b"transient".to_vec())
            .await
            .expect("stored");

        store.delete(&stored.key).await.expect("first delete");
        store
            .delete(&stored.key)
            .await
            .expect("deleting an absent blob is not an error");
        assert!(!store.exists(&stored.key).await.expect("query succeeds"));
    }

    #[tokio::test]
    async fn concurrent_writes_of_the_same_content_all_succeed() {
        let store = std::sync::Arc::new(store("concurrent"));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                store
                    .put(BlobClass::Original, b"contended bytes".to_vec())
                    .await
            }));
        }

        let mut keys = Vec::new();
        for handle in handles {
            keys.push(handle.await.expect("task joined").expect("stored").key);
        }
        // One location, no failures, whatever order the writes interleaved in.
        assert!(keys.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            store.get(&keys[0]).await.expect("read back"),
            b"contended bytes"
        );
    }

    #[tokio::test]
    async fn no_temporary_files_are_left_behind() {
        let store = store("no-temps");
        store
            .put(BlobClass::Original, b"clean up".to_vec())
            .await
            .expect("stored");

        let leftovers: Vec<_> = walkdir(&store.root)
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".tmp-"))
            })
            .collect();
        assert!(leftovers.is_empty(), "left temporary files: {leftovers:?}");
    }

    /// Collects every file beneath `root`.
    fn walkdir(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    found.push(path);
                }
            }
        }
        found
    }

    // ------------------------------------------------------------- inspection

    #[test]
    fn a_pdf_is_detected_from_its_signature() {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend_from_slice(&[0_u8; 64]);
        assert_eq!(MagicByteInspector.detect(&pdf), Some(MediaType::Pdf));
    }

    #[test]
    fn a_png_is_detected_from_its_signature() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        assert_eq!(MagicByteInspector.detect(&png), Some(MediaType::Png));
    }

    #[test]
    fn plain_text_has_no_signature_to_detect() {
        // Not a failure: the extension is the only signal for text formats.
        assert_eq!(MagicByteInspector.detect(b"just some words"), None);
    }

    #[test]
    fn an_unsupported_binary_is_not_accepted() {
        // A Windows executable has a recognizable signature that is not on the
        // supported list.
        let mut exe = b"MZ".to_vec();
        exe.extend_from_slice(&[0_u8; 64]);
        assert_eq!(MagicByteInspector.detect(&exe), None);
    }

    #[test]
    fn empty_content_detects_nothing() {
        assert_eq!(MagicByteInspector.detect(b""), None);
    }
}
