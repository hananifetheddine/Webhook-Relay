mod common;

use std::time::Duration;

use serde_json::json;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use webhook_relay::signing::verify_signature;
use webhook_relay::worker::{self};

use crate::common::{get_json, post_json, register_and_login, spawn, spawn_with};

#[tokio::test]
async fn health_register_login_and_auth_errors() {
    let app = spawn().await;
    let health = app
        .client
        .get(format!("{}/health", app.base))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    let email = format!("ada-{}@example.com", Uuid::new_v4());
    let created = app
        .client
        .post(format!("{}/auth/register", app.base))
        .json(&json!({ "email": &email, "password": "correct-horse" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);

    let duplicate = app
        .client
        .post(format!("{}/auth/register", app.base))
        .json(&json!({ "email": &email, "password": "correct-horse" }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 409);

    let bad_password = app
        .client
        .post(format!("{}/auth/login", app.base))
        .json(&json!({ "email": &email, "password": "wrong-password" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_password.status(), 401);

    let missing = app
        .client
        .get(format!("{}/endpoints", app.base))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 401);
}

#[tokio::test]
async fn endpoint_secret_is_returned_once_then_delete_hides_it() {
    let app = spawn().await;
    let token = register_and_login(&app).await;

    let created = post_json(
        &app,
        &token,
        "/endpoints",
        json!({
            "url": "https://example.com/hooks",
            "event_types": ["invoice.paid", "invoice.paid"]
        }),
    )
    .await;
    assert_eq!(created.status(), 201);
    let created: serde_json::Value = created.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap();
    assert!(secret.starts_with("whsec_"));
    assert_eq!(created["event_types"].as_array().unwrap().len(), 1);

    let invalid = post_json(
        &app,
        &token,
        "/endpoints",
        json!({ "url": "ftp://example.com/hooks", "event_types": ["invoice.paid"] }),
    )
    .await;
    assert_eq!(invalid.status(), 400);

    let listed = get_json(&app, &token, "/endpoints").await;
    assert_eq!(listed.status(), 200);
    let listed: serde_json::Value = listed.json().await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(listed[0].get("secret").is_none());

    let id = created["id"].as_str().unwrap();
    let deleted = app
        .client
        .delete(format!("{}/endpoints/{id}", app.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);

    let listed = get_json(&app, &token, "/endpoints").await;
    let listed: serde_json::Value = listed.json().await.unwrap();
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delivers_only_matching_endpoints_with_a_valid_signature() {
    let app = spawn().await;
    let token = register_and_login(&app).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hooks"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let matching = post_json(
        &app,
        &token,
        "/endpoints",
        json!({
            "url": format!("{}/hooks", server.uri()),
            "event_types": ["invoice.paid"]
        }),
    )
    .await;
    assert_eq!(matching.status(), 201);
    let matching: serde_json::Value = matching.json().await.unwrap();
    let secret = matching["secret"].as_str().unwrap().to_string();

    let ignored = post_json(
        &app,
        &token,
        "/endpoints",
        json!({
            "url": format!("{}/other", server.uri()),
            "event_types": ["user.created"]
        }),
    )
    .await;
    assert_eq!(ignored.status(), 201);

    let created = post_json(
        &app,
        &token,
        "/events",
        json!({ "type": "invoice.paid", "payload": { "id": "inv_1", "ok": true } }),
    )
    .await;
    assert_eq!(created.status(), 202);
    let created: serde_json::Value = created.json().await.unwrap();
    assert_eq!(created["deliveries"].as_array().unwrap().len(), 1);
    let event_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();

    let sent = worker::process_event(&app.state, event_id).await.unwrap();
    assert_eq!(sent, 1);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        request.headers.get("x-webhook-event").unwrap(),
        "invoice.paid"
    );
    let timestamp: i64 = request
        .headers
        .get("x-webhook-timestamp")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let signature = request
        .headers
        .get("x-webhook-signature")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(verify_signature(
        &secret,
        &request.body,
        signature,
        timestamp,
        0
    ));

    let detail = get_json(&app, &token, &format!("/events/{event_id}")).await;
    assert_eq!(detail.status(), 200);
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["payload"]["id"], "inv_1");
    assert_eq!(detail["attempts"][0]["status"], "success");
    assert_eq!(detail["attempts"][0]["http_status"], 204);
    assert_eq!(detail["attempts"][0]["attempt_count"], 1);
}

#[tokio::test]
async fn failed_delivery_is_retried_on_replay() {
    let app = spawn().await;
    let token = register_and_login(&app).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hooks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let endpoint = post_json(
        &app,
        &token,
        "/endpoints",
        json!({ "url": format!("{}/hooks", server.uri()), "event_types": ["*"] }),
    )
    .await;
    assert_eq!(endpoint.status(), 201);

    let created = post_json(
        &app,
        &token,
        "/events",
        json!({ "type": "ping", "payload": { "n": 1 } }),
    )
    .await;
    let created: serde_json::Value = created.json().await.unwrap();
    let event_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();

    assert_eq!(
        worker::process_event(&app.state, event_id).await.unwrap(),
        1
    );
    let detail = get_json(&app, &token, &format!("/events/{event_id}")).await;
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["attempts"][0]["status"], "failed");
    assert_eq!(detail["attempts"][0]["http_status"], 500);
    assert_eq!(detail["attempts"][0]["attempt_count"], 1);
    assert!(detail["attempts"][0]["next_attempt_at"].is_string());

    let replay = post_json(
        &app,
        &token,
        &format!("/events/{event_id}/replay"),
        json!({}),
    )
    .await;
    assert_eq!(replay.status(), 202);
    let replay: serde_json::Value = replay.json().await.unwrap();
    assert_eq!(replay["requeued"], 1);
    assert_eq!(replay["event"]["attempts"][0]["status"], "pending");
    assert_eq!(replay["event"]["attempts"][0]["attempt_count"], 0);

    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/hooks"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    assert_eq!(
        worker::process_event(&app.state, event_id).await.unwrap(),
        1
    );
    let detail = get_json(&app, &token, &format!("/events/{event_id}")).await;
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["attempts"][0]["status"], "success");
    assert_eq!(detail["attempts"][0]["http_status"], 200);
}

#[tokio::test]
async fn delivery_is_exhausted_after_the_attempt_budget() {
    let app = spawn_with(2, 5).await;
    let token = register_and_login(&app).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hooks"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let endpoint = post_json(
        &app,
        &token,
        "/endpoints",
        json!({ "url": format!("{}/hooks", server.uri()), "event_types": ["ping"] }),
    )
    .await;
    assert_eq!(endpoint.status(), 201);

    let created = post_json(
        &app,
        &token,
        "/events",
        json!({ "type": "ping", "payload": {} }),
    )
    .await;
    let created: serde_json::Value = created.json().await.unwrap();
    let event_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let delivery_id = Uuid::parse_str(created["deliveries"][0]["id"].as_str().unwrap()).unwrap();

    assert_eq!(
        worker::process_event(&app.state, event_id).await.unwrap(),
        1
    );
    worker::make_due(&app.state, delivery_id).await.unwrap();
    assert_eq!(
        worker::process_event(&app.state, event_id).await.unwrap(),
        1
    );

    let detail = get_json(&app, &token, &format!("/events/{event_id}")).await;
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["attempts"][0]["status"], "exhausted");
    assert_eq!(detail["attempts"][0]["attempt_count"], 2);
    assert!(detail["attempts"][0]["next_attempt_at"].is_null());
}

#[tokio::test]
async fn timeout_is_a_failed_attempt() {
    let app = spawn_with(6, 1).await;
    let token = register_and_login(&app).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/hooks"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;

    let endpoint = post_json(
        &app,
        &token,
        "/endpoints",
        json!({ "url": format!("{}/hooks", server.uri()), "event_types": ["ping"] }),
    )
    .await;
    assert_eq!(endpoint.status(), 201);

    let created = post_json(
        &app,
        &token,
        "/events",
        json!({ "type": "ping", "payload": { "slow": true } }),
    )
    .await;
    let created: serde_json::Value = created.json().await.unwrap();
    let event_id = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();

    assert_eq!(
        worker::process_event(&app.state, event_id).await.unwrap(),
        1
    );
    let detail = get_json(&app, &token, &format!("/events/{event_id}")).await;
    let detail: serde_json::Value = detail.json().await.unwrap();
    assert_eq!(detail["attempts"][0]["status"], "failed");
    assert!(detail["attempts"][0]["http_status"].is_null());
    assert_eq!(detail["attempts"][0]["attempt_count"], 1);
}

#[tokio::test]
async fn events_are_private_and_can_be_stored_without_a_subscriber() {
    let app = spawn().await;
    let token = register_and_login(&app).await;
    let other = register_and_login(&app).await;

    let created = post_json(
        &app,
        &token,
        "/events",
        json!({ "type": "invoice.paid", "payload": { "secret": "keep" } }),
    )
    .await;
    assert_eq!(created.status(), 202);
    let created: serde_json::Value = created.json().await.unwrap();
    assert!(created["deliveries"].as_array().unwrap().is_empty());
    let event_id = created["id"].as_str().unwrap();

    let hidden = get_json(&app, &other, &format!("/events/{event_id}")).await;
    assert_eq!(hidden.status(), 404);

    let visible = get_json(&app, &token, &format!("/events/{event_id}")).await;
    assert_eq!(visible.status(), 200);
}
