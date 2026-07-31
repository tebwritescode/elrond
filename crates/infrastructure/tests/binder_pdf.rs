//! Tests for native binder PDF rendering.
//!
//! Every assertion here reloads the produced bytes with a PDF parser rather than
//! trusting the builder's own bookkeeping: a renderer that reports success while
//! emitting a file no viewer can open is the failure mode that matters.

use elrond_application::ports::{
    BinderPlan, BinderRenderer, BinderSettings, CoverSpec, PageNumbering, PageSize, PlanEntry,
    RenderError,
};
use elrond_infrastructure::NativeBinderRenderer;
use lopdf::{Document, Object};
use time::OffsetDateTime;

/// A fixed instant, so output is reproducible.
fn built_at() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid timestamp")
}

/// Builds a source PDF with `pages` pages, using the same renderer under test so
/// the fixture is always a document this code can actually read.
async fn source_pdf(title: &str, pages: usize) -> Vec<u8> {
    let entries = (0..pages)
        .map(|index| PlanEntry::Section {
            level: 0,
            title: format!("{title} page {index}"),
            path: Vec::new(),
        })
        .collect();

    let rendered = NativeBinderRenderer
        .render(BinderPlan {
            cover: CoverSpec::default(),
            settings: BinderSettings {
                include_cover: false,
                include_toc: false,
                include_separators: true,
                document_separators: false,
                page_numbering: PageNumbering::None,
                duplex_blank_pages: false,
                page_size: PageSize::A4,
            },
            entries,
            built_at: built_at(),
        })
        .await
        .expect("fixture renders");

    assert_eq!(rendered.page_count as usize, pages);
    rendered.bytes
}

/// A plan with a cover, contents, two sections, and three documents.
async fn sample_plan() -> BinderPlan {
    BinderPlan {
        cover: CoverSpec {
            title: "Governance Binder".to_owned(),
            subtitle: Some("Policies and board minutes".to_owned()),
            organization: Some("Records Office".to_owned()),
            release_label: Some("Release 1".to_owned()),
            built_on: Some("1 January 2026".to_owned()),
        },
        settings: BinderSettings::default(),
        entries: vec![
            PlanEntry::Section {
                level: 0,
                title: "Policies".to_owned(),
                path: Vec::new(),
            },
            PlanEntry::Document {
                level: 1,
                title: "Retention Policy".to_owned(),
                path: vec!["Policies".to_owned()],
                pdf: source_pdf("retention", 3).await,
            },
            PlanEntry::Document {
                level: 1,
                title: "Access Policy".to_owned(),
                path: vec!["Policies".to_owned()],
                pdf: source_pdf("access", 2).await,
            },
            PlanEntry::Section {
                level: 0,
                title: "Board Minutes".to_owned(),
                path: Vec::new(),
            },
            PlanEntry::Document {
                level: 1,
                title: "January Minutes".to_owned(),
                path: vec!["Board Minutes".to_owned()],
                pdf: source_pdf("january", 1).await,
            },
        ],
        built_at: built_at(),
    }
}

#[tokio::test]
async fn a_binder_renders_to_a_readable_pdf() {
    let rendered = NativeBinderRenderer
        .render(sample_plan().await)
        .expect_render()
        .await;

    assert!(
        rendered.bytes.starts_with(b"%PDF-"),
        "output does not begin with a PDF header"
    );
    assert!(
        rendered.bytes.windows(5).any(|window| window == b"%%EOF"),
        "output has no end-of-file marker"
    );

    // The real check: a parser can read it back.
    let reloaded = Document::load_mem(&rendered.bytes).expect("output parses as a PDF");
    assert_eq!(
        reloaded.get_pages().len(),
        rendered.page_count as usize,
        "the reported page count disagrees with the file"
    );
}

/// Small helper so the tests read as one expression.
trait ExpectRender {
    async fn expect_render(self) -> elrond_application::ports::RenderedBinder;
}

impl<F> ExpectRender for F
where
    F: std::future::Future<Output = Result<elrond_application::ports::RenderedBinder, RenderError>>,
{
    async fn expect_render(self) -> elrond_application::ports::RenderedBinder {
        self.await.expect("rendering succeeds")
    }
}

#[tokio::test]
async fn the_page_count_accounts_for_every_part() {
    let rendered = NativeBinderRenderer
        .render(sample_plan().await)
        .expect_render()
        .await;

    // 1 cover + 1 contents + 2 section separators + 3 document separators
    // + (3 + 2 + 1) document pages.
    assert_eq!(rendered.page_count, 13);
    assert_eq!(
        Document::load_mem(&rendered.bytes)
            .expect("parses")
            .get_pages()
            .len(),
        13
    );
}

#[tokio::test]
async fn contents_page_numbers_match_where_content_actually_lands() {
    let rendered = NativeBinderRenderer
        .render(sample_plan().await)
        .expect_render()
        .await;

    let by_title = |title: &str| {
        rendered
            .placements
            .iter()
            .find(|placement| placement.title == title)
            .unwrap_or_else(|| panic!("{title} is missing from the layout"))
            .clone()
    };

    // Cover is 1, contents is 2, so the first separator is 3.
    let policies = by_title("Policies");
    assert_eq!(policies.page_start, 3);
    assert!(policies.is_section);

    // Retention starts at its own separator on 4 and occupies it plus three
    // content pages.
    let retention = by_title("Retention Policy");
    assert_eq!(retention.page_start, 4);
    assert_eq!(retention.page_count, 4);

    let access = by_title("Access Policy");
    assert_eq!(access.page_start, 8);
    assert_eq!(access.page_count, 3);

    // Second section separator, then the last document's separator.
    assert_eq!(by_title("Board Minutes").page_start, 11);
    assert_eq!(by_title("January Minutes").page_start, 12);

    // The layout must never claim a page beyond the end of the file.
    for placement in &rendered.placements {
        assert!(
            placement.page_start + placement.page_count <= rendered.page_count + 1,
            "{} runs past the end of the binder",
            placement.title
        );
    }
}

#[tokio::test]
async fn every_section_gets_its_own_full_page_separator() {
    let plan = sample_plan().await;
    let with_separators = NativeBinderRenderer
        .render(plan.clone())
        .expect_render()
        .await;

    let without = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                include_separators: false,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    // Two sections, so exactly two pages differ.
    assert_eq!(with_separators.page_count - without.page_count, 2);

    for placement in &without.placements {
        if placement.is_section {
            assert_eq!(placement.page_count, 0, "a separator was emitted anyway");
        }
    }
}

#[tokio::test]
async fn every_document_gets_its_own_separator_page_by_default() {
    let plan = sample_plan().await;
    let with_document_separators = NativeBinderRenderer
        .render(plan.clone())
        .expect_render()
        .await;

    let without = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                document_separators: false,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    // Three documents, so exactly three pages differ.
    assert_eq!(with_document_separators.page_count - without.page_count, 3);

    // Without them, a document's entry occupies only its own content pages and
    // points straight at the first of them.
    let retention = without
        .placements
        .iter()
        .find(|placement| placement.title == "Retention Policy")
        .expect("retention is placed");
    assert_eq!(retention.page_count, 3);
    assert_eq!(retention.page_start, 4);
}

#[tokio::test]
async fn the_cover_and_contents_can_be_turned_off() {
    let plan = sample_plan().await;
    let bare = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                include_cover: false,
                include_toc: false,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    // 2 section separators + 3 document separators + 6 document pages.
    assert_eq!(bare.page_count, 11);
    // The first section now starts on page one.
    assert_eq!(bare.placements[0].page_start, 1);
}

#[tokio::test]
async fn duplex_padding_puts_every_separator_on_a_right_hand_page() {
    let plan = sample_plan().await;
    let duplex = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                duplex_blank_pages: true,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    // Section and document separators alike: every entry begins on a
    // right-hand page, or its separator would print on the back of a sheet.
    for placement in &duplex.placements {
        assert_eq!(
            placement.page_start % 2,
            1,
            "{} starts on a left-hand page, so it would print on the back of a sheet",
            placement.title
        );
    }

    assert_eq!(
        Document::load_mem(&duplex.bytes)
            .expect("parses")
            .get_pages()
            .len(),
        duplex.page_count as usize
    );
}

#[tokio::test]
async fn the_output_carries_a_bookmark_outline() {
    let rendered = NativeBinderRenderer
        .render(sample_plan().await)
        .expect_render()
        .await;
    let document = Document::load_mem(&rendered.bytes).expect("parses");

    let catalog = document.catalog().expect("a catalog");
    let outlines = catalog.get(b"Outlines").expect("an outline reference");
    let outline_id = outlines.as_reference().expect("an indirect reference");
    let outline = document
        .get_object(outline_id)
        .and_then(Object::as_dict)
        .expect("the outline resolves");

    // Two top-level sections.
    assert_eq!(outline.get(b"Count").and_then(Object::as_i64).ok(), Some(2));
    assert!(outline.get(b"First").is_ok());
    assert!(outline.get(b"Last").is_ok());

    // The reader should open showing them.
    assert_eq!(
        catalog
            .get(b"PageMode")
            .and_then(Object::as_name)
            .ok()
            .map(<[u8]>::to_vec),
        Some(b"UseOutlines".to_vec())
    );
}

#[tokio::test]
async fn documents_are_nested_under_their_section_in_the_outline() {
    let rendered = NativeBinderRenderer
        .render(sample_plan().await)
        .expect_render()
        .await;
    let document = Document::load_mem(&rendered.bytes).expect("parses");

    let outline_id = document
        .catalog()
        .expect("a catalog")
        .get(b"Outlines")
        .and_then(Object::as_reference)
        .expect("an outline");
    let outline = document
        .get_object(outline_id)
        .and_then(Object::as_dict)
        .expect("resolves");

    let first_id = outline
        .get(b"First")
        .and_then(Object::as_reference)
        .expect("a first child");
    let first = document
        .get_object(first_id)
        .and_then(Object::as_dict)
        .expect("resolves");

    // "Policies" holds two documents.
    assert_eq!(first.get(b"Count").and_then(Object::as_i64).ok(), Some(2));
    assert!(
        first.get(b"First").is_ok(),
        "the section should have children"
    );
}

#[tokio::test]
async fn rendering_is_deterministic() {
    let plan = sample_plan().await;
    let first = NativeBinderRenderer
        .render(plan.clone())
        .expect_render()
        .await;
    let second = NativeBinderRenderer.render(plan).expect_render().await;

    // Without this the output checksum recorded against a release would be
    // meaningless, and a rebuild could never be compared to the original.
    assert_eq!(
        first.bytes, second.bytes,
        "two builds of the same plan produced different bytes"
    );
}

#[tokio::test]
async fn changing_the_content_changes_the_output() {
    let plan = sample_plan().await;
    let baseline = NativeBinderRenderer
        .render(plan.clone())
        .expect_render()
        .await;

    let mut retitled = plan;
    retitled.cover.title = "Governance Binder, Second Edition".to_owned();
    let changed = NativeBinderRenderer.render(retitled).expect_render().await;

    assert_ne!(baseline.bytes, changed.bytes);
}

#[tokio::test]
async fn page_numbering_can_be_switched_off_without_changing_pagination() {
    let plan = sample_plan().await;
    let numbered = NativeBinderRenderer
        .render(plan.clone())
        .expect_render()
        .await;
    let unnumbered = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                page_numbering: PageNumbering::None,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    assert_eq!(numbered.page_count, unnumbered.page_count);
    // Stamping adds content, so the files must differ.
    assert_ne!(numbered.bytes, unnumbered.bytes);
    assert!(Document::load_mem(&unnumbered.bytes).is_ok());
}

#[tokio::test]
async fn letter_paper_produces_a_letter_sized_page() {
    let plan = sample_plan().await;
    let rendered = NativeBinderRenderer
        .render(BinderPlan {
            settings: BinderSettings {
                page_size: PageSize::Letter,
                ..plan.settings.clone()
            },
            ..plan
        })
        .expect_render()
        .await;

    let document = Document::load_mem(&rendered.bytes).expect("parses");
    let (_, first_page) = document
        .get_pages()
        .into_iter()
        .next()
        .expect("at least one page");
    let page = document
        .get_object(first_page)
        .and_then(Object::as_dict)
        .expect("resolves");
    let media_box = page
        .get(b"MediaBox")
        .and_then(Object::as_array)
        .expect("a media box");

    let width = media_box[2].as_float().expect("a number");
    let height = media_box[3].as_float().expect("a number");
    assert!((width - 612.0).abs() < 0.5, "width was {width}");
    assert!((height - 792.0).abs() < 0.5, "height was {height}");
}

#[tokio::test]
async fn a_title_needing_escaping_does_not_corrupt_the_output() {
    let mut plan = sample_plan().await;
    // Parentheses and a backslash would terminate a PDF literal string early and
    // turn the rest of the title into operators.
    plan.cover.title = r"Policies (2026) \ Final".to_owned();
    plan.entries[0] = PlanEntry::Section {
        level: 0,
        title: r"Section (a) \ subsection".to_owned(),
        path: vec!["Root (top)".to_owned()],
    };

    let rendered = NativeBinderRenderer.render(plan).expect_render().await;
    let document = Document::load_mem(&rendered.bytes).expect("output still parses");
    assert_eq!(document.get_pages().len(), rendered.page_count as usize);
}

#[tokio::test]
async fn an_empty_plan_is_refused() {
    let error = NativeBinderRenderer
        .render(BinderPlan {
            cover: CoverSpec::default(),
            settings: BinderSettings::default(),
            entries: Vec::new(),
            built_at: built_at(),
        })
        .await
        .expect_err("an empty binder has nothing to build");
    assert!(matches!(error, RenderError::EmptyPlan));
}

#[tokio::test]
async fn an_unreadable_source_names_the_document() {
    let error = NativeBinderRenderer
        .render(BinderPlan {
            cover: CoverSpec::default(),
            settings: BinderSettings::default(),
            entries: vec![PlanEntry::Document {
                level: 0,
                title: "Broken Attachment".to_owned(),
                path: Vec::new(),
                pdf: b"this is not a pdf".to_vec(),
            }],
            built_at: built_at(),
        })
        .await
        .expect_err("a corrupt source cannot be merged");

    match error {
        RenderError::UnreadableSource { title } => assert_eq!(title, "Broken Attachment"),
        other => panic!("expected an unreadable-source error, got {other}"),
    }
}

#[tokio::test]
async fn a_long_contents_list_spills_onto_further_pages() {
    let mut entries = Vec::new();
    for index in 0..80 {
        entries.push(PlanEntry::Section {
            level: 0,
            title: format!("Section {index}"),
            path: Vec::new(),
        });
    }

    let rendered = NativeBinderRenderer
        .render(BinderPlan {
            cover: CoverSpec {
                title: "Large Binder".to_owned(),
                ..CoverSpec::default()
            },
            settings: BinderSettings::default(),
            entries,
            built_at: built_at(),
        })
        .expect_render()
        .await;

    // 1 cover + n contents pages + 80 separators.
    assert!(
        rendered.page_count > 81,
        "the contents did not spill: {} pages",
        rendered.page_count
    );
    assert_eq!(
        Document::load_mem(&rendered.bytes)
            .expect("parses")
            .get_pages()
            .len(),
        rendered.page_count as usize
    );

    // The first separator must start after the contents, not on top of it.
    let first = &rendered.placements[0];
    assert!(
        first.page_start > 2,
        "first section landed on page {}",
        first.page_start
    );
}

#[tokio::test]
async fn merged_source_pages_survive_intact() {
    let source = source_pdf("attachment", 4).await;
    let rendered = NativeBinderRenderer
        .render(BinderPlan {
            cover: CoverSpec::default(),
            settings: BinderSettings {
                include_cover: false,
                include_toc: false,
                include_separators: false,
                document_separators: false,
                ..BinderSettings::default()
            },
            entries: vec![PlanEntry::Document {
                level: 0,
                title: "Attachment".to_owned(),
                path: Vec::new(),
                pdf: source,
            }],
            built_at: built_at(),
        })
        .expect_render()
        .await;

    assert_eq!(rendered.page_count, 4);

    // Every page must resolve, carry a media box, and hang off the merged tree.
    let document = Document::load_mem(&rendered.bytes).expect("parses");
    for (number, page_id) in document.get_pages() {
        let page = document
            .get_object(page_id)
            .and_then(Object::as_dict)
            .unwrap_or_else(|_| panic!("page {number} does not resolve"));
        assert!(
            page.get(b"MediaBox").is_ok(),
            "page {number} lost its media box"
        );
        assert!(
            page.get(b"Parent").is_ok(),
            "page {number} was not reparented"
        );
    }
}
