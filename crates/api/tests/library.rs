//! End-to-end tests for the document library endpoints.
//!
//! These drive real multipart uploads through the assembled router into the real
//! blob store and SQLite, so the ingestion contract is checked the way a client
//! experiences it.

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use elrond_api::ApiConfig;
use elrond_api::cookies::{CSRF_COOKIE, CSRF_HEADER, SESSION_COOKIE};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Boundary used for every multipart body in these tests.
const BOUNDARY: &str = "elrondtestboundary";

/// A signed-in client.
struct Client {
    session: String,
    csrf: String,
}

impl Client {
    /// Renders the `Cookie` header.
    fn cookies(&self) -> String {
        format!(
            "{SESSION_COOKIE}={}; {CSRF_COOKIE}={}",
            self.session, self.csrf
        )
    }
}

/// Extracts a cookie value from a response.
fn set_cookie(response: &Response<Body>, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .map(|value| {
            value
                .trim_start_matches(&prefix)
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
}

/// Reads a body as JSON.
async fn json_body(response: Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body is readable");
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// Reads a body as raw bytes.
async fn raw_body(response: Response<Body>) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body is readable")
        .to_vec()
}

/// Builds an app and signs in an administrator.
async fn signed_in() -> (Router, Client) {
    let app = support::build_with(ApiConfig::development()).await.router;

    let bootstrap = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bootstrap")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    let csrf = set_cookie(&bootstrap, CSRF_COOKIE).expect("csrf cookie");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, format!("{CSRF_COOKIE}={csrf}"))
                .header(CSRF_HEADER, &csrf)
                .body(Body::from(
                    json!({
                        "username": "records.admin",
                        "password": "a sufficiently long passphrase"
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::CREATED);

    let session = set_cookie(&response, SESSION_COOKIE).expect("session cookie");
    let csrf = set_cookie(&response, CSRF_COOKIE).expect("rotated csrf cookie");
    (app, Client { session, csrf })
}

/// Builds a multipart body with a file part and optional text parts.
fn multipart(filename: &str, content_type: &str, bytes: &[u8], fields: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// Sends a multipart upload.
async fn upload(
    app: &Router,
    client: &Client,
    path: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    fields: &[(&str, &str)],
) -> Response<Body> {
    let body = multipart(filename, content_type, bytes, fields);
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .header(header::COOKIE, client.cookies())
                .header(CSRF_HEADER, &client.csrf)
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds")
}

/// Sends a GET request as a signed-in client.
async fn get(app: &Router, client: &Client, path: &str) -> Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header::COOKIE, client.cookies())
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds")
}

/// Sends a JSON request as a signed-in client.
async fn send_json(
    app: &Router,
    client: &Client,
    method: &str,
    path: &str,
    body: Value,
) -> Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, client.cookies())
                .header(CSRF_HEADER, &client.csrf)
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router responds")
}

/// A minimal but structurally valid PDF.
fn pdf_bytes(marker: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(marker.as_bytes());
    bytes.extend_from_slice(b"\n%%EOF\n");
    bytes
}

// ------------------------------------------------------------------- uploads

#[tokio::test]
async fn uploading_a_pdf_creates_a_draft_document() {
    let (app, client) = signed_in().await;

    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "Retention Policy.pdf",
        "application/pdf",
        &pdf_bytes("retention"),
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = json_body(response).await;
    let document = &body["document"];
    assert_eq!(document["title"], "Retention Policy");
    // Nothing reaches viewers or a binder until it has been through review.
    assert_eq!(document["lifecycle"], "draft");
    assert_eq!(document["version_count"], 1);
    assert_eq!(document["current_version"]["number"], 1);
    assert_eq!(document["current_version"]["media_type"], "application/pdf");
    assert_eq!(document["current_version"]["has_pdf"], true);
    assert_eq!(document["current_version"]["awaiting_conversion"], false);
    // With no category supplied it lands in the on-demand "Unfiled" root.
    assert_eq!(document["category_name"], "Unfiled");
}

#[tokio::test]
async fn a_storage_key_is_never_exposed_to_a_client() {
    let (app, client) = signed_in().await;
    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "policy.pdf",
        "application/pdf",
        &pdf_bytes("x"),
        &[],
    )
    .await;

    let body = json_body(response).await.to_string();
    assert!(
        !body.contains("originals/"),
        "internal storage layout leaked: {body}"
    );
    assert!(!body.contains("storage_key"), "storage key leaked: {body}");
}

#[tokio::test]
async fn the_title_is_derived_from_the_filename_but_can_be_overridden() {
    let (app, client) = signed_in().await;

    let derived = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "annual_report-2026.pdf",
            "application/pdf",
            &pdf_bytes("a"),
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(derived["document"]["title"], "annual report 2026");

    let explicit = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "annual_report-2026.pdf",
            "application/pdf",
            &pdf_bytes("b"),
            &[("title", "Annual Report 2026")],
        )
        .await,
    )
    .await;
    assert_eq!(explicit["document"]["title"], "Annual Report 2026");
}

#[tokio::test]
async fn a_traversal_filename_is_sanitized_rather_than_rejected() {
    let (app, client) = signed_in().await;

    let body = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "../../etc/passwd.pdf",
            "application/pdf",
            &pdf_bytes("traversal"),
            &[],
        )
        .await,
    )
    .await;

    let filename = body["document"]["current_version"]["filename"]
        .as_str()
        .expect("a filename");
    assert_eq!(filename, "passwd.pdf");
    assert!(!filename.contains(".."), "path components survived");
    assert!(!filename.contains('/'), "path separators survived");
}

#[tokio::test]
async fn identical_content_is_deduplicated_and_reported() {
    let (app, client) = signed_in().await;
    let bytes = pdf_bytes("same content");

    let first = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "first.pdf",
            "application/pdf",
            &bytes,
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(first["deduplicated"], false);
    assert_eq!(first["duplicate_of"], Value::Null);

    let second = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "second.pdf",
            "application/pdf",
            &bytes,
            &[],
        )
        .await,
    )
    .await;

    // The bytes are stored once, but the second upload is still its own document:
    // the same file legitimately belongs in more than one place.
    assert_eq!(second["deduplicated"], true);
    assert_eq!(second["duplicate_of"], first["document"]["id"]);
    assert_ne!(second["document"]["id"], first["document"]["id"]);
    assert_eq!(
        second["document"]["current_version"]["checksum"],
        first["document"]["current_version"]["checksum"]
    );
}

#[tokio::test]
async fn content_wins_over_a_lying_extension() {
    let (app, client) = signed_in().await;

    // A PNG named .pdf. Storing it as a PDF would hand a broken file to the PDF
    // pipeline, so the mismatch is reported instead.
    let png = [
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R',
    ];
    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "disguised.pdf",
        "application/pdf",
        &png,
        &[],
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = json_body(response).await;
    assert_eq!(body["code"], "conflict");
}

#[tokio::test]
async fn an_unsupported_file_type_is_refused() {
    let (app, client) = signed_in().await;

    // A Windows executable: a recognizable signature that is not on the supported
    // list, and no extension that would rescue it.
    let mut exe = b"MZ".to_vec();
    exe.extend_from_slice(&[0_u8; 64]);

    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "payload.exe",
        "application/octet-stream",
        &exe,
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_text_file_is_accepted_on_the_strength_of_its_extension() {
    let (app, client) = signed_in().await;

    // Plain text has no magic bytes, so the extension is the only signal and must
    // be honoured rather than treated as unidentifiable.
    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "notes.txt",
        "text/plain",
        b"Minutes of the meeting.",
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = json_body(response).await;
    assert_eq!(
        body["document"]["current_version"]["media_type"],
        "text/plain"
    );
    // Not a PDF, so it waits for a generated copy before it can be viewed.
    assert_eq!(body["document"]["current_version"]["has_pdf"], false);
    assert_eq!(
        body["document"]["current_version"]["awaiting_conversion"],
        true
    );
}

#[tokio::test]
async fn an_empty_file_is_refused() {
    let (app, client) = signed_in().await;
    let response = upload(
        &app,
        &client,
        "/api/v1/documents",
        "empty.pdf",
        "application/pdf",
        b"",
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_upload_without_a_file_part_is_a_bad_request() {
    let (app, client) = signed_in().await;

    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nNo file\r\n--{BOUNDARY}--\r\n"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/documents")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .header(header::COOKIE, client.cookies())
                .header(CSRF_HEADER, &client.csrf)
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_upload_without_a_csrf_token_is_refused() {
    let (app, client) = signed_in().await;

    let body = multipart("policy.pdf", "application/pdf", &pdf_bytes("x"), &[]);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/documents")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .header(header::COOKIE, client.cookies())
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ------------------------------------------------------------------- imports

/// Builds a ZIP archive in memory from `(path, bytes)` pairs.
fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut cursor);
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in files {
        writer.start_file(*name, options).expect("start entry");
        writer.write_all(bytes).expect("write entry");
    }
    writer.finish().expect("finish archive");
    cursor.into_inner()
}

#[tokio::test]
async fn importing_a_zip_recreates_its_folders_as_categories() {
    let (app, client) = signed_in().await;

    let archive = zip_bytes(&[
        ("Policies/Retention Policy.pdf", &pdf_bytes("retention")),
        ("Policies/Access/Access Policy.pdf", &pdf_bytes("access")),
        ("Minutes/January.pdf", &pdf_bytes("january")),
    ]);

    let response = upload(
        &app,
        &client,
        "/api/v1/documents/import",
        "library.zip",
        "application/zip",
        &archive,
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = json_body(response).await;
    let imported = body["imported"].as_array().expect("imported list");
    assert_eq!(imported.len(), 3);
    assert_eq!(body["skipped"].as_array().expect("skipped list").len(), 0);

    // Folders became categories, nested as they were in the archive.
    assert_eq!(imported[0]["category_name"], "Policies");
    assert_eq!(imported[1]["category_name"], "Access");
    assert_eq!(imported[2]["category_name"], "Minutes");

    // Where each document came from inside the archive is recorded.
    assert_eq!(
        imported[1]["source_path"],
        "Policies/Access/Access Policy.pdf"
    );

    // The tree shows the created hierarchy.
    let tree = json_body(get(&app, &client, "/api/v1/categories").await).await;
    let policies = tree
        .as_array()
        .expect("a tree")
        .iter()
        .find(|node| node["name"] == "Policies")
        .expect("Policies exists");
    assert_eq!(policies["children"][0]["name"], "Access");
}

#[tokio::test]
async fn an_import_skips_junk_instead_of_failing_the_archive() {
    let (app, client) = signed_in().await;

    let archive = zip_bytes(&[
        ("Policies/Retention Policy.pdf", &pdf_bytes("retention")),
        (
            "Policies/.DS_Store",
            b"junk the operating system left behind",
        ),
        ("Policies/setup.exe", b"MZ not welcome"),
    ]);

    let response = upload(
        &app,
        &client,
        "/api/v1/documents/import",
        "library.zip",
        "application/zip",
        &archive,
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = json_body(response).await;
    assert_eq!(body["imported"].as_array().expect("imported").len(), 1);

    let skipped = body["skipped"].as_array().expect("skipped");
    assert_eq!(skipped.len(), 2);
    assert_eq!(skipped[0]["path"], "Policies/.DS_Store");
    assert_eq!(skipped[1]["path"], "Policies/setup.exe");
    for entry in skipped {
        assert!(
            entry["reason"].as_str().expect("a reason").contains("file"),
            "the reason should say what was wrong: {entry:?}"
        );
    }
}

#[tokio::test]
async fn an_import_can_target_an_existing_category() {
    let (app, client) = signed_in().await;

    let created = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "Archive" }),
        )
        .await,
    )
    .await;
    let root = created["id"].as_str().expect("an id").to_owned();

    let archive = zip_bytes(&[("2025/Report.pdf", &pdf_bytes("report"))]);
    let response = upload(
        &app,
        &client,
        "/api/v1/documents/import",
        "old.zip",
        "application/zip",
        &archive,
        &[("category_id", root.as_str())],
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    // The archive's folders were created under the chosen root.
    let tree = json_body(get(&app, &client, "/api/v1/categories").await).await;
    let archive_node = tree
        .as_array()
        .expect("a tree")
        .iter()
        .find(|node| node["name"] == "Archive")
        .expect("Archive exists");
    assert_eq!(archive_node["children"][0]["name"], "2025");
}

#[tokio::test]
async fn a_file_that_is_not_a_zip_is_refused() {
    let (app, client) = signed_in().await;

    let response = upload(
        &app,
        &client,
        "/api/v1/documents/import",
        "not-an-archive.zip",
        "application/zip",
        b"%PDF-1.7 this is a pdf, not a zip",
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = json_body(response).await;
    assert_eq!(body["code"], "archive_unreadable");
}

// ---------------------------------------------------------------- categories

#[tokio::test]
async fn categories_nest_and_report_rolled_up_counts() {
    let (app, client) = signed_in().await;

    let parent = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "Policies" }),
        )
        .await,
    )
    .await;
    let parent_id = parent["id"].as_str().expect("an id").to_owned();

    let child = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "2026", "parent_id": parent_id }),
        )
        .await,
    )
    .await;
    let child_id = child["id"].as_str().expect("an id").to_owned();

    upload(
        &app,
        &client,
        "/api/v1/documents",
        "nested.pdf",
        "application/pdf",
        &pdf_bytes("nested"),
        &[("category_id", &child_id)],
    )
    .await;

    let tree = json_body(get(&app, &client, "/api/v1/categories").await).await;
    let policies = tree
        .as_array()
        .expect("an array")
        .iter()
        .find(|node| node["name"] == "Policies")
        .expect("Policies is present");

    // The document is filed in the child, so the parent's direct count is zero but
    // its rolled-up count is one.
    assert_eq!(policies["document_count"], 0);
    assert_eq!(policies["total_document_count"], 1);
    assert_eq!(policies["children"][0]["name"], "2026");
    assert_eq!(policies["children"][0]["document_count"], 1);
}

#[tokio::test]
async fn a_duplicate_sibling_category_name_is_refused() {
    let (app, client) = signed_in().await;
    send_json(
        &app,
        &client,
        "POST",
        "/api/v1/categories",
        json!({ "name": "Policies" }),
    )
    .await;

    let response = send_json(
        &app,
        &client,
        "POST",
        "/api/v1/categories",
        json!({ "name": "policies" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_category_cannot_be_moved_inside_itself() {
    let (app, client) = signed_in().await;

    let parent = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "Policies" }),
        )
        .await,
    )
    .await;
    let parent_id = parent["id"].as_str().expect("an id").to_owned();

    let child = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "2026", "parent_id": parent_id }),
        )
        .await,
    )
    .await;
    let child_id = child["id"].as_str().expect("an id").to_owned();

    // Moving the parent under its own child would detach the subtree into a cycle
    // and make every tree walk recurse forever.
    let response = send_json(
        &app,
        &client,
        "PATCH",
        &format!("/api/v1/categories/{parent_id}"),
        json!({ "parent_id": child_id }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_category_holding_documents_cannot_be_deleted() {
    let (app, client) = signed_in().await;

    let category = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "Policies" }),
        )
        .await,
    )
    .await;
    let category_id = category["id"].as_str().expect("an id").to_owned();

    upload(
        &app,
        &client,
        "/api/v1/documents",
        "kept.pdf",
        "application/pdf",
        &pdf_bytes("kept"),
        &[("category_id", &category_id)],
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/categories/{category_id}"))
                .header(header::COOKIE, client.cookies())
                .header(CSRF_HEADER, &client.csrf)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

// ------------------------------------------------------------------- listing

#[tokio::test]
async fn documents_are_listed_with_a_total() {
    let (app, client) = signed_in().await;
    for index in 0..3 {
        upload(
            &app,
            &client,
            "/api/v1/documents",
            &format!("policy-{index}.pdf"),
            "application/pdf",
            &pdf_bytes(&format!("policy {index}")),
            &[],
        )
        .await;
    }

    let body = json_body(get(&app, &client, "/api/v1/documents").await).await;
    assert_eq!(body["total"], 3);
    assert_eq!(body["documents"].as_array().expect("an array").len(), 3);
    assert_eq!(body["limit"], 50);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn the_page_size_is_capped() {
    let (app, client) = signed_in().await;
    let body = json_body(get(&app, &client, "/api/v1/documents?limit=100000").await).await;
    // A crafted limit must not make the server materialize the whole library.
    assert_eq!(body["limit"], 200);
}

#[tokio::test]
async fn paging_returns_distinct_rows() {
    let (app, client) = signed_in().await;
    for index in 0..5 {
        upload(
            &app,
            &client,
            "/api/v1/documents",
            &format!("policy-{index}.pdf"),
            "application/pdf",
            &pdf_bytes(&format!("policy {index}")),
            &[],
        )
        .await;
    }

    let first = json_body(get(&app, &client, "/api/v1/documents?limit=2&offset=0").await).await;
    let second = json_body(get(&app, &client, "/api/v1/documents?limit=2&offset=2").await).await;

    let ids = |page: &Value| {
        page["documents"]
            .as_array()
            .expect("an array")
            .iter()
            .map(|row| row["id"].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>()
    };
    let first_ids = ids(&first);
    let second_ids = ids(&second);

    assert_eq!(first_ids.len(), 2);
    assert_eq!(second_ids.len(), 2);
    assert!(
        first_ids.iter().all(|id| !second_ids.contains(id)),
        "pages overlapped: {first_ids:?} then {second_ids:?}"
    );
    assert_eq!(first["total"], 5);
}

#[tokio::test]
async fn an_unknown_sort_column_is_rejected() {
    let (app, client) = signed_in().await;
    let response = get(&app, &client, "/api/v1/documents?sort=nonsense").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_category_filter_includes_descendants_by_default() {
    let (app, client) = signed_in().await;

    let parent = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "Policies" }),
        )
        .await,
    )
    .await;
    let parent_id = parent["id"].as_str().expect("an id").to_owned();
    let child = json_body(
        send_json(
            &app,
            &client,
            "POST",
            "/api/v1/categories",
            json!({ "name": "2026", "parent_id": parent_id }),
        )
        .await,
    )
    .await;
    let child_id = child["id"].as_str().expect("an id").to_owned();

    upload(
        &app,
        &client,
        "/api/v1/documents",
        "nested.pdf",
        "application/pdf",
        &pdf_bytes("nested"),
        &[("category_id", &child_id)],
    )
    .await;

    let inclusive = json_body(
        get(
            &app,
            &client,
            &format!("/api/v1/documents?category_id={parent_id}"),
        )
        .await,
    )
    .await;
    assert_eq!(inclusive["total"], 1, "descendants should be included");

    let exclusive = json_body(
        get(
            &app,
            &client,
            &format!("/api/v1/documents?category_id={parent_id}&include_descendants=false"),
        )
        .await,
    )
    .await;
    assert_eq!(exclusive["total"], 0, "only direct children were asked for");
}

// -------------------------------------------------------------------- search

#[tokio::test]
async fn search_finds_a_document_by_title() {
    let (app, client) = signed_in().await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "Retention Policy.pdf",
        "application/pdf",
        &pdf_bytes("a"),
        &[],
    )
    .await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "Travel Expenses.pdf",
        "application/pdf",
        &pdf_bytes("b"),
        &[],
    )
    .await;

    let body = json_body(get(&app, &client, "/api/v1/documents?q=retention").await).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["documents"][0]["title"], "Retention Policy");
}

#[tokio::test]
async fn search_finds_a_document_by_tag() {
    let (app, client) = signed_in().await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "minutes.pdf",
        "application/pdf",
        &pdf_bytes("a"),
        &[("tags", "board, governance")],
    )
    .await;

    let body = json_body(get(&app, &client, "/api/v1/documents?q=governance").await).await;
    assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn a_hostile_search_query_does_not_error() {
    let (app, client) = signed_in().await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "Retention Policy.pdf",
        "application/pdf",
        &pdf_bytes("a"),
        &[],
    )
    .await;

    // An unbalanced quote is valid FTS5 syntax that would otherwise surface as a
    // database error the user cannot act on.
    for hostile in ["retention%22", "(retention", "title%3Aretention", "%22%22"] {
        let response = get(&app, &client, &format!("/api/v1/documents?q={hostile}")).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "query {hostile:?} should not error"
        );
    }
}

#[tokio::test]
async fn a_search_matching_nothing_returns_an_empty_page() {
    let (app, client) = signed_in().await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "policy.pdf",
        "application/pdf",
        &pdf_bytes("a"),
        &[],
    )
    .await;

    let body = json_body(get(&app, &client, "/api/v1/documents?q=nonexistentterm").await).await;
    assert_eq!(body["total"], 0);
    assert!(body["documents"].as_array().expect("an array").is_empty());
}

// ---------------------------------------------------------------------- tags

#[tokio::test]
async fn tags_are_created_on_upload_and_deduplicated_by_case() {
    let (app, client) = signed_in().await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "a.pdf",
        "application/pdf",
        &pdf_bytes("a"),
        &[("tags", "Board Minutes, policy")],
    )
    .await;
    upload(
        &app,
        &client,
        "/api/v1/documents",
        "b.pdf",
        "application/pdf",
        &pdf_bytes("b"),
        &[("tags", "board minutes")],
    )
    .await;

    let tags = json_body(get(&app, &client, "/api/v1/tags").await).await;
    let tags = tags.as_array().expect("an array");
    assert_eq!(tags.len(), 2, "case variants should be one tag: {tags:?}");

    let board = tags
        .iter()
        .find(|tag| tag["label"] == "Board Minutes")
        .expect("the first spelling is kept");
    assert_eq!(board["document_count"], 2);
}

// -------------------------------------------------------- detail and download

#[tokio::test]
async fn a_document_detail_includes_its_version_history() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("first"),
            &[],
        )
        .await,
    )
    .await;
    let id = created["document"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    upload(
        &app,
        &client,
        &format!("/api/v1/documents/{id}/versions"),
        "policy.pdf",
        "application/pdf",
        &pdf_bytes("second"),
        &[("title", "Corrected the retention period")],
    )
    .await;

    let detail = json_body(get(&app, &client, &format!("/api/v1/documents/{id}")).await).await;
    assert_eq!(detail["version_count"], 2);
    assert_eq!(detail["current_version"]["number"], 2);

    let versions = detail["versions"].as_array().expect("an array");
    assert_eq!(versions.len(), 2);
    // Newest first, and the earlier version is retained because a binder release
    // may pin it.
    assert_eq!(versions[0]["number"], 2);
    assert_eq!(versions[1]["number"], 1);
    assert_eq!(versions[0]["note"], "Corrected the retention period");
}

#[tokio::test]
async fn the_original_downloads_byte_for_byte() {
    let (app, client) = signed_in().await;
    let bytes = pdf_bytes("exact bytes matter");

    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &bytes,
            &[],
        )
        .await,
    )
    .await;
    let version_id = created["document"]["current_version"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let response = get(
        &app,
        &client,
        &format!("/api/v1/versions/{version_id}/original"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("attachment")),
        Some(true)
    );
    assert!(response.headers().contains_key(header::ETAG));

    // Immutability is the product's core promise, so this is the assertion that
    // matters most in this file.
    assert_eq!(raw_body(response).await, bytes);
}

#[tokio::test]
async fn a_pdf_original_is_served_inline_for_the_viewer() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("viewable"),
            &[],
        )
        .await,
    )
    .await;
    let version_id = created["document"]["current_version"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let response = get(&app, &client, &format!("/api/v1/versions/{version_id}/pdf")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("inline")),
        Some(true)
    );
}

#[tokio::test]
async fn a_pdf_is_unavailable_until_a_non_pdf_has_been_converted() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "notes.txt",
            "text/plain",
            b"Some notes.",
            &[],
        )
        .await,
    )
    .await;
    let version_id = created["document"]["current_version"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    // Conversion arrives in a later milestone, so this reports a conflict rather
    // than serving the text file as though it were a PDF.
    let response = get(&app, &client, &format!("/api/v1/versions/{version_id}/pdf")).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // The original is still downloadable.
    let original = get(
        &app,
        &client,
        &format!("/api/v1/versions/{version_id}/original"),
    )
    .await;
    assert_eq!(original.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_document_is_a_not_found() {
    let (app, client) = signed_in().await;
    let response = get(
        &app,
        &client,
        "/api/v1/documents/019fb07b-5f27-75a1-b4f0-c8ba56882c36",
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ------------------------------------------------------- metadata and lifecycle

#[tokio::test]
async fn metadata_can_be_updated_including_replacing_the_tag_set() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("a"),
            &[("tags", "draft, old")],
        )
        .await,
    )
    .await;
    let id = created["document"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let category_id = created["document"]["category_id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let updated = json_body(
        send_json(
            &app,
            &client,
            "PATCH",
            &format!("/api/v1/documents/{id}"),
            json!({
                "title": "Retention Policy",
                "category_id": category_id,
                "review_due_at": "2027-01-01T00:00:00Z",
                "tags": ["approved"]
            }),
        )
        .await,
    )
    .await;

    assert_eq!(updated["title"], "Retention Policy");
    assert_eq!(updated["review_due_at"], "2027-01-01T00:00:00Z");

    let labels: Vec<&str> = updated["tags"]
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|tag| tag["label"].as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["approved"],
        "the tag set is replaced wholesale"
    );
}

#[tokio::test]
async fn a_malformed_review_date_is_rejected() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("a"),
            &[],
        )
        .await,
    )
    .await;
    let id = created["document"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let category_id = created["document"]["category_id"]
        .as_str()
        .expect("an id")
        .to_owned();

    let response = send_json(
        &app,
        &client,
        "PATCH",
        &format!("/api/v1/documents/{id}"),
        json!({
            "title": "Retention Policy",
            "category_id": category_id,
            "review_due_at": "next Tuesday"
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_document_walks_the_lifecycle_and_refuses_illegal_moves() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("a"),
            &[],
        )
        .await,
    )
    .await;
    let id = created["document"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let path = format!("/api/v1/documents/{id}/lifecycle");

    // Publishing without review is not a legal move.
    let straight_to_published = send_json(
        &app,
        &client,
        "POST",
        &path,
        json!({ "lifecycle": "published" }),
    )
    .await;
    assert_eq!(
        straight_to_published.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let in_review = json_body(
        send_json(
            &app,
            &client,
            "POST",
            &path,
            json!({ "lifecycle": "in_review" }),
        )
        .await,
    )
    .await;
    assert_eq!(in_review["lifecycle"], "in_review");

    let published = json_body(
        send_json(
            &app,
            &client,
            "POST",
            &path,
            json!({ "lifecycle": "published" }),
        )
        .await,
    )
    .await;
    assert_eq!(published["lifecycle"], "published");

    // A published document cannot return to an editable state; that is what makes
    // a binder release reproducible.
    let back_to_draft = send_json(
        &app,
        &client,
        "POST",
        &path,
        json!({ "lifecycle": "draft" }),
    )
    .await;
    assert_eq!(back_to_draft.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn content_cannot_be_appended_while_a_document_is_in_review() {
    let (app, client) = signed_in().await;
    let created = json_body(
        upload(
            &app,
            &client,
            "/api/v1/documents",
            "policy.pdf",
            "application/pdf",
            &pdf_bytes("a"),
            &[],
        )
        .await,
    )
    .await;
    let id = created["document"]["id"]
        .as_str()
        .expect("an id")
        .to_owned();

    send_json(
        &app,
        &client,
        "POST",
        &format!("/api/v1/documents/{id}/lifecycle"),
        json!({ "lifecycle": "in_review" }),
    )
    .await;

    let response = upload(
        &app,
        &client,
        &format!("/api/v1/documents/{id}/versions"),
        "policy.pdf",
        "application/pdf",
        &pdf_bytes("sneaky change"),
        &[],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "content is frozen while reviewers are looking at it"
    );
}

// ------------------------------------------------------------- authentication

#[tokio::test]
async fn library_endpoints_require_a_session() {
    let app = support::build_with(ApiConfig::development()).await.router;

    for path in ["/api/v1/documents", "/api/v1/categories", "/api/v1/tags"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "for {path}");
    }
}
