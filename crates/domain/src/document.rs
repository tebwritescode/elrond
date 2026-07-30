//! Document lifecycle rules.
//!
//! The lifecycle is the load-bearing invariant of the whole product: a binder
//! release pins published version identifiers, so anything that lets a published
//! version change underneath a release breaks reproducibility. The state machine
//! is therefore expressed as data with an explicit transition table rather than
//! as scattered `if` checks at the call sites.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

use crate::checksum::Sha256Checksum;
use crate::error::DomainError;
use crate::filename::{OriginalFilename, StorageKey};
use crate::id::{CategoryId, DocumentId, DocumentVersionId, UserId};
use crate::media::MediaType;

/// Where a document sits in its editorial workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Being prepared. Freely editable, not visible to viewers.
    Draft,
    /// Submitted for approval. Editing is frozen while reviewers look at it.
    InReview,
    /// Approved and distributable. Immutable from here on.
    Published,
    /// Replaced by a newer published version, but retained because existing
    /// binder releases still pin it.
    Superseded,
    /// Withdrawn from active use. Terminal.
    Archived,
}

impl LifecycleState {
    /// Every state, in workflow order.
    pub const ALL: [Self; 5] = [
        Self::Draft,
        Self::InReview,
        Self::Published,
        Self::Superseded,
        Self::Archived,
    ];

    /// Stable wire and storage representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::InReview => "in_review",
            Self::Published => "published",
            Self::Superseded => "superseded",
            Self::Archived => "archived",
        }
    }

    /// States reachable directly from this one.
    ///
    /// `Archived` is terminal on purpose. Un-archiving would let a document
    /// re-enter `Draft` and mutate content that an audit record or a binder
    /// release already refers to; the supported path is to publish a new
    /// version instead.
    pub const fn allowed_next(self) -> &'static [Self] {
        match self {
            Self::Draft => &[Self::InReview, Self::Archived],
            // Returning to Draft is how a reviewer requests changes.
            Self::InReview => &[Self::Draft, Self::Published, Self::Archived],
            Self::Published => &[Self::Superseded, Self::Archived],
            Self::Superseded => &[Self::Archived],
            Self::Archived => &[],
        }
    }

    /// Whether `self` may move directly to `next`.
    pub fn can_transition_to(self, next: Self) -> bool {
        // A no-op transition is not an error at the API boundary, but it is not
        // a state change either, so it is reported separately.
        self.allowed_next().contains(&next)
    }

    /// Moves to `next`, or explains why the move is forbidden.
    pub fn transition_to(self, next: Self) -> Result<Self, LifecycleTransitionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(LifecycleTransitionError {
                from: self,
                to: next,
            })
        }
    }

    /// Whether the document's own fields and file may still be edited in place.
    pub fn is_editable(self) -> bool {
        matches!(self, Self::Draft)
    }

    /// Whether the content is frozen and may only change by publishing a new
    /// immutable version.
    pub fn requires_new_version_to_change(self) -> bool {
        matches!(self, Self::Published | Self::Superseded | Self::Archived)
    }

    /// Whether a viewer-role account may see the document.
    pub fn is_visible_to_viewers(self) -> bool {
        matches!(self, Self::Published)
    }

    /// Whether a binder release may pin this state.
    ///
    /// Only published versions are eligible, which is what makes a release
    /// reproducible.
    pub fn is_bindable(self) -> bool {
        matches!(self, Self::Published)
    }

    /// Whether the state is terminal.
    pub fn is_terminal(self) -> bool {
        self.allowed_next().is_empty()
    }
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LifecycleState {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "in_review" => Ok(Self::InReview),
            "published" => Ok(Self::Published),
            "superseded" => Ok(Self::Superseded),
            "archived" => Ok(Self::Archived),
            _ => Err(DomainError::Invalid {
                field: "lifecycle_state",
                reason: "unknown_state",
            }),
        }
    }
}

/// A refused lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a {from} document cannot move to {to}")]
pub struct LifecycleTransitionError {
    /// The state the document is currently in.
    pub from: LifecycleState,
    /// The state that was requested.
    pub to: LifecycleState,
}

/// Longest document title.
const TITLE_MAX: usize = 300;

/// A validated document title.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct DocumentTitle(String);

impl DocumentTitle {
    /// Validates and normalizes a title.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");

        if normalized.is_empty() {
            return Err(DomainError::Required { field: "title" });
        }
        if normalized.chars().count() > TITLE_MAX {
            return Err(DomainError::TooLong {
                field: "title",
                max: TITLE_MAX,
            });
        }
        if normalized.chars().any(char::is_control) {
            return Err(DomainError::Invalid {
                field: "title",
                reason: "contains_control_characters",
            });
        }
        Ok(Self(normalized))
    }

    /// Derives a title from an uploaded filename.
    ///
    /// The extension is dropped and separators become spaces, so
    /// `annual_report-2026.pdf` becomes `annual report 2026` rather than being
    /// shown to users as a filename.
    pub fn from_filename(
        filename: &crate::filename::OriginalFilename,
    ) -> Result<Self, DomainError> {
        let spaced = filename
            .stem()
            .replace(['_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        // Fall back to the raw stem if replacing separators emptied it, which
        // happens for a name like `___.pdf`.
        if spaced.is_empty() {
            Self::parse(filename.stem())
        } else {
            Self::parse(&spaced)
        }
    }

    /// Borrows the title.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// First character uppercased, for alphabetical indexes in binders.
    pub fn index_letter(&self) -> char {
        self.0
            .chars()
            .find(|c| c.is_alphanumeric())
            .map_or('#', |c| c.to_ascii_uppercase())
    }
}

impl fmt::Display for DocumentTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocumentTitle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A monotonically increasing version number, starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionNumber(u32);

impl VersionNumber {
    /// The first version of a document.
    pub const FIRST: Self = Self(1);

    /// Wraps a stored value, rejecting zero.
    pub fn new(value: u32) -> Result<Self, DomainError> {
        if value == 0 {
            return Err(DomainError::Invalid {
                field: "version_number",
                reason: "versions_start_at_one",
            });
        }
        Ok(Self(value))
    }

    /// The next version after this one.
    #[must_use]
    pub fn next(self) -> Self {
        // Saturating rather than wrapping: a wrapped version number would silently
        // reorder history. Reaching u32::MAX versions of one document is not a
        // real scenario, but a wrap would be catastrophic if it happened.
        Self(self.0.saturating_add(1))
    }

    /// The underlying number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for VersionNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One immutable version of a document.
///
/// A version is never edited after it is created. Replacing a document's content
/// appends a new version, which is what lets a binder release pin an exact
/// version id and rebuild identically later.
///
/// Deliberately not `Serialize`. This struct holds [`StorageKey`] values, and the
/// internal storage layout is not something a client should ever see; the API
/// layer builds its own view type instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentVersion {
    /// Stable identifier. This is what a binder release pins.
    pub id: DocumentVersionId,
    /// The document this version belongs to.
    pub document_id: DocumentId,
    /// Sequence within the document, starting at 1.
    pub number: VersionNumber,
    /// Filename as uploaded, for display and download only.
    pub original_filename: OriginalFilename,
    /// Type of the original file.
    pub media_type: MediaType,
    /// Size of the original in bytes.
    pub byte_size: u64,
    /// Checksum of the original.
    pub checksum: Sha256Checksum,
    /// Content-addressed location of the original.
    pub storage_key: StorageKey,
    /// Checksum of the generated PDF, once one exists.
    pub derivative_checksum: Option<Sha256Checksum>,
    /// Location of the generated PDF, once one exists.
    pub derivative_key: Option<StorageKey>,
    /// Account that created this version.
    pub created_by: UserId,
    /// Optional note describing what changed.
    pub note: Option<String>,
    /// Creation timestamp in UTC.
    pub created_at: OffsetDateTime,
}

impl DocumentVersion {
    /// Whether a PDF suitable for viewing and binding is available.
    ///
    /// A PDF original is its own renderable form; anything else needs its
    /// derivative to have been generated.
    pub fn is_renderable(&self) -> bool {
        if self.media_type.needs_pdf_derivative() {
            self.derivative_key.is_some()
        } else {
            true
        }
    }

    /// The blob to render, view, and merge into a binder.
    pub fn renderable_key(&self) -> Option<&StorageKey> {
        if self.media_type.needs_pdf_derivative() {
            self.derivative_key.as_ref()
        } else {
            Some(&self.storage_key)
        }
    }

    /// Whether a PDF derivative still has to be produced.
    pub fn awaits_derivative(&self) -> bool {
        self.media_type.needs_pdf_derivative() && self.derivative_key.is_none()
    }
}

/// A logical document, independent of its versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    /// Stable identifier.
    pub id: DocumentId,
    /// Human-readable title.
    pub title: DocumentTitle,
    /// The single primary category.
    pub category_id: CategoryId,
    /// Where the document sits in its editorial workflow.
    pub lifecycle: LifecycleState,
    /// The most recent version.
    pub current_version_id: DocumentVersionId,
    /// How many versions exist.
    pub version_count: u32,
    /// Folder-relative path this document came from, when it was bulk-imported.
    ///
    /// Preserved so provenance survives an import; `None` for a direct upload.
    pub source_path: Option<String>,
    /// When the document should next be reviewed.
    pub review_due_at: Option<OffsetDateTime>,
    /// Account that created the document.
    pub created_by: UserId,
    /// Creation timestamp in UTC.
    pub created_at: OffsetDateTime,
    /// Last modification timestamp in UTC.
    pub updated_at: OffsetDateTime,
}

impl Document {
    /// Whether metadata and content may be edited in place.
    pub fn is_editable(&self) -> bool {
        self.lifecycle.is_editable()
    }

    /// Whether a binder release may pin this document's current version.
    pub fn is_bindable(&self) -> bool {
        self.lifecycle.is_bindable()
    }

    /// Whether a viewer-role account may see it.
    pub fn is_visible_to_viewers(&self) -> bool {
        self.lifecycle.is_visible_to_viewers()
    }

    /// Whether the review date has passed as of `now`.
    pub fn is_review_overdue(&self, now: OffsetDateTime) -> bool {
        // Only published material can be overdue: a draft is already being worked
        // on, and an archived document is out of use.
        self.lifecycle == LifecycleState::Published
            && self.review_due_at.is_some_and(|due| now >= due)
    }

    /// Whether review falls due within `window` of `now`.
    pub fn is_review_due_soon(&self, now: OffsetDateTime, window: Duration) -> bool {
        self.lifecycle == LifecycleState::Published
            && self
                .review_due_at
                .is_some_and(|due| due > now && due <= now + window)
    }

    /// Applies a lifecycle transition, returning the updated document.
    pub fn transition_to(mut self, next: LifecycleState) -> Result<Self, DomainError> {
        self.lifecycle = self.lifecycle.transition_to(next)?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::LifecycleState as S;
    use super::*;

    #[test]
    fn states_round_trip_through_storage_form() {
        for state in S::ALL {
            assert_eq!(state.as_str().parse::<S>().expect("known state"), state);
        }
    }

    #[test]
    fn unknown_state_is_rejected() {
        assert!("pending".parse::<S>().is_err());
    }

    #[test]
    fn happy_path_walks_draft_to_published() {
        let state = S::Draft
            .transition_to(S::InReview)
            .and_then(|s| s.transition_to(S::Published))
            .expect("standard approval path");
        assert_eq!(state, S::Published);
    }

    #[test]
    fn reviewer_can_send_a_document_back_to_draft() {
        assert!(S::InReview.can_transition_to(S::Draft));
    }

    #[test]
    fn published_cannot_return_to_an_editable_state() {
        for forbidden in [S::Draft, S::InReview] {
            let error = S::Published
                .transition_to(forbidden)
                .expect_err("published content must not become editable again");
            assert_eq!(error.from, S::Published);
            assert_eq!(error.to, forbidden);
        }
    }

    #[test]
    fn draft_cannot_be_published_without_review() {
        assert!(!S::Draft.can_transition_to(S::Published));
    }

    #[test]
    fn archived_is_terminal() {
        assert!(S::Archived.is_terminal());
        for target in S::ALL {
            assert!(
                !S::Archived.can_transition_to(target),
                "archived must not reach {target}"
            );
        }
    }

    #[test]
    fn every_state_can_be_archived_except_archived_itself() {
        for state in S::ALL {
            let expected = state != S::Archived;
            assert_eq!(state.can_transition_to(S::Archived), expected);
        }
    }

    #[test]
    fn no_state_transitions_to_itself() {
        for state in S::ALL {
            assert!(!state.can_transition_to(state));
        }
    }

    #[test]
    fn only_draft_is_editable_in_place() {
        for state in S::ALL {
            assert_eq!(state.is_editable(), state == S::Draft);
        }
    }

    #[test]
    fn only_published_is_bindable_and_viewer_visible() {
        for state in S::ALL {
            assert_eq!(state.is_bindable(), state == S::Published);
            assert_eq!(state.is_visible_to_viewers(), state == S::Published);
        }
    }

    #[test]
    fn frozen_states_require_a_new_version() {
        assert!(!S::Draft.requires_new_version_to_change());
        assert!(!S::InReview.requires_new_version_to_change());
        for frozen in [S::Published, S::Superseded, S::Archived] {
            assert!(frozen.requires_new_version_to_change());
        }
    }

    // ---------------------------------------------------------------- titles

    #[test]
    fn titles_collapse_whitespace() {
        assert_eq!(
            DocumentTitle::parse("  Annual   Report \n 2026 ")
                .expect("valid")
                .as_str(),
            "Annual Report 2026"
        );
    }

    #[test]
    fn titles_reject_blank_and_control_characters() {
        assert!(DocumentTitle::parse("").is_err());
        assert!(DocumentTitle::parse("   ").is_err());
        assert!(DocumentTitle::parse("Report\u{0007}").is_err());
    }

    #[test]
    fn overlong_titles_are_rejected() {
        assert_eq!(
            DocumentTitle::parse(&"a".repeat(TITLE_MAX + 1))
                .expect_err("too long")
                .code(),
            "field_too_long"
        );
    }

    #[test]
    fn a_title_derived_from_a_filename_drops_the_extension_and_separators() {
        let filename = OriginalFilename::parse("annual_report-2026.pdf").expect("valid");
        assert_eq!(
            DocumentTitle::from_filename(&filename)
                .expect("valid")
                .as_str(),
            "annual report 2026"
        );
    }

    #[test]
    fn a_filename_of_only_separators_still_yields_a_title() {
        // Replacing separators would empty the string, so the raw stem is used.
        let filename = OriginalFilename::parse("___.pdf").expect("valid");
        assert_eq!(
            DocumentTitle::from_filename(&filename)
                .expect("valid")
                .as_str(),
            "___"
        );
    }

    #[test]
    fn index_letters_are_uppercase_and_skip_punctuation() {
        let cases = [
            ("annual report", 'A'),
            ("2026 budget", '2'),
            ("\"quoted\" title", 'Q'),
            ("-- dashes", 'D'),
        ];
        for (title, expected) in cases {
            assert_eq!(
                DocumentTitle::parse(title).expect("valid").index_letter(),
                expected,
                "for {title:?}"
            );
        }
    }

    // -------------------------------------------------------- version numbers

    #[test]
    fn versions_start_at_one_and_zero_is_refused() {
        assert_eq!(VersionNumber::FIRST.get(), 1);
        assert!(VersionNumber::new(0).is_err());
        assert_eq!(VersionNumber::new(7).expect("valid").get(), 7);
    }

    #[test]
    fn version_numbers_increase_and_never_wrap() {
        assert_eq!(VersionNumber::FIRST.next().get(), 2);

        // A wrapped version number would silently reorder history.
        let last = VersionNumber::new(u32::MAX).expect("valid");
        assert_eq!(last.next(), last);
    }

    #[test]
    fn version_numbers_order_naturally() {
        let mut numbers = [
            VersionNumber::new(10).expect("valid"),
            VersionNumber::FIRST,
            VersionNumber::new(3).expect("valid"),
        ];
        numbers.sort();
        assert_eq!(
            numbers.iter().map(|n| n.get()).collect::<Vec<_>>(),
            vec![1, 3, 10]
        );
    }

    // ------------------------------------------------------ document versions

    fn version(media_type: MediaType, derivative: bool) -> DocumentVersion {
        let digest: Sha256Checksum =
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .expect("valid digest");
        DocumentVersion {
            id: DocumentVersionId::new(),
            document_id: DocumentId::new(),
            number: VersionNumber::FIRST,
            original_filename: OriginalFilename::parse("report.bin").expect("valid"),
            media_type,
            byte_size: 1024,
            checksum: digest,
            storage_key: StorageKey::derive(crate::filename::BlobClass::Original, digest),
            derivative_checksum: derivative.then_some(digest),
            derivative_key: derivative.then(|| StorageKey::derive_derivative(digest)),
            created_by: UserId::new(),
            note: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_pdf_original_is_renderable_without_a_derivative() {
        let pdf = version(MediaType::Pdf, false);
        assert!(pdf.is_renderable());
        assert!(!pdf.awaits_derivative());
        assert_eq!(pdf.renderable_key(), Some(&pdf.storage_key));
    }

    #[test]
    fn a_non_pdf_original_is_not_renderable_until_converted() {
        let pending = version(MediaType::Docx, false);
        assert!(!pending.is_renderable());
        assert!(pending.awaits_derivative());
        assert_eq!(pending.renderable_key(), None);

        let converted = version(MediaType::Docx, true);
        assert!(converted.is_renderable());
        assert!(!converted.awaits_derivative());
        assert_eq!(
            converted.renderable_key(),
            converted.derivative_key.as_ref(),
            "the derivative is what gets viewed and bound, not the original"
        );
    }

    #[test]
    fn every_non_pdf_type_awaits_a_derivative() {
        for media in MediaType::ALL {
            let candidate = version(media, false);
            assert_eq!(
                candidate.awaits_derivative(),
                media != MediaType::Pdf,
                "{media:?}"
            );
        }
    }

    // --------------------------------------------------------------- documents

    fn document(lifecycle: S, review_due_at: Option<OffsetDateTime>) -> Document {
        Document {
            id: DocumentId::new(),
            title: DocumentTitle::parse("Annual Report").expect("valid"),
            category_id: CategoryId::new(),
            lifecycle,
            current_version_id: DocumentVersionId::new(),
            version_count: 1,
            source_path: None,
            review_due_at,
            created_by: UserId::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn document_capabilities_follow_its_lifecycle() {
        for state in S::ALL {
            let doc = document(state, None);
            assert_eq!(doc.is_editable(), state == S::Draft);
            assert_eq!(doc.is_bindable(), state == S::Published);
            assert_eq!(doc.is_visible_to_viewers(), state == S::Published);
        }
    }

    #[test]
    fn a_document_transition_updates_the_document() {
        let doc = document(S::Draft, None);
        let submitted = doc.transition_to(S::InReview).expect("allowed");
        assert_eq!(submitted.lifecycle, S::InReview);
    }

    #[test]
    fn a_forbidden_document_transition_reports_a_domain_error() {
        let error = document(S::Published, None)
            .transition_to(S::Draft)
            .expect_err("forbidden");
        assert_eq!(error.code(), "lifecycle_transition_forbidden");
    }

    #[test]
    fn only_published_documents_can_be_overdue_for_review() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(400);
        let past = OffsetDateTime::UNIX_EPOCH + Duration::days(200);

        for state in S::ALL {
            let doc = document(state, Some(past));
            assert_eq!(
                doc.is_review_overdue(now),
                state == S::Published,
                "a {state} document should not be reported overdue"
            );
        }
    }

    #[test]
    fn a_document_with_no_review_date_is_never_overdue() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(400);
        assert!(!document(S::Published, None).is_review_overdue(now));
        assert!(!document(S::Published, None).is_review_due_soon(now, Duration::days(30)));
    }

    #[test]
    fn review_due_soon_covers_the_window_but_not_the_past() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
        let window = Duration::days(30);

        let inside = document(S::Published, Some(now + Duration::days(10)));
        assert!(inside.is_review_due_soon(now, window));
        assert!(!inside.is_review_overdue(now));

        let beyond = document(S::Published, Some(now + Duration::days(31)));
        assert!(!beyond.is_review_due_soon(now, window));

        // Already overdue is a different queue, so it must not also count as
        // "due soon" and be shown twice on the dashboard.
        let overdue = document(S::Published, Some(now - Duration::days(1)));
        assert!(!overdue.is_review_due_soon(now, window));
        assert!(overdue.is_review_overdue(now));
    }

    #[test]
    fn the_exact_due_instant_counts_as_overdue() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(100);
        assert!(document(S::Published, Some(now)).is_review_overdue(now));
    }

    #[test]
    fn transition_error_converts_into_a_domain_error() {
        let error: DomainError = S::Published
            .transition_to(S::Draft)
            .expect_err("forbidden")
            .into();
        assert_eq!(error.code(), "lifecycle_transition_forbidden");
        assert_eq!(error.field(), None);
    }
}
