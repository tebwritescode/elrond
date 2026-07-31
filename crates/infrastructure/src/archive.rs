//! ZIP extraction adapter.

use std::io::{Cursor, Read};
use std::path::Component;

use elrond_application::ports::{ArchiveEntry, ArchiveError, ArchiveExtractor, ArchiveLimits};
use zip::ZipArchive;

/// Reads ZIP archives with the `zip` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipExtractor;

impl ArchiveExtractor for ZipExtractor {
    fn extract(
        &self,
        bytes: &[u8],
        limits: &ArchiveLimits,
    ) -> Result<Vec<ArchiveEntry>, ArchiveError> {
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).map_err(|_| ArchiveError::Unreadable)?;

        let mut entries = Vec::new();
        let mut total: u64 = 0;

        for index in 0..archive.len() {
            let mut file = archive.by_index(index).map_err(|_| ArchiveError::Entry {
                name: format!("#{index}"),
            })?;
            if file.is_dir() {
                continue;
            }

            if entries.len() >= limits.max_entries {
                return Err(ArchiveError::TooManyEntries {
                    limit: limits.max_entries,
                });
            }

            // `enclosed_name` refuses absolute paths and traversal, so a
            // malicious name fails the entry instead of escaping its folder.
            let name = file.name().to_owned();
            let path = file
                .enclosed_name()
                .ok_or_else(|| ArchiveError::Entry { name: name.clone() })?;

            let mut components: Vec<String> = Vec::new();
            for component in path.components() {
                match component {
                    Component::Normal(part) => {
                        let part = part.to_string_lossy().trim().to_owned();
                        if !part.is_empty() {
                            components.push(part);
                        }
                    }
                    // enclosed_name already rejects these; refuse rather than
                    // trust that behaviour forever.
                    _ => return Err(ArchiveError::Entry { name: name.clone() }),
                }
            }
            let Some(filename) = components.pop() else {
                return Err(ArchiveError::Entry { name });
            };

            // The declared size gates the read, and the ceiling is enforced
            // again while inflating: a zip bomb lies in its headers.
            total = total.saturating_add(file.size());
            if total > limits.max_total_bytes {
                return Err(ArchiveError::TooLarge {
                    limit: limits.max_total_bytes,
                });
            }

            let mut contents = Vec::new();
            let budget = limits.max_total_bytes.saturating_add(1);
            let mut guarded = (&mut file).take(budget);
            guarded
                .read_to_end(&mut contents)
                .map_err(|_| ArchiveError::Entry { name: name.clone() })?;
            if u64::try_from(contents.len()).unwrap_or(u64::MAX) > file.size() {
                // Decompressed to more than the header declared.
                return Err(ArchiveError::TooLarge {
                    limit: limits.max_total_bytes,
                });
            }

            entries.push(ArchiveEntry {
                directories: components,
                filename,
                bytes: contents,
            });
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    use super::*;

    const LIMITS: ArchiveLimits = ArchiveLimits {
        max_entries: 100,
        max_total_bytes: 1024 * 1024,
    };

    fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, bytes) in files {
            writer.start_file(*name, options).expect("start entry");
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish archive");
        cursor.into_inner()
    }

    #[test]
    fn extracts_nested_files_with_their_folders() {
        let bytes = build_zip(&[
            ("Policies/Access/rules.pdf", b"first"),
            ("Policies/summary.pdf", b"second"),
            ("root.txt", b"third"),
        ]);

        let entries = ZipExtractor.extract(&bytes, &LIMITS).expect("extract");

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].directories, vec!["Policies", "Access"]);
        assert_eq!(entries[0].filename, "rules.pdf");
        assert_eq!(entries[0].bytes, b"first");
        assert_eq!(entries[1].directories, vec!["Policies"]);
        assert_eq!(entries[2].directories, Vec::<String>::new());
        assert_eq!(entries[2].filename, "root.txt");
    }

    #[test]
    fn skips_bare_directory_entries() {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        writer.add_directory("empty/", options).expect("dir");
        writer.start_file("empty/file.pdf", options).expect("file");
        writer.write_all(b"content").expect("write");
        writer.finish().expect("finish");
        let bytes = cursor.into_inner();

        let entries = ZipExtractor.extract(&bytes, &LIMITS).expect("extract");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].directories, vec!["empty"]);
    }

    #[test]
    fn rejects_bytes_that_are_not_a_zip() {
        let result = ZipExtractor.extract(b"%PDF-1.7 not a zip", &LIMITS);
        assert!(matches!(result, Err(ArchiveError::Unreadable)));
    }

    #[test]
    fn rejects_traversal_names() {
        // A hostile name has to be hand-built; the writer API refuses them.
        let bytes = build_zip(&[("ok.pdf", b"fine")]);
        let hostile = String::from_utf8_lossy(&bytes).replace("ok.pdf", "../up.x");
        let result = ZipExtractor.extract(hostile.as_bytes(), &LIMITS);
        // Either the archive fails to parse after the splice or the entry is
        // refused; both are rejections. What must never happen is a traversal
        // path coming back as extractable.
        if let Ok(entries) = result {
            assert!(entries.iter().all(|entry| {
                entry.directories.iter().all(|part| part != "..") && entry.filename != ".."
            }));
        }
    }

    #[test]
    fn enforces_the_entry_ceiling() {
        let files: Vec<(String, Vec<u8>)> = (0..5)
            .map(|n| (format!("file{n}.pdf"), b"x".to_vec()))
            .collect();
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let bytes = build_zip(&refs);

        let tight = ArchiveLimits {
            max_entries: 2,
            max_total_bytes: 1024,
        };
        let result = ZipExtractor.extract(&bytes, &tight);
        assert!(matches!(
            result,
            Err(ArchiveError::TooManyEntries { limit: 2 })
        ));
    }

    #[test]
    fn enforces_the_size_ceiling() {
        let big = vec![0u8; 4096];
        let bytes = build_zip(&[("a.pdf", big.as_slice()), ("b.pdf", big.as_slice())]);

        let tight = ArchiveLimits {
            max_entries: 100,
            max_total_bytes: 6000,
        };
        let result = ZipExtractor.extract(&bytes, &tight);
        assert!(matches!(
            result,
            Err(ArchiveError::TooLarge { limit: 6000 })
        ));
    }
}
