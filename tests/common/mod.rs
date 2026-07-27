use std::time::Duration;

use serde_json::{Value, json};
use sqlx::mysql::MySqlPoolOptions;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use uuid::Uuid;

use webhook_relay::{AppState, Config, migrate, router};

static MIGRATED: Mutex<bool> = Mutex::const_new(false);

pub struct TestApp {
    pub base: String,
    pub state: AppState,
    pub client: reqwest::Client,
}

pub async fn spawn() -> TestApp {
    spawn_with(6, 5).await
}

pub async fn spawn_with(max_delivery_attempts: u32, http_timeout_secs: u64) -> TestApp {
    dotenvy::dotenv().ok();
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set to a MySQL database");
    let config = Config::for_tests(database_url, max_delivery_attempts, http_timeout_secs);
    let pool = MySqlPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .expect("failed to connect to MySQL");

    {
        let mut migrated = MIGRATED.lock().await;
        if !*migrated {
            migrate(&pool).await.expect("migrations failed");
            *migrated = true;
        }
    }

    let state = AppState::new(pool, config).expect("failed to build state");
    let app = router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            panic!("test server failed: {err}");
        }
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    for _ in 0..50 {
        if client.get(format!("{base}/health")).send().await.is_ok() {
            return TestApp {
                base,
                state,
                client,
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("test server did not become ready");
}

pub async fn register_and_login(app: &TestApp) -> String {
    let email = format!("user-{}@example.com", Uuid::new_v4());
    let password = "correct-horse";
    let response = app
        .client
        .post(format!("{}/auth/register", app.base))
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .expect("register");
    assert_eq!(
        response.status(),
        201,
        "{}",
        response.text().await.unwrap_or_default()
    );

    let response = app
        .client
        .post(format!("{}/auth/login", app.base))
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .expect("login");
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("login json");
    body["token"].as_str().expect("token").to_string()
}

pub async fn post_json(app: &TestApp, token: &str, path: &str, body: Value) -> reqwest::Response {
    app.client
        .post(format!("{}{path}", app.base))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("request")
}

pub async fn get_json(app: &TestApp, token: &str, path: &str) -> reqwest::Response {
    app.client
        .get(format!("{}{path}", app.base))
        .bearer_auth(token)
        .send()
        .await
        .expect("request")
}
