use std::collections::BTreeMap;

use deunicode::deunicode;
use elrond_application::{BinderError, BinderRenderer};
use elrond_domain::binders::PrintableBinderDocument;
use lopdf::{
    Document, LoadOptions, Object, ObjectId, Stream,
    content::{Content, Operation},
    dictionary,
};
use sha2::Digest;

pub struct LopdfBinderRenderer;

struct LoadedDocument {
    source: PrintableBinderDocument,
    pdf: Document,
    pages: usize,
    start_page: usize,
}

impl BinderRenderer for LopdfBinderRenderer {
    fn render(&self, documents: Vec<PrintableBinderDocument>) -> Result<Vec<u8>, BinderError> {
        let mut loaded = documents
            .into_iter()
            .map(|source| {
                let pdf = Document::load_mem_with_options(
                    &source.pdf_content,
                    LoadOptions::with_max_decompressed_size(32 * 1024 * 1024),
                )
                .map_err(|_| {
                    BinderError::Render(format!("{} is not a readable PDF", source.title))
                })?;
                let pages = pdf.get_pages().len();
                if pages == 0 {
                    return Err(BinderError::Render(format!(
                        "{} contains no pages",
                        source.title
                    )));
                }
                Ok(LoadedDocument {
                    source,
                    pdf,
                    pages,
                    start_page: 0,
                })
            })
            .collect::<Result<Vec<_>, BinderError>>()?;
        let index_rows = index_row_count(&loaded);
        let index_pages = index_rows.div_ceil(36).max(1);
        if binder_page_count(&loaded, index_pages) > 20_000 {
            return Err(BinderError::Render(
                "the binder exceeds the 20,000 page limit".into(),
            ));
        }
        let mut page = index_pages + 1;
        let mut previous_category = None;
        for document in &mut loaded {
            if previous_category != Some(document.source.category_path.as_str()) {
                page += 1;
                previous_category = Some(&document.source.category_path);
            }
            document.start_page = page;
            page += document.pages + 1;
        }

        let mut parts = Vec::new();
        let index_lines = index_lines(&loaded);
        for (index, lines) in index_lines.chunks(36).enumerate() {
            parts.push(text_page(
                if index == 0 {
                    "Binder Index"
                } else {
                    "Binder Index, continued"
                },
                lines,
            )?);
        }
        let mut previous_category: Option<String> = None;
        for document in loaded {
            if previous_category.as_deref() != Some(document.source.category_path.as_str()) {
                parts.push(text_page(&document.source.category_path, &[])?);
                previous_category = Some(document.source.category_path.clone());
            }
            parts.push(text_page(
                &document.source.title,
                &[
                    format!("Category: {}", document.source.category_path),
                    format!("Version: {}", document.source.version_number),
                ],
            )?);
            parts.push(document.pdf);
        }
        merge_documents(parts)
    }
}

fn binder_page_count(documents: &[LoadedDocument], index_pages: usize) -> usize {
    let category_pages = index_row_count(documents) - documents.len();
    index_pages
        + category_pages
        + documents.len()
        + documents
            .iter()
            .map(|document| document.pages)
            .sum::<usize>()
}

fn index_row_count(documents: &[LoadedDocument]) -> usize {
    let mut rows = documents.len();
    let mut previous = None;
    for document in documents {
        if previous != Some(document.source.category_path.as_str()) {
            rows += 1;
            previous = Some(&document.source.category_path);
        }
    }
    rows
}

fn index_lines(documents: &[LoadedDocument]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut previous = None;
    for document in documents {
        if previous != Some(document.source.category_path.as_str()) {
            lines.push(format!(
                "{} ................................ {}",
                document.source.category_path,
                document.start_page - 1
            ));
            previous = Some(&document.source.category_path);
        }
        let end_page = document.start_page + document.pages;
        let pages = if end_page == document.start_page {
            document.start_page.to_string()
        } else {
            format!("{}-{}", document.start_page, end_page)
        };
        lines.push(format!(
            "    {} (v{}) ................................ {}",
            document.source.title, document.source.version_number, pages
        ));
    }
    lines
}

fn text_page(title: &str, lines: &[String]) -> Result<Document, BinderError> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let regular_font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let bold_font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
    });
    let resources = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => regular_font, "F2" => bold_font },
    });
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F2".into(), 28.into()]),
        Operation::new("Td", vec![54.into(), 700.into()]),
        Operation::new("Tj", vec![Object::string_literal(clean_text(title, 70))]),
        Operation::new("ET", vec![]),
    ];
    for (index, line) in lines.iter().enumerate() {
        operations.extend([
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 10.into()]),
            Operation::new("Td", vec![54.into(), (660 - index as i64 * 17).into()]),
            Operation::new("Tj", vec![Object::string_literal(clean_text(line, 105))]),
            Operation::new("ET", vec![]),
        ]);
    }
    let content = Content { operations }
        .encode()
        .map_err(|error| BinderError::Render(error.to_string()))?;
    let content_id = document.add_object(Stream::new(dictionary! {}, content));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "Resources" => resources, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    Ok(document)
}

fn clean_text(value: &str, limit: usize) -> String {
    let mut cleaned: String = deunicode(value)
        .chars()
        .filter(|character| !character.is_ascii_control())
        .collect();
    if cleaned.is_empty() {
        cleaned = "Label".into();
    }
    let changed = cleaned != value || cleaned.chars().count() > limit;
    if !changed {
        return cleaned;
    }
    let suffix = &hex::encode(sha2::Sha256::digest(value.as_bytes()))[..6];
    let suffix = format!(" [{suffix}]");
    let available = limit.saturating_sub(suffix.len());
    let base = if cleaned.chars().count() > available {
        format!(
            "{}...",
            cleaned
                .chars()
                .take(available.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        cleaned
    };
    format!("{base}{suffix}")
}

fn merge_documents(documents: Vec<Document>) -> Result<Vec<u8>, BinderError> {
    let mut pages = Vec::new();
    let mut objects = BTreeMap::new();
    let mut max_id = 1;
    for mut source in documents {
        sanitize_annotation_actions(&mut source);
        source.renumber_objects_with(max_id);
        max_id = source.max_id + 1;
        for object_id in source.get_pages().into_values() {
            pages.push((object_id, flattened_page(&source, object_id)?));
        }
        objects.extend(source.objects);
    }

    let mut output = Document::with_version("1.7");
    let mut catalog: Option<(ObjectId, Object)> = None;
    let mut page_tree: Option<(ObjectId, Object)> = None;
    for (object_id, object) in objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                catalog.get_or_insert((object_id, object));
            }
            b"Pages" => {
                page_tree.get_or_insert((object_id, object));
            }
            b"Page" | b"Outlines" | b"Outline" => continue,
            _ => {
                output.objects.insert(object_id, object);
            }
        }
    }
    let (catalog_id, catalog_object) =
        catalog.ok_or_else(|| BinderError::Render("PDF catalog missing".into()))?;
    let (pages_id, pages_object) =
        page_tree.ok_or_else(|| BinderError::Render("PDF page tree missing".into()))?;
    for (object_id, object) in &pages {
        let mut dictionary = object
            .as_dict()
            .map_err(|error| BinderError::Render(error.to_string()))?
            .clone();
        dictionary.set("Parent", pages_id);
        output
            .objects
            .insert(*object_id, Object::Dictionary(dictionary));
    }
    let mut pages_dictionary = pages_object
        .as_dict()
        .map_err(|error| BinderError::Render(error.to_string()))?
        .clone();
    pages_dictionary.set("Count", pages.len() as u32);
    pages_dictionary.set(
        "Kids",
        pages
            .into_iter()
            .map(|(object_id, _)| Object::Reference(object_id))
            .collect::<Vec<_>>(),
    );
    output
        .objects
        .insert(pages_id, Object::Dictionary(pages_dictionary));

    let mut catalog_dictionary = catalog_object
        .as_dict()
        .map_err(|error| BinderError::Render(error.to_string()))?
        .clone();
    catalog_dictionary.set("Pages", pages_id);
    catalog_dictionary.remove(b"Outlines");
    output
        .objects
        .insert(catalog_id, Object::Dictionary(catalog_dictionary));
    output.trailer.set("Root", catalog_id);
    output.max_id = output.objects.len() as u32;
    output.renumber_objects();
    let mut bytes = Vec::new();
    output
        .save_to(&mut bytes)
        .map_err(|error| BinderError::Render(error.to_string()))?;
    Ok(bytes)
}

fn flattened_page(document: &Document, object_id: ObjectId) -> Result<Object, BinderError> {
    let mut page = document
        .get_object(object_id)
        .and_then(Object::as_dict)
        .map_err(|error| BinderError::Render(error.to_string()))?
        .clone();
    let mut parent = page
        .get(b"Parent")
        .ok()
        .and_then(|value| value.as_reference().ok());
    while let Some(parent_id) = parent {
        let ancestor = document
            .get_object(parent_id)
            .and_then(Object::as_dict)
            .map_err(|error| BinderError::Render(error.to_string()))?;
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if page.get(key).is_err()
                && let Ok(value) = ancestor.get(key)
            {
                page.set(key, value.clone());
            }
        }
        parent = ancestor
            .get(b"Parent")
            .ok()
            .and_then(|value| value.as_reference().ok());
    }
    page.remove(b"AA");
    Ok(Object::Dictionary(page))
}

fn sanitize_annotation_actions(document: &mut Document) {
    for object in document.objects.values_mut() {
        sanitize_active_object(object);
    }
}

fn sanitize_active_object(object: &mut Object) {
    match object {
        Object::Dictionary(dictionary) => {
            let object_type = dictionary
                .get(b"Type")
                .ok()
                .and_then(|value| value.as_name().ok());
            let is_annotation =
                dictionary.get(b"Subtype").is_ok() && dictionary.get(b"Rect").is_ok();
            let is_form_field = dictionary.get(b"FT").is_ok();
            if is_annotation || is_form_field {
                for key in [
                    b"A".as_slice(),
                    b"AA",
                    b"Activation",
                    b"3DA",
                    b"RichMediaSettings",
                ] {
                    dictionary.remove(key);
                }
            } else if object_type == Some(b"Page") {
                dictionary.remove(b"AA");
            } else if object_type == Some(b"Catalog") {
                dictionary.remove(b"AA");
                dictionary.remove(b"OpenAction");
            }
            for value in dictionary.iter_mut().map(|(_, value)| value) {
                sanitize_active_object(value);
            }
        }
        Object::Array(values) => {
            for value in values {
                sanitize_active_object(value);
            }
        }
        Object::Stream(_) => {}
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_index_category_and_document_separators_and_every_source_page() {
        let documents = vec![
            source("Policy A", "Policies", 2),
            source("Policy B", "Policies", 1),
        ];
        let bytes = LopdfBinderRenderer
            .render(documents)
            .expect("binder should render");
        let binder = Document::load_mem(&bytes).expect("binder should be readable");

        assert_eq!(binder.get_pages().len(), 7);
        assert_eq!(
            page_texts(&binder),
            [
                "Binder IndexPolicies ................................ 2    Policy A (v1) ................................ 3-5    Policy B (v1) ................................ 6-7",
                "Policies",
                "Policy ACategory: PoliciesVersion: 1",
                "Policy A page 1",
                "Policy A page 2",
                "Policy BCategory: PoliciesVersion: 1",
                "Policy B page 1",
            ]
        );
    }

    #[test]
    fn paginates_the_index_without_changing_separator_page_references() {
        let documents = (1..=36)
            .map(|number| source(&format!("Policy {number:02}"), "Policies", 1))
            .collect();
        let bytes = LopdfBinderRenderer
            .render(documents)
            .expect("binder should render");
        let binder = Document::load_mem(&bytes).expect("binder should be readable");
        let texts = page_texts(&binder);

        assert_eq!(binder.get_pages().len(), 75);
        assert!(texts[0].starts_with("Binder IndexPolicies ................................ 3"));
        assert!(texts[0].contains("Policy 35 (v1) ................................ 72-73"));
        assert_eq!(
            texts[1],
            "Binder Index, continued    Policy 36 (v1) ................................ 74-75"
        );
        assert_eq!(texts[2], "Policies");
        assert_eq!(texts[3], "Policy 01Category: PoliciesVersion: 1");
        assert_eq!(texts[4], "Policy 01 page 1");
    }

    #[test]
    fn counts_generated_pages_toward_the_page_limit() {
        let source = source("Large document", "Policies", 1);
        let loaded = vec![LoadedDocument {
            source,
            pdf: text_page("Source", &[]).unwrap(),
            pages: 19_998,
            start_page: 0,
        }];

        assert_eq!(binder_page_count(&loaded, 1), 20_001);
    }

    #[test]
    fn preserves_logical_page_order_instead_of_object_id_order() {
        let mut original = merge_documents(vec![
            text_page("First source page", &[]).unwrap(),
            text_page("Second source page", &[]).unwrap(),
        ])
        .and_then(|bytes| {
            Document::load_mem(&bytes).map_err(|error| BinderError::Render(error.to_string()))
        })
        .unwrap();
        let source_pages: Vec<ObjectId> = original.get_pages().into_values().collect();
        let parent = original
            .get_object(source_pages[0])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap();
        original
            .get_object_mut(parent)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "Kids",
                vec![
                    Object::Reference(source_pages[1]),
                    Object::Reference(source_pages[0]),
                ],
            );
        let mut source_bytes = Vec::new();
        original.save_to(&mut source_bytes).unwrap();
        let binder = LopdfBinderRenderer
            .render(vec![PrintableBinderDocument {
                title: "Reordered".into(),
                category_path: "Tests".into(),
                version_number: 1,
                pdf_sha256: String::new(),
                pdf_storage_key: String::new(),
                pdf_content: source_bytes,
            }])
            .and_then(|bytes| {
                Document::load_mem(&bytes).map_err(|error| BinderError::Render(error.to_string()))
            })
            .unwrap();
        let output_pages: Vec<ObjectId> = binder.get_pages().into_values().collect();
        let first_source_content = binder.get_page_content(output_pages[3]);

        assert!(
            first_source_content
                .windows(18)
                .any(|window| window == b"Second source page")
        );
    }

    #[test]
    fn materializes_inherited_page_properties_and_removes_actions() {
        let mut document = text_page("Inherited", &[]).unwrap();
        let page_id = document.get_pages()[&1];
        let annotation_id = document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
            "Contents" => Object::string_literal("Visible annotation"),
        });
        let parent_id = document
            .get_object(page_id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap();
        let mut inherited = Vec::new();
        {
            let page = document
                .get_object_mut(page_id)
                .unwrap()
                .as_dict_mut()
                .unwrap();
            for key in [b"Resources".as_slice(), b"MediaBox"] {
                inherited.push((key.to_vec(), page.remove(key).unwrap()));
            }
            page.set("Rotate", 90);
            page.set(
                "AA",
                dictionary! { "O" => dictionary! { "S" => "JavaScript" } },
            );
            page.set("Annots", vec![Object::Reference(annotation_id)]);
        }
        let parent = document
            .get_object_mut(parent_id)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        for (key, value) in inherited {
            parent.set(key, value);
        }
        sanitize_annotation_actions(&mut document);
        let flattened = flattened_page(&document, page_id).unwrap();
        let page = flattened.as_dict().unwrap();

        assert!(page.get(b"Resources").is_ok());
        assert!(page.get(b"MediaBox").is_ok());
        assert!(page.get(b"Rotate").is_ok());
        assert!(page.get(b"AA").is_err());
        assert!(page.get(b"Annots").is_ok());
        assert!(
            document
                .get_object(annotation_id)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"A")
                .is_err()
        );
    }

    #[test]
    fn transliterates_and_disambiguates_labels_for_standard_pdf_fonts() {
        let transliterated = clean_text("Políticas", 70);
        assert!(transliterated.starts_with("Politicas ["));
        let shortened = clean_text(&"policy".repeat(30), 40);
        assert!(shortened.len() <= 40);
        assert!(shortened.contains('['));
    }

    #[test]
    fn does_not_treat_ordinary_resource_names_as_actions() {
        let mut resources = Object::Dictionary(dictionary! {
            "A" => dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
        });
        sanitize_active_object(&mut resources);

        assert!(resources.as_dict().unwrap().get(b"A").is_ok());
    }

    fn page_texts(document: &Document) -> Vec<String> {
        document
            .get_pages()
            .into_values()
            .map(|page_id| {
                Content::decode(&document.get_page_content(page_id))
                    .unwrap()
                    .operations
                    .into_iter()
                    .filter(|operation| operation.operator == "Tj")
                    .map(|operation| {
                        operation.operands[0]
                            .as_str()
                            .map(|text| String::from_utf8_lossy(text).into_owned())
                            .unwrap()
                    })
                    .collect()
            })
            .collect()
    }

    fn source(title: &str, category: &str, pages: usize) -> PrintableBinderDocument {
        let mut merged = Vec::new();
        for page in 0..pages {
            merged.push(text_page(&format!("{title} page {}", page + 1), &[]).unwrap());
        }
        let mut pdf = merge_documents(merged).unwrap();
        PrintableBinderDocument {
            title: title.into(),
            category_path: category.into(),
            version_number: 1,
            pdf_sha256: String::new(),
            pdf_storage_key: String::new(),
            pdf_content: std::mem::take(&mut pdf),
        }
    }
}
