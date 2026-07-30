//! Binder PDF rendering.
//!
//! Generates the cover, the full-page section separators, and the table of
//! contents, then merges the source document PDFs behind them and applies the
//! outline, page numbers, and metadata.
//!
//! Rendering is done natively rather than by delegating the merge to an external
//! service. Two reasons: a binder must be buildable with no other process
//! running, and the outline and page-label work has to happen after the merge
//! anyway, so a round trip would only add a failure mode.
//!
//! Output is deterministic. Nothing reads the clock, object ids are assigned in a
//! fixed order, and the document identifier is derived from the content, so
//! rebuilding a release reproduces its recorded checksum.

mod text;

use std::collections::BTreeMap;

use async_trait::async_trait;
use elrond_application::ports::{
    BinderPlan, BinderRenderer, PageNumbering, PageSize, PlacedEntry, PlanEntry, RenderError,
    RenderedBinder,
};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use self::text::{Face, escape_literal, to_win_ansi, width_of, wrap};

/// Margin around generated pages, in points.
const MARGIN: f32 = 56.0;

/// Baseline distance from the foot of the page for a page number.
const FOOTER_BASELINE: f32 = 32.0;

/// Renders binders with `lopdf`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeBinderRenderer;

#[async_trait]
impl BinderRenderer for NativeBinderRenderer {
    async fn render(&self, plan: BinderPlan) -> Result<RenderedBinder, RenderError> {
        // PDF assembly is CPU-bound and can take seconds for a large binder, so it
        // runs on the blocking pool rather than stalling an async worker.
        tokio::task::spawn_blocking(move || build(&plan))
            .await
            .map_err(RenderError::backend)?
    }
}

/// One page destined for the output, before assembly.
struct PendingPage {
    /// The page dictionary.
    dictionary: Dictionary,
    /// Objects this page depends on, already carrying final ids.
    objects: Vec<(ObjectId, Object)>,
}

/// Registers a generated page and returns its id.
fn push_page(document: &mut Document, pages: &mut Vec<ObjectId>, page: PendingPage) -> ObjectId {
    for (id, object) in page.objects {
        document.objects.insert(id, object);
    }
    let id = document.add_object(Object::Dictionary(page.dictionary));
    pages.push(id);
    id
}

/// Assembles the whole binder.
fn build(plan: &BinderPlan) -> Result<RenderedBinder, RenderError> {
    if plan.entries.is_empty() {
        return Err(RenderError::EmptyPlan);
    }

    let (width, height) = plan.settings.page_size.points();
    let mut document = Document::with_version("1.7");

    // Fonts are shared by every generated page. Registered first so their ids are
    // stable regardless of the binder's contents.
    let regular = document.add_object(font_dictionary(Face::Regular));
    let bold = document.add_object(font_dictionary(Face::Bold));

    // ---------------------------------------------------------------- layout
    //
    // Laid out before anything is rendered, because the contents page has to know
    // the page number of every entry, and those numbers depend on how many pages
    // the contents itself occupies.
    let layout = plan_layout(plan)?;

    let mut pages: Vec<ObjectId> = Vec::new();
    let mut outline: Vec<(u8, String, usize)> = Vec::new();

    if plan.settings.include_cover {
        let page = cover_page(&mut document, plan, width, height, regular, bold);
        push_page(&mut document, &mut pages, page);
    }

    if plan.settings.include_toc {
        for contents in contents_pages(&mut document, plan, &layout, width, height, regular, bold) {
            push_page(&mut document, &mut pages, contents);
        }
    }

    // ------------------------------------------------------------- body pages
    for entry in &plan.entries {
        match entry {
            PlanEntry::Section { level, title, path } => {
                outline.push((*level, title.clone(), pages.len()));

                if plan.settings.include_separators {
                    // Pad so a separator always falls on a right-hand page when the
                    // binder is printed double-sided.
                    if plan.settings.duplex_blank_pages && pages.len() % 2 == 1 {
                        let blank = blank_page(width, height);
                        push_page(&mut document, &mut pages, blank);
                    }
                    let separator =
                        separator_page(&mut document, title, path, width, height, regular, bold);
                    push_page(&mut document, &mut pages, separator);
                }
            }
            PlanEntry::Document { level, title, pdf } => {
                outline.push((*level, title.clone(), pages.len()));
                merge_source(&mut document, &mut pages, title, pdf)?;
            }
        }
    }

    // -------------------------------------------------------------- assembly
    let pages_id = document.new_object_id();
    for page in &pages {
        if let Ok(Object::Dictionary(dictionary)) = document.get_object_mut(*page) {
            dictionary.set("Parent", pages_id);
        }
    }

    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => pages.iter().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
            "Count" => i64::from(page_count),
        }),
    );

    if plan.settings.page_numbering == PageNumbering::Continuous {
        stamp_page_numbers(&mut document, &pages, width, regular);
    }

    let outline_id = build_outline(&mut document, &pages, &outline);

    let mut catalog = dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    };
    if let Some(outline_id) = outline_id {
        catalog.set("Outlines", outline_id);
        // Opens the bookmark panel, which is how a reader navigates a binder.
        catalog.set("PageMode", Object::Name(b"UseOutlines".to_vec()));
    }
    let catalog_id = document.add_object(Object::Dictionary(catalog));

    let info_id = document.add_object(Object::Dictionary(dictionary! {
        "Title" => Object::string_literal(plan.cover.title.clone()),
        "Producer" => Object::string_literal("Elrond"),
        "CreationDate" => Object::string_literal(pdf_date(plan.built_at)),
        "ModDate" => Object::string_literal(pdf_date(plan.built_at)),
    }));

    document.trailer.set("Root", catalog_id);
    document.trailer.set("Info", info_id);
    document.compress();

    // A fixed identifier keeps two builds of the same release byte-identical; a
    // random one would defeat the recorded output checksum.
    let identifier = Object::string_literal(stable_identifier(plan));
    document
        .trailer
        .set("ID", Object::Array(vec![identifier.clone(), identifier]));

    let mut bytes = Vec::new();
    document.save_to(&mut bytes).map_err(RenderError::backend)?;

    Ok(RenderedBinder {
        bytes,
        page_count,
        placements: layout,
    })
}

/// Works out where every entry will land, before any page is rendered.
fn plan_layout(plan: &BinderPlan) -> Result<Vec<PlacedEntry>, RenderError> {
    // Source page counts are needed up front, so each PDF is parsed once here and
    // once again during the merge. Parsing is cheap next to rendering, and the
    // alternative is holding every parsed document in memory for the whole build.
    let mut counts: Vec<u32> = Vec::new();
    for entry in &plan.entries {
        if let PlanEntry::Document { title, pdf, .. } = entry {
            let source = Document::load_mem(pdf).map_err(|_| RenderError::UnreadableSource {
                title: title.clone(),
            })?;
            let count = u32::try_from(source.get_pages().len()).unwrap_or(0);
            if count == 0 {
                return Err(RenderError::EmptySource {
                    title: title.clone(),
                });
            }
            counts.push(count);
        }
    }

    // The contents page count depends on how many entries there are, which does
    // not depend on the contents page. One pass is enough.
    let front_matter = u32::from(plan.settings.include_cover)
        + if plan.settings.include_toc {
            contents_page_count(plan)
        } else {
            0
        };

    let mut placements = Vec::new();
    let mut cursor = front_matter;
    let mut document_index = 0;

    for entry in &plan.entries {
        match entry {
            PlanEntry::Section { level, title, .. } => {
                if plan.settings.include_separators
                    && plan.settings.duplex_blank_pages
                    && cursor % 2 == 1
                {
                    cursor += 1;
                }
                let pages = u32::from(plan.settings.include_separators);
                placements.push(PlacedEntry {
                    title: title.clone(),
                    level: *level,
                    is_section: true,
                    page_start: cursor + 1,
                    page_count: pages,
                });
                cursor += pages;
            }
            PlanEntry::Document { level, title, .. } => {
                let pages = counts.get(document_index).copied().unwrap_or(0);
                document_index += 1;
                placements.push(PlacedEntry {
                    title: title.clone(),
                    level: *level,
                    is_section: false,
                    page_start: cursor + 1,
                    page_count: pages,
                });
                cursor += pages;
            }
        }
    }

    Ok(placements)
}

/// How many pages the table of contents needs.
fn contents_page_count(plan: &BinderPlan) -> u32 {
    let rows = u32::try_from(plan.entries.len()).unwrap_or(u32::MAX);
    let per_page = contents_rows_per_page(plan.settings.page_size);
    rows.div_ceil(per_page).max(1)
}

/// Contents rows that fit on one page.
fn contents_rows_per_page(size: PageSize) -> u32 {
    let (_, height) = size.points();
    // Heading plus its gap eats the top of the first page; using the same figure
    // for every page keeps the count simple and errs toward more pages, never
    // fewer, so entries can never overflow.
    let mut remaining = height - MARGIN * 2.0 - 60.0;
    let mut rows: u32 = 0;

    // Counted rather than divided-and-cast: a float-to-integer cast here would
    // need a truncation waiver, and the loop is bounded by the page height.
    while remaining >= CONTENTS_ROW_HEIGHT {
        remaining -= CONTENTS_ROW_HEIGHT;
        rows += 1;
    }
    rows.max(1)
}

/// Vertical space one contents row occupies.
const CONTENTS_ROW_HEIGHT: f32 = 20.0;

/// Builds the font resource dictionary for a face.
fn font_dictionary(face: Face) -> Object {
    Object::Dictionary(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => Object::Name(face.base_font().as_bytes().to_vec()),
        // Without this the viewer assumes StandardEncoding and accented Latin
        // characters render as the wrong glyph.
        "Encoding" => Object::Name(b"WinAnsiEncoding".to_vec()),
    })
}

/// Resources naming both fonts.
fn resources(regular: ObjectId, bold: ObjectId) -> Dictionary {
    dictionary! {
        "Font" => dictionary! {
            Face::Regular.resource() => regular,
            Face::Bold.resource() => bold,
        },
    }
}

/// Builds a page from a content stream.
fn page_from(
    document: &mut Document,
    operations: Vec<Operation>,
    width: f32,
    height: f32,
    regular: ObjectId,
    bold: ObjectId,
) -> PendingPage {
    let content = Content { operations };
    let stream = Stream::new(Dictionary::new(), content.encode().unwrap_or_default());
    let stream_id = document.new_object_id();

    PendingPage {
        dictionary: dictionary! {
            "Type" => "Page",
            "MediaBox" => media_box(width, height),
            "Resources" => resources(regular, bold),
            "Contents" => stream_id,
        },
        objects: vec![(stream_id, Object::Stream(stream))],
    }
}

/// A media box covering the whole page.
fn media_box(width: f32, height: f32) -> Object {
    Object::Array(vec![
        0.into(),
        0.into(),
        Object::Real(width),
        Object::Real(height),
    ])
}

/// An empty page, used for duplex padding.
fn blank_page(width: f32, height: f32) -> PendingPage {
    PendingPage {
        dictionary: dictionary! {
            "Type" => "Page",
            "MediaBox" => media_box(width, height),
            "Resources" => Dictionary::new(),
        },
        objects: Vec::new(),
    }
}

/// Emits operations drawing one line of text at a baseline.
fn draw_text(operations: &mut Vec<Operation>, text: &str, face: Face, size: f32, x: f32, y: f32) {
    let encoded = escape_literal(&to_win_ansi(text));
    operations.push(Operation::new("BT", vec![]));
    operations.push(Operation::new(
        "Tf",
        vec![
            Object::Name(face.resource().as_bytes().to_vec()),
            size.into(),
        ],
    ));
    operations.push(Operation::new("Td", vec![Object::Real(x), Object::Real(y)]));
    operations.push(Operation::new(
        "Tj",
        vec![Object::String(encoded, lopdf::StringFormat::Literal)],
    ));
    operations.push(Operation::new("ET", vec![]));
}

/// Emits operations drawing text centred on the page.
fn draw_centred(
    operations: &mut Vec<Operation>,
    text: &str,
    face: Face,
    size: f32,
    page_width: f32,
    y: f32,
) {
    let x = (page_width - width_of(text, face, size)) / 2.0;
    draw_text(operations, text, face, size, x.max(MARGIN), y);
}

/// Emits a horizontal rule.
fn draw_rule(operations: &mut Vec<Operation>, x0: f32, x1: f32, y: f32, thickness: f32) {
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new("w", vec![Object::Real(thickness)]));
    operations.push(Operation::new("m", vec![Object::Real(x0), Object::Real(y)]));
    operations.push(Operation::new("l", vec![Object::Real(x1), Object::Real(y)]));
    operations.push(Operation::new("S", vec![]));
    operations.push(Operation::new("Q", vec![]));
}

/// The front cover.
fn cover_page(
    document: &mut Document,
    plan: &BinderPlan,
    width: f32,
    height: f32,
    regular: ObjectId,
    bold: ObjectId,
) -> PendingPage {
    let mut operations = Vec::new();
    let measure = width - MARGIN * 2.0;

    if let Some(organization) = &plan.cover.organization {
        draw_centred(
            &mut operations,
            organization,
            Face::Regular,
            11.0,
            width,
            height - MARGIN - 40.0,
        );
    }

    // The title block sits above the optical centre, which reads better than true
    // centring on a portrait page.
    let title_lines = wrap(&plan.cover.title, Face::Bold, 30.0, measure);
    let mut y = height * 0.62;
    for line in &title_lines {
        draw_centred(&mut operations, line, Face::Bold, 30.0, width, y);
        y -= 38.0;
    }

    draw_rule(&mut operations, width * 0.3, width * 0.7, y - 6.0, 1.0);
    y -= 34.0;

    if let Some(subtitle) = &plan.cover.subtitle {
        for line in wrap(subtitle, Face::Regular, 14.0, measure) {
            draw_centred(&mut operations, &line, Face::Regular, 14.0, width, y);
            y -= 20.0;
        }
    }

    let mut footer = height * 0.16;
    if let Some(release) = &plan.cover.release_label {
        draw_centred(&mut operations, release, Face::Bold, 11.0, width, footer);
        footer -= 16.0;
    }
    if let Some(built_on) = &plan.cover.built_on {
        draw_centred(
            &mut operations,
            built_on,
            Face::Regular,
            10.0,
            width,
            footer,
        );
    }

    page_from(document, operations, width, height, regular, bold)
}

/// A full-page separator announcing a section.
fn separator_page(
    document: &mut Document,
    title: &str,
    path: &[String],
    width: f32,
    height: f32,
    regular: ObjectId,
    bold: ObjectId,
) -> PendingPage {
    let mut operations = Vec::new();
    let measure = width - MARGIN * 2.0;

    // The ancestor path is shown so a reader who opens the binder at a separator
    // knows where they are in a nested structure, not just the leaf name.
    if !path.is_empty() {
        draw_centred(
            &mut operations,
            &path.join("   ·   "),
            Face::Regular,
            10.0,
            width,
            height * 0.58,
        );
    }

    let lines = wrap(title, Face::Bold, 26.0, measure);
    #[expect(clippy::cast_precision_loss, reason = "a title has few lines")]
    let block = lines.len() as f32;
    let mut y = height * 0.5 + (block - 1.0) * 16.0;
    for line in &lines {
        draw_centred(&mut operations, line, Face::Bold, 26.0, width, y);
        y -= 32.0;
    }

    draw_rule(&mut operations, width * 0.35, width * 0.65, y + 8.0, 1.5);

    page_from(document, operations, width, height, regular, bold)
}

/// The table of contents, across as many pages as it needs.
fn contents_pages(
    document: &mut Document,
    plan: &BinderPlan,
    layout: &[PlacedEntry],
    width: f32,
    height: f32,
    regular: ObjectId,
    bold: ObjectId,
) -> Vec<PendingPage> {
    let per_page = contents_rows_per_page(plan.settings.page_size) as usize;
    let mut built = Vec::new();

    for (index, chunk) in layout.chunks(per_page.max(1)).enumerate() {
        let mut operations = Vec::new();
        let mut y = height - MARGIN;

        if index == 0 {
            draw_text(
                &mut operations,
                "Contents",
                Face::Bold,
                20.0,
                MARGIN,
                y - 20.0,
            );
            draw_rule(&mut operations, MARGIN, width - MARGIN, y - 32.0, 1.0);
            y -= 60.0;
        } else {
            draw_text(
                &mut operations,
                "Contents (continued)",
                Face::Regular,
                10.0,
                MARGIN,
                y - 12.0,
            );
            y -= 34.0;
        }

        for entry in chunk {
            let face = if entry.is_section {
                Face::Bold
            } else {
                Face::Regular
            };
            let size = if entry.is_section { 11.5 } else { 10.5 };
            let indent = MARGIN + f32::from(entry.level) * 16.0;
            let number = entry.page_start.to_string();
            let number_width = width_of(&number, Face::Regular, 10.0);
            let number_x = width - MARGIN - number_width;

            // Truncate rather than wrap, so every row is one line and the dotted
            // leader always reaches the page number.
            let available = number_x - indent - 18.0;
            let label = truncate(&entry.title, face, size, available);

            draw_text(&mut operations, &label, face, size, indent, y);

            let label_end = indent + width_of(&label, face, size);
            if number_x - label_end > 12.0 {
                draw_leader(&mut operations, label_end + 4.0, number_x - 4.0, y + 3.0);
            }
            draw_text(&mut operations, &number, Face::Regular, 10.0, number_x, y);

            y -= 20.0;
        }

        built.push(page_from(
            document, operations, width, height, regular, bold,
        ));
    }

    if built.is_empty() {
        built.push(page_from(
            document,
            Vec::new(),
            width,
            height,
            regular,
            bold,
        ));
    }
    built
}

/// A dotted leader between a contents entry and its page number.
fn draw_leader(operations: &mut Vec<Operation>, x0: f32, x1: f32, y: f32) {
    operations.push(Operation::new("q", vec![]));
    operations.push(Operation::new("w", vec![Object::Real(0.5)]));
    operations.push(Operation::new(
        "d",
        vec![
            Object::Array(vec![Object::Real(0.5), Object::Real(3.0)]),
            0.into(),
        ],
    ));
    operations.push(Operation::new("m", vec![Object::Real(x0), Object::Real(y)]));
    operations.push(Operation::new("l", vec![Object::Real(x1), Object::Real(y)]));
    operations.push(Operation::new("S", vec![]));
    operations.push(Operation::new("Q", vec![]));
}

/// Shortens text with an ellipsis until it fits.
fn truncate(value: &str, face: Face, size: f32, max_width: f32) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    if width_of(value, face, size) <= max_width {
        return value.to_owned();
    }

    let mut shortened = String::new();
    for character in value.chars() {
        let mut probe = shortened.clone();
        probe.push(character);
        if width_of(&format!("{probe}\u{2026}"), face, size) > max_width {
            break;
        }
        shortened = probe;
    }
    format!("{}\u{2026}", shortened.trim_end())
}

/// Copies a source PDF's pages into the output.
fn merge_source(
    document: &mut Document,
    pages: &mut Vec<ObjectId>,
    title: &str,
    pdf: &[u8],
) -> Result<(), RenderError> {
    let mut source = Document::load_mem(pdf).map_err(|_| RenderError::UnreadableSource {
        title: title.to_owned(),
    })?;

    // Shift the source's object ids clear of everything already in the output, so
    // the two documents' numbering cannot collide.
    source.renumber_objects_with(document.max_id + 1);

    let source_pages = source.get_pages();
    if source_pages.is_empty() {
        return Err(RenderError::EmptySource {
            title: title.to_owned(),
        });
    }
    let page_ids: Vec<ObjectId> = source_pages.values().copied().collect();

    // The source's own Pages and Catalog nodes are dropped; its pages are
    // reparented onto the output's single page tree.
    for (id, object) in source.objects {
        let kind = object
            .as_dict()
            .ok()
            .and_then(|dictionary| dictionary.get(b"Type").ok())
            .and_then(|value| value.as_name().ok())
            .map(<[u8]>::to_vec);

        match kind.as_deref() {
            Some(b"Pages" | b"Catalog") => {}
            _ => {
                document.objects.insert(id, object);
            }
        }
    }

    document.max_id = document.max_id.max(source.max_id);
    pages.extend(page_ids);
    Ok(())
}

/// Stamps a page number onto the foot of every page.
fn stamp_page_numbers(document: &mut Document, pages: &[ObjectId], width: f32, font: ObjectId) {
    for (index, page_id) in pages.iter().enumerate() {
        let number = index + 1;
        let label = number.to_string();
        let x = (width - width_of(&label, Face::Regular, 9.0)) / 2.0;

        let mut operations = Vec::new();
        draw_text(
            &mut operations,
            &label,
            Face::Regular,
            9.0,
            x,
            FOOTER_BASELINE,
        );
        let stamp = Stream::new(
            Dictionary::new(),
            Content { operations }.encode().unwrap_or_default(),
        );
        let stamp_id = document.add_object(Object::Stream(stamp));

        // A merged page's own stream may leave the graphics state unbalanced, so
        // the original content is bracketed before anything is appended. Without
        // this, a source that opens a `q` without closing it would drag the stamp
        // into its transform.
        let open = document.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            b"q\n".to_vec(),
        )));
        let close = document.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            b"Q\n".to_vec(),
        )));

        ensure_font(document, *page_id, font);

        let Ok(Object::Dictionary(page)) = document.get_object_mut(*page_id) else {
            continue;
        };
        let existing = page.get(b"Contents").ok().cloned();
        let mut contents = vec![Object::Reference(open)];
        match existing {
            Some(Object::Array(items)) => contents.extend(items),
            Some(item @ Object::Reference(_)) => contents.push(item),
            // A page with no content is a legitimate blank; it still gets a number.
            _ => {}
        }
        contents.push(Object::Reference(close));
        contents.push(Object::Reference(stamp_id));
        page.set("Contents", Object::Array(contents));
    }
}

/// Makes sure a page's resources name the footer font.
///
/// A merged page carries its own resource dictionary, which will not know about
/// Elrond's font, and a `Tf` naming an unlisted font is undefined behaviour.
fn ensure_font(document: &mut Document, page_id: ObjectId, font: ObjectId) {
    // Resources may be inline on the page or an indirect object shared between
    // pages; both have to be handled, and a shared one must not be mutated in a
    // way that surprises the pages sharing it. Adding one more font entry is
    // additive, so sharing is safe here.
    let resources_ref = document
        .get_object(page_id)
        .ok()
        .and_then(|object| object.as_dict().ok())
        .and_then(|page| page.get(b"Resources").ok())
        .cloned();

    match resources_ref {
        Some(Object::Reference(id)) => {
            if let Ok(Object::Dictionary(resources)) = document.get_object_mut(id) {
                add_font_entry(resources, font);
            }
        }
        Some(Object::Dictionary(mut resources)) => {
            add_font_entry(&mut resources, font);
            if let Ok(Object::Dictionary(page)) = document.get_object_mut(page_id) {
                page.set("Resources", Object::Dictionary(resources));
            }
        }
        _ => {
            let mut resources = Dictionary::new();
            add_font_entry(&mut resources, font);
            if let Ok(Object::Dictionary(page)) = document.get_object_mut(page_id) {
                page.set("Resources", Object::Dictionary(resources));
            }
        }
    }
}

/// Adds Elrond's regular font to a resource dictionary, keeping existing fonts.
fn add_font_entry(resources: &mut Dictionary, font: ObjectId) {
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(existing)) => existing.clone(),
        _ => Dictionary::new(),
    };
    fonts.set(Face::Regular.resource(), font);
    resources.set("Font", Object::Dictionary(fonts));
}

/// Builds the bookmark outline.
///
/// Written directly rather than through the helper API because the nesting has to
/// follow the binder's own section levels, and every destination has to resolve to
/// a page that exists in the merged output.
fn build_outline(
    document: &mut Document,
    pages: &[ObjectId],
    entries: &[(u8, String, usize)],
) -> Option<ObjectId> {
    if entries.is_empty() {
        return None;
    }

    let outline_id = document.new_object_id();
    let item_ids: Vec<ObjectId> = entries.iter().map(|_| document.new_object_id()).collect();

    // Each entry's parent is the nearest preceding entry at a shallower level.
    let mut parents: Vec<Option<usize>> = Vec::with_capacity(entries.len());
    let mut stack: Vec<(u8, usize)> = Vec::new();
    for (index, (level, _, _)) in entries.iter().enumerate() {
        while stack.last().is_some_and(|(open, _)| *open >= *level) {
            stack.pop();
        }
        parents.push(stack.last().map(|(_, parent)| *parent));
        stack.push((*level, index));
    }

    // Siblings, grouped by parent, in document order.
    let mut children: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    for (index, parent) in parents.iter().enumerate() {
        children.entry(*parent).or_default().push(index);
    }

    for (index, (_, title, page_index)) in entries.iter().enumerate() {
        let siblings = children.get(&parents[index]).cloned().unwrap_or_default();
        let position = siblings.iter().position(|candidate| *candidate == index);

        let mut item = dictionary! {
            "Title" => Object::string_literal(title.clone()),
            "Parent" => parents[index].map_or(outline_id, |parent| item_ids[parent]),
        };

        // A destination past the end would make a viewer refuse the whole outline.
        if let Some(page) = pages.get(*page_index) {
            item.set(
                "Dest",
                Object::Array(vec![
                    Object::Reference(*page),
                    Object::Name(b"Fit".to_vec()),
                ]),
            );
        }

        if let Some(position) = position {
            if position > 0 {
                item.set("Prev", item_ids[siblings[position - 1]]);
            }
            if position + 1 < siblings.len() {
                item.set("Next", item_ids[siblings[position + 1]]);
            }
        }

        if let Some(own_children) = children.get(&Some(index))
            && let (Some(first), Some(last)) = (own_children.first(), own_children.last())
        {
            item.set("First", item_ids[*first]);
            item.set("Last", item_ids[*last]);
            // Positive means open; a binder is more useful with its structure
            // showing than collapsed.
            item.set("Count", i64::try_from(own_children.len()).unwrap_or(0));
        }

        document
            .objects
            .insert(item_ids[index], Object::Dictionary(item));
    }

    let roots = children.get(&None).cloned().unwrap_or_default();
    let mut outline = dictionary! { "Type" => "Outlines" };
    if let (Some(first), Some(last)) = (roots.first(), roots.last()) {
        outline.set("First", item_ids[*first]);
        outline.set("Last", item_ids[*last]);
        outline.set("Count", i64::try_from(roots.len()).unwrap_or(0));
    }
    document
        .objects
        .insert(outline_id, Object::Dictionary(outline));

    Some(outline_id)
}

/// Formats a timestamp as a PDF date string.
fn pdf_date(at: time::OffsetDateTime) -> String {
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// A document identifier derived from the plan, so rebuilds match.
fn stable_identifier(plan: &BinderPlan) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(plan.cover.title.as_bytes());
    hasher.update(pdf_date(plan.built_at).as_bytes());
    for entry in &plan.entries {
        match entry {
            PlanEntry::Section { level, title, .. } => {
                hasher.update([b'S', *level]);
                hasher.update(title.as_bytes());
            }
            PlanEntry::Document { level, title, pdf } => {
                hasher.update([b'D', *level]);
                hasher.update(title.as_bytes());
                hasher.update(Sha256::digest(pdf));
            }
        }
    }
    hex::encode(hasher.finalize())
}
