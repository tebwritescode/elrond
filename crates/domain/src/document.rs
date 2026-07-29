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

use crate::error::DomainError;

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
            Err(LifecycleTransitionError { from: self, to: next })
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
