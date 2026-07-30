//! Supported file types.
//!
//! Modelled as a closed enumeration rather than an open MIME string. Elrond has
//! to know, for every stored file, whether it is already a PDF, whether a PDF
//! derivative must be generated, and whether text can be extracted from it. An
//! arbitrary MIME string cannot answer those questions, and accepting one would
//! mean storing files the rest of the system does not know how to handle.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// What Elrond has to do with a file once it is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// Already the canonical distribution format.
    Pdf,
    /// Needs rendering into a PDF page.
    Image,
    /// Needs conversion by an office-document pipeline.
    Office,
    /// Plain text that can be typeset into a PDF directly.
    Text,
}

/// A file type Elrond accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    /// Portable Document Format.
    Pdf,

    /// PNG image.
    Png,
    /// JPEG image.
    Jpeg,
    /// TIFF image, common for scanned material.
    Tiff,
    /// WebP image.
    Webp,
    /// GIF image.
    Gif,

    /// Word document, OOXML.
    Docx,
    /// Excel workbook, OOXML.
    Xlsx,
    /// PowerPoint presentation, OOXML.
    Pptx,
    /// Word document, legacy binary.
    Doc,
    /// Excel workbook, legacy binary.
    Xls,
    /// PowerPoint presentation, legacy binary.
    Ppt,
    /// OpenDocument text.
    Odt,
    /// OpenDocument spreadsheet.
    Ods,
    /// OpenDocument presentation.
    Odp,
    /// Rich Text Format.
    Rtf,

    /// Plain text.
    PlainText,
    /// Markdown.
    Markdown,
    /// Comma-separated values.
    Csv,
}

impl MediaType {
    /// Every accepted type.
    pub const ALL: [Self; 19] = [
        Self::Pdf,
        Self::Png,
        Self::Jpeg,
        Self::Tiff,
        Self::Webp,
        Self::Gif,
        Self::Docx,
        Self::Xlsx,
        Self::Pptx,
        Self::Doc,
        Self::Xls,
        Self::Ppt,
        Self::Odt,
        Self::Ods,
        Self::Odp,
        Self::Rtf,
        Self::PlainText,
        Self::Markdown,
        Self::Csv,
    ];

    /// Canonical MIME type, and the value stored in the database.
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Doc => "application/msword",
            Self::Xls => "application/vnd.ms-excel",
            Self::Ppt => "application/vnd.ms-powerpoint",
            Self::Odt => "application/vnd.oasis.opendocument.text",
            Self::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            Self::Odp => "application/vnd.oasis.opendocument.presentation",
            Self::Rtf => "application/rtf",
            Self::PlainText => "text/plain",
            Self::Markdown => "text/markdown",
            Self::Csv => "text/csv",
        }
    }

    /// Canonical lowercase extension, without a leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tiff",
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Doc => "doc",
            Self::Xls => "xls",
            Self::Ppt => "ppt",
            Self::Odt => "odt",
            Self::Ods => "ods",
            Self::Odp => "odp",
            Self::Rtf => "rtf",
            Self::PlainText => "txt",
            Self::Markdown => "md",
            Self::Csv => "csv",
        }
    }

    /// What has to happen to the file after storage.
    pub const fn kind(self) -> DocumentKind {
        match self {
            Self::Pdf => DocumentKind::Pdf,
            Self::Png | Self::Jpeg | Self::Tiff | Self::Webp | Self::Gif => DocumentKind::Image,
            Self::Docx
            | Self::Xlsx
            | Self::Pptx
            | Self::Doc
            | Self::Xls
            | Self::Ppt
            | Self::Odt
            | Self::Ods
            | Self::Odp
            | Self::Rtf => DocumentKind::Office,
            Self::PlainText | Self::Markdown | Self::Csv => DocumentKind::Text,
        }
    }

    /// Whether a generated PDF copy is required for viewing and binding.
    ///
    /// PDF is the canonical format, so everything else keeps its immutable
    /// original and gains a derivative.
    pub fn needs_pdf_derivative(self) -> bool {
        self.kind() != DocumentKind::Pdf
    }

    /// Whether text can be read out of the file without OCR.
    pub fn has_extractable_text(self) -> bool {
        matches!(
            self.kind(),
            DocumentKind::Pdf | DocumentKind::Office | DocumentKind::Text
        )
    }

    /// Whether the file may need OCR to become searchable.
    ///
    /// A PDF is included because a scanned PDF is images in a PDF wrapper, which
    /// is exactly the case OCR exists for.
    pub fn may_require_ocr(self) -> bool {
        matches!(self.kind(), DocumentKind::Image | DocumentKind::Pdf)
    }

    /// Resolves a MIME type, accepting common alternates.
    pub fn from_mime(mime: &str) -> Option<Self> {
        // Parameters like `; charset=utf-8` are not part of the identity.
        let base = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        if let Some(exact) = Self::ALL
            .into_iter()
            .find(|candidate| candidate.mime() == base)
        {
            return Some(exact);
        }

        // Alternates that appear in the wild and from older tooling.
        match base.as_str() {
            "image/tif" => Some(Self::Tiff),
            "image/jpg" | "image/pjpeg" => Some(Self::Jpeg),
            "text/x-markdown" | "text/x-md" => Some(Self::Markdown),
            "application/x-rtf" | "text/rtf" => Some(Self::Rtf),
            "application/x-pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// Resolves an extension, with or without a leading dot.
    pub fn from_extension(extension: &str) -> Option<Self> {
        let normalized = extension.trim_start_matches('.').to_ascii_lowercase();
        if let Some(exact) = Self::ALL
            .into_iter()
            .find(|candidate| candidate.extension() == normalized)
        {
            return Some(exact);
        }

        match normalized.as_str() {
            "jpeg" | "jpe" => Some(Self::Jpeg),
            "tif" => Some(Self::Tiff),
            "markdown" => Some(Self::Markdown),
            "text" | "log" => Some(Self::PlainText),
            _ => None,
        }
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.mime())
    }
}

impl FromStr for MediaType {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_mime(value).ok_or(DomainError::Invalid {
            field: "media_type",
            reason: "unsupported_file_type",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_round_trips_through_its_mime() {
        for media in MediaType::ALL {
            assert_eq!(
                MediaType::from_mime(media.mime()),
                Some(media),
                "{media:?} did not round-trip"
            );
        }
    }

    #[test]
    fn every_type_round_trips_through_its_extension() {
        for media in MediaType::ALL {
            assert_eq!(
                MediaType::from_extension(media.extension()),
                Some(media),
                "{media:?} did not round-trip"
            );
        }
    }

    #[test]
    fn mime_types_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for media in MediaType::ALL {
            assert!(seen.insert(media.mime()), "duplicate mime for {media:?}");
        }
    }

    #[test]
    fn extensions_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for media in MediaType::ALL {
            assert!(
                seen.insert(media.extension()),
                "duplicate extension for {media:?}"
            );
        }
    }

    #[test]
    fn charset_parameters_are_ignored() {
        assert_eq!(
            MediaType::from_mime("text/plain; charset=utf-8"),
            Some(MediaType::PlainText)
        );
    }

    #[test]
    fn mime_matching_is_case_insensitive() {
        assert_eq!(
            MediaType::from_mime("APPLICATION/PDF"),
            Some(MediaType::Pdf)
        );
    }

    #[test]
    fn common_alternates_are_accepted() {
        assert_eq!(MediaType::from_mime("image/jpg"), Some(MediaType::Jpeg));
        assert_eq!(MediaType::from_mime("image/tif"), Some(MediaType::Tiff));
        assert_eq!(MediaType::from_extension("jpeg"), Some(MediaType::Jpeg));
        assert_eq!(MediaType::from_extension(".TIF"), Some(MediaType::Tiff));
    }

    #[test]
    fn unsupported_types_are_rejected() {
        for candidate in [
            "application/x-msdownload",
            "application/zip",
            "video/mp4",
            "application/octet-stream",
            "",
        ] {
            assert_eq!(
                MediaType::from_mime(candidate),
                None,
                "accepted {candidate:?}"
            );
        }
        assert_eq!(MediaType::from_extension("exe"), None);
        assert_eq!(MediaType::from_extension("zip"), None);
    }

    #[test]
    fn parsing_an_unsupported_type_names_the_field() {
        let error = "video/mp4".parse::<MediaType>().expect_err("unsupported");
        assert_eq!(error.field(), Some("media_type"));
        assert_eq!(error.code(), "field_invalid");
    }

    #[test]
    fn only_pdf_avoids_a_derivative() {
        for media in MediaType::ALL {
            assert_eq!(
                media.needs_pdf_derivative(),
                media != MediaType::Pdf,
                "{media:?}"
            );
        }
    }

    #[test]
    fn images_have_no_extractable_text_but_may_need_ocr() {
        assert!(!MediaType::Png.has_extractable_text());
        assert!(MediaType::Png.may_require_ocr());
    }

    #[test]
    fn pdfs_have_extractable_text_and_may_still_need_ocr() {
        // A scanned PDF is images in a PDF wrapper.
        assert!(MediaType::Pdf.has_extractable_text());
        assert!(MediaType::Pdf.may_require_ocr());
    }

    #[test]
    fn office_documents_never_need_ocr() {
        for media in MediaType::ALL {
            if media.kind() == DocumentKind::Office {
                assert!(!media.may_require_ocr(), "{media:?}");
                assert!(media.has_extractable_text(), "{media:?}");
            }
        }
    }
}
