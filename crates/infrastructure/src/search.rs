//! SQLite FTS5 full-text search.

use async_trait::async_trait;
use elrond_application::ports::{IndexedDocument, RepositoryError, SearchIndex, SearchOutcome};
use elrond_domain::DocumentId;
use sqlx::{Pool, Row, Sqlite};

/// Most terms taken from one query.
///
/// A query with hundreds of terms is either a paste accident or an attempt to make
/// the planner do unbounded work.
const MAX_TERMS: usize = 16;

/// Full-text search over the `documents_fts` virtual table.
#[derive(Debug, Clone)]
pub struct SqliteSearchIndex {
    pool: Pool<Sqlite>,
}

impl SqliteSearchIndex {
    /// Binds the index to a connected database.
    pub fn new(database: &crate::db::Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

/// Rewrites user input into a safe FTS5 `MATCH` expression.
///
/// FTS5 has its own query language: bare `"`, `(`, `*`, `:`, `^`, `-`, `NEAR`, `OR`
/// and `AND` are all operators. Passing user input straight through means an
/// unbalanced quote surfaces as `fts5: syntax error near ...`, which the person
/// searching can neither understand nor fix.
///
/// So the input is reduced to alphanumeric terms and each is re-emitted as a
/// quoted phrase with a prefix wildcard. Everything the user typed is treated as
/// text, no operator can survive, and type-ahead still works because of the
/// trailing `*`.
///
/// Returns `None` when nothing searchable remains.
fn to_match_expression(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|term| !term.is_empty())
        // Quotes are stripped by the split above, so no term can close the phrase
        // it is about to be wrapped in.
        .map(str::to_lowercase)
        .take(MAX_TERMS)
        .collect();

    if terms.is_empty() {
        return None;
    }

    // AND rather than OR: with OR, one common word would drag in most of the
    // library and bury the documents that match everything the user typed.
    Some(
        terms
            .iter()
            .map(|term| format!("\"{term}\"*"))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

#[async_trait]
impl SearchIndex for SqliteSearchIndex {
    async fn index(&self, document: IndexedDocument) -> Result<(), RepositoryError> {
        let id = document.document_id.to_string();
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        // FTS5 has no upsert, so an entry is replaced by deleting and reinserting.
        // Inside a transaction, so a concurrent search never observes the document
        // as missing.
        sqlx::query("DELETE FROM documents_fts WHERE document_id = ?1")
            .bind(&id)
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;

        sqlx::query(
            "INSERT INTO documents_fts (document_id, title, filename, tags, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&id)
        .bind(&document.title)
        .bind(&document.filename)
        .bind(&document.tags)
        .bind(&document.content)
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn remove(&self, document_id: DocumentId) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM documents_fts WHERE document_id = ?1")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn search(&self, query: &str, limit: u32) -> Result<SearchOutcome, RepositoryError> {
        let Some(expression) = to_match_expression(query) else {
            return Ok(SearchOutcome::default());
        };

        // `bm25` with column weights: a title hit matters far more than a body hit,
        // and a filename hit more than a tag. Lower bm25 output is a better match,
        // hence the ascending order.
        let rows = sqlx::query(
            "SELECT document_id
             FROM documents_fts
             WHERE documents_fts MATCH ?1
             ORDER BY bm25(documents_fts, 0.0, 10.0, 5.0, 3.0, 1.0)
             LIMIT ?2",
        )
        .bind(&expression)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        let mut document_ids = Vec::with_capacity(rows.len());
        for row in &rows {
            let raw: String = row
                .try_get("document_id")
                .map_err(RepositoryError::backend)?;
            // A row whose id cannot be parsed is a stale or corrupt index entry.
            // Skipping it degrades the result set rather than failing the search.
            if let Ok(id) = raw.parse::<DocumentId>() {
                document_ids.push(id);
            } else {
                tracing::warn!(
                    document_id = raw,
                    "skipping an unparseable search index entry"
                );
            }
        }

        Ok(SearchOutcome { document_ids })
    }

    async fn rebuild(&self, documents: Vec<IndexedDocument>) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        sqlx::query("DELETE FROM documents_fts")
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;

        for document in &documents {
            sqlx::query(
                "INSERT INTO documents_fts (document_id, title, filename, tags, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(document.document_id.to_string())
            .bind(&document.title)
            .bind(&document.filename)
            .bind(&document.tags)
            .bind(&document.content)
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;
        }

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn index() -> (Database, SqliteSearchIndex) {
        let database = Database::connect_in_memory().await.expect("connects");
        let index = SqliteSearchIndex::new(&database);
        (database, index)
    }

    fn document(
        id: DocumentId,
        title: &str,
        filename: &str,
        tags: &str,
        content: &str,
    ) -> IndexedDocument {
        IndexedDocument {
            document_id: id,
            title: title.to_owned(),
            filename: filename.to_owned(),
            tags: tags.to_owned(),
            content: content.to_owned(),
        }
    }

    // ------------------------------------------------------ query sanitization

    #[test]
    fn plain_terms_become_prefix_phrases_joined_by_and() {
        assert_eq!(
            to_match_expression("board minutes").as_deref(),
            Some("\"board\"* AND \"minutes\"*")
        );
    }

    #[test]
    fn fts5_operators_are_neutralized_rather_than_passed_through() {
        // Every one of these is valid FTS5 syntax that would either error or change
        // the meaning of the query if it reached MATCH intact.
        for hostile in [
            "board\"",
            "board\" OR \"",
            "board NEAR minutes",
            "board OR minutes",
            "board AND NOT minutes",
            "(board",
            "board*)",
            "title:board",
            "^board",
            "\"unbalanced",
            "-board",
        ] {
            let expression = to_match_expression(hostile).expect("some terms remain");
            // Quotes only ever appear as the wrappers this function adds, in pairs.
            assert_eq!(
                expression.matches('"').count() % 2,
                0,
                "unbalanced quotes for {hostile:?}: {expression}"
            );
            for forbidden in ['(', ')', ':', '^', '-'] {
                assert!(
                    !expression.contains(forbidden),
                    "{forbidden:?} survived for {hostile:?}: {expression}"
                );
            }
        }
    }

    #[test]
    fn operator_words_are_treated_as_text() {
        // Quoted, so FTS5 reads them as terms rather than as operators.
        assert_eq!(
            to_match_expression("near or and").as_deref(),
            Some("\"near\"* AND \"or\"* AND \"and\"*")
        );
    }

    #[test]
    fn a_query_with_nothing_searchable_yields_no_expression() {
        for empty in ["", "   ", "***", "()", "\"\"", "---"] {
            assert_eq!(to_match_expression(empty), None, "for {empty:?}");
        }
    }

    #[test]
    fn the_term_count_is_capped() {
        let many = (0..100)
            .map(|index| format!("term{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let expression = to_match_expression(&many).expect("terms remain");
        assert_eq!(expression.matches(" AND ").count(), MAX_TERMS - 1);
    }

    #[test]
    fn accented_input_is_preserved_for_the_tokenizer_to_fold() {
        assert_eq!(
            to_match_expression("résumé").as_deref(),
            Some("\"résumé\"*")
        );
    }

    // ---------------------------------------------------------------- indexing

    #[tokio::test]
    async fn an_indexed_document_is_found_by_title() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(
                id,
                "Retention Policy",
                "retention.pdf",
                "policy",
                "",
            ))
            .await
            .expect("indexed");

        let outcome = index.search("retention", 10).await.expect("searched");
        assert_eq!(outcome.document_ids, vec![id]);
    }

    #[tokio::test]
    async fn a_document_is_found_by_filename_tag_and_content() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(
                id,
                "Untitled",
                "quarterly-figures.xlsx",
                "finance board",
                "The committee approved the schedule.",
            ))
            .await
            .expect("indexed");

        for query in ["quarterly", "finance", "committee", "schedule"] {
            let outcome = index.search(query, 10).await.expect("searched");
            assert_eq!(outcome.document_ids, vec![id], "for {query:?}");
        }
    }

    #[tokio::test]
    async fn prefix_search_supports_type_ahead() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(id, "Retention Policy", "r.pdf", "", ""))
            .await
            .expect("indexed");

        for partial in ["ret", "rete", "reten", "retention"] {
            assert_eq!(
                index
                    .search(partial, 10)
                    .await
                    .expect("searched")
                    .document_ids,
                vec![id],
                "for {partial:?}"
            );
        }
    }

    #[tokio::test]
    async fn all_terms_must_match() {
        let (_db, index) = index().await;
        let both = DocumentId::new();
        let one = DocumentId::new();
        index
            .index(document(both, "Retention Policy", "a.pdf", "", ""))
            .await
            .expect("indexed");
        index
            .index(document(one, "Retention Schedule", "b.pdf", "", ""))
            .await
            .expect("indexed");

        let outcome = index
            .search("retention policy", 10)
            .await
            .expect("searched");
        assert_eq!(
            outcome.document_ids,
            vec![both],
            "a document matching only one term should not appear"
        );
    }

    #[tokio::test]
    async fn a_title_match_outranks_a_body_match() {
        let (_db, index) = index().await;
        let in_title = DocumentId::new();
        let in_body = DocumentId::new();
        index
            .index(document(
                in_body,
                "Something Else",
                "b.pdf",
                "",
                "retention retention retention",
            ))
            .await
            .expect("indexed");
        index
            .index(document(in_title, "Retention", "a.pdf", "", ""))
            .await
            .expect("indexed");

        let outcome = index.search("retention", 10).await.expect("searched");
        assert_eq!(
            outcome.document_ids.first(),
            Some(&in_title),
            "column weights should put the title hit first"
        );
    }

    #[tokio::test]
    async fn diacritics_are_folded_both_ways() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(id, "Résumé of Findings", "r.pdf", "", ""))
            .await
            .expect("indexed");

        for query in ["resume", "résumé", "RESUME"] {
            assert_eq!(
                index
                    .search(query, 10)
                    .await
                    .expect("searched")
                    .document_ids,
                vec![id],
                "for {query:?}"
            );
        }
    }

    #[tokio::test]
    async fn reindexing_replaces_rather_than_duplicates() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(id, "Original Title", "a.pdf", "", ""))
            .await
            .expect("indexed");
        index
            .index(document(id, "Corrected Title", "a.pdf", "", ""))
            .await
            .expect("reindexed");

        assert_eq!(
            index
                .search("corrected", 10)
                .await
                .expect("searched")
                .document_ids,
            vec![id]
        );
        assert!(
            index
                .search("original", 10)
                .await
                .expect("searched")
                .document_ids
                .is_empty(),
            "the previous entry should be gone, not shadowed"
        );
    }

    #[tokio::test]
    async fn removing_a_document_removes_it_from_results() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(id, "Retention Policy", "a.pdf", "", ""))
            .await
            .expect("indexed");
        index.remove(id).await.expect("removed");

        assert!(
            index
                .search("retention", 10)
                .await
                .expect("searched")
                .document_ids
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_hostile_query_never_produces_a_syntax_error() {
        let (_db, index) = index().await;
        index
            .index(document(
                DocumentId::new(),
                "Retention Policy",
                "a.pdf",
                "",
                "",
            ))
            .await
            .expect("indexed");

        // Each of these is valid FTS5 syntax that would either error or silently
        // change meaning if it reached MATCH intact. What matters here is only that
        // the search completes; what it matches is covered below.
        for hostile in [
            "retention\"",
            "retention OR \"",
            "retention NEAR policy",
            "retention AND NOT policy",
            "(retention",
            "retention*)",
            "title:retention",
            "^retention",
            "-retention",
            "\"\"\"\"",
            "((((",
            "* * *",
        ] {
            index
                .search(hostile, 10)
                .await
                .unwrap_or_else(|error| panic!("query {hostile:?} errored: {error}"));
        }
    }

    #[tokio::test]
    async fn punctuation_noise_does_not_change_what_matches() {
        let (_db, index) = index().await;
        let id = DocumentId::new();
        index
            .index(document(id, "Retention Policy", "a.pdf", "", ""))
            .await
            .expect("indexed");

        // Noise made only of operator characters is discarded, so these are all the
        // same query as a plain "retention".
        for noisy in [
            "retention\"",
            "(retention",
            "retention*)",
            "^retention",
            "-retention",
            "\"retention",
            "  retention  ",
        ] {
            assert_eq!(
                index
                    .search(noisy, 10)
                    .await
                    .expect("searched")
                    .document_ids,
                vec![id],
                "for {noisy:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_operator_word_becomes_a_required_term() {
        let (_db, index) = index().await;
        index
            .index(document(
                DocumentId::new(),
                "Retention Policy",
                "a.pdf",
                "",
                "",
            ))
            .await
            .expect("indexed");

        // `OR` is neutralized into a literal term, so it narrows the query rather
        // than widening it. Narrowing is the safe direction: the alternative is an
        // attacker-supplied `OR` matching the whole library.
        assert!(
            index
                .search("retention OR nonsense", 10)
                .await
                .expect("searched")
                .document_ids
                .is_empty(),
            "a bare OR must not widen the result set"
        );
    }

    #[tokio::test]
    async fn an_empty_query_returns_nothing_without_touching_the_index() {
        let (_db, index) = index().await;
        index
            .index(document(DocumentId::new(), "Retention", "a.pdf", "", ""))
            .await
            .expect("indexed");

        // Distinct from "match everything": an empty search box should not dump the
        // library through the relevance-ordered path.
        assert!(
            index
                .search("   ", 10)
                .await
                .expect("searched")
                .document_ids
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_limit_is_respected() {
        let (_db, index) = index().await;
        for number in 0..10 {
            index
                .index(document(
                    DocumentId::new(),
                    &format!("Policy {number}"),
                    "a.pdf",
                    "",
                    "",
                ))
                .await
                .expect("indexed");
        }

        assert_eq!(
            index
                .search("policy", 3)
                .await
                .expect("searched")
                .document_ids
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn a_rebuild_replaces_the_whole_index() {
        let (_db, index) = index().await;
        let stale = DocumentId::new();
        let fresh = DocumentId::new();
        index
            .index(document(stale, "Stale Entry", "a.pdf", "", ""))
            .await
            .expect("indexed");

        index
            .rebuild(vec![document(fresh, "Fresh Entry", "b.pdf", "", "")])
            .await
            .expect("rebuilt");

        assert!(
            index
                .search("stale", 10)
                .await
                .expect("searched")
                .document_ids
                .is_empty()
        );
        assert_eq!(
            index
                .search("fresh", 10)
                .await
                .expect("searched")
                .document_ids,
            vec![fresh]
        );
    }
}
