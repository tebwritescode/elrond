//! Rendering port for binder output.
//!
//! The use case decides *what* goes in a binder and in what order; this port
//! turns that plan into PDF bytes. Keeping the split here means the composition
//! rules are testable without a PDF library, and a different renderer is a
//! substitution rather than a rewrite.

use async_trait::async_trait;
use thiserror::Error;

/// Paper size for generated pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    /// 210 × 297 mm.
    A4,
    /// 8.5 × 11 in.
    Letter,
}

impl PageSize {
    /// Width and height in PostScript points.
    pub const fn points(self) -> (f32, f32) {
        match self {
            // 595.276 × 841.890, rounded to the values every PDF tool uses.
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612.0, 792.0),
        }
    }
}

/// How pages are numbered in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageNumbering {
    /// No page numbers.
    None,
    /// One sequence across the whole binder.
    Continuous,
}

/// Settings that shape the generated output.
///
/// Snapshotted into every release, so a rebuild uses the settings the release was
/// made with rather than whatever the binder looks like today.
#[expect(
    clippy::struct_excessive_bools,
    reason = "these are independent operator toggles, not a state machine; \
              collapsing them into an enum would misrepresent them as mutually exclusive"
)]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BinderSettings {
    /// Paper size for generated pages.
    pub page_size: PageSize,
    /// Whether to emit a cover page.
    pub include_cover: bool,
    /// Whether to emit a table of contents.
    pub include_toc: bool,
    /// Whether to emit a full-page separator before each section.
    pub include_separators: bool,
    /// Whether to emit a full-page separator before each document as well.
    pub document_separators: bool,
    /// How to number pages.
    pub page_numbering: PageNumbering,
    /// Whether to pad sections so each separator falls on a right-hand page.
    ///
    /// Only meaningful when printing double-sided; it inserts a blank page where
    /// the count would otherwise put a separator on the back of a sheet.
    pub duplex_blank_pages: bool,
}

impl Default for BinderSettings {
    fn default() -> Self {
        Self {
            page_size: PageSize::A4,
            include_cover: true,
            include_toc: true,
            include_separators: true,
            document_separators: true,
            page_numbering: PageNumbering::Continuous,
            duplex_blank_pages: false,
        }
    }
}

/// Text for the front cover.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverSpec {
    /// Main title.
    pub title: String,
    /// Optional subtitle.
    pub subtitle: Option<String>,
    /// Owning organization.
    pub organization: Option<String>,
    /// Release label, for example "Release 3".
    pub release_label: Option<String>,
    /// Build date, already formatted for display.
    pub built_on: Option<String>,
}

/// One item in the ordered binder plan.
#[derive(Debug, Clone)]
pub enum PlanEntry {
    /// A section, which becomes a full-page separator and an outline entry.
    Section {
        /// Nesting depth, starting at 0.
        level: u8,
        /// Section name.
        title: String,
        /// Ancestor names, outermost first, shown above the title.
        path: Vec<String>,
    },
    /// A document, whose PDF pages are merged in.
    Document {
        /// Nesting depth of the containing section.
        level: u8,
        /// Title shown in the contents and the outline.
        title: String,
        /// Category names enclosing the document, outermost first. Shown on the
        /// document's own separator page.
        path: Vec<String>,
        /// The document's PDF bytes.
        pdf: Vec<u8>,
    },
}

/// Everything needed to render one binder.
#[derive(Debug, Clone)]
pub struct BinderPlan {
    /// Cover text.
    pub cover: CoverSpec,
    /// Output settings.
    pub settings: BinderSettings,
    /// Ordered contents.
    pub entries: Vec<PlanEntry>,
    /// Timestamp written into the document metadata.
    ///
    /// Supplied rather than read from the clock so two builds of the same release
    /// produce byte-identical output.
    pub built_at: time::OffsetDateTime,
}

/// Where one plan entry ended up in the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedEntry {
    /// Title as it appears in the contents.
    pub title: String,
    /// Nesting depth.
    pub level: u8,
    /// Whether this entry is a section.
    pub is_section: bool,
    /// One-based page number the entry starts on.
    pub page_start: u32,
    /// How many pages the entry occupies.
    pub page_count: u32,
}

/// A rendered binder.
#[derive(Debug, Clone)]
pub struct RenderedBinder {
    /// The PDF bytes.
    pub bytes: Vec<u8>,
    /// Total pages.
    pub page_count: u32,
    /// Where each entry landed, for the release record.
    pub placements: Vec<PlacedEntry>,
}

/// A failure while rendering.
#[derive(Debug, Error)]
pub enum RenderError {
    /// A source document could not be parsed as a PDF.
    #[error("{title} could not be read as a PDF")]
    UnreadableSource {
        /// Which document failed.
        title: String,
    },

    /// A source PDF had no pages.
    #[error("{title} contains no pages")]
    EmptySource {
        /// Which document failed.
        title: String,
    },

    /// The plan contained nothing to render.
    #[error("the binder has no content to build")]
    EmptyPlan,

    /// The renderer itself failed.
    #[error("binder rendering failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl RenderError {
    /// Wraps an arbitrary renderer error.
    pub fn backend<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(error))
    }
}

/// Turns a binder plan into PDF bytes.
#[async_trait]
pub trait BinderRenderer: Send + Sync + 'static {
    /// Renders a plan.
    ///
    /// Must be deterministic: the same plan has to produce the same bytes, or the
    /// output checksum recorded against a release is meaningless.
    async fn render(&self, plan: BinderPlan) -> Result<RenderedBinder, RenderError>;
}
