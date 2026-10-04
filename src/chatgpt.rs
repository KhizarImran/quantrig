//! ChatGPT plan OAuth. Tokens stay in an owner-only file outside the sandbox.
use crate::store;
use axum::{extract::Query, response::Html, routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation, Algorithm};
use ring::{digest, rand::{SecureRandom, SystemRandom}};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::OnceLock, time::{Duration, SystemTime, UNIX_EPOCH}};
use tokio::sync::Mutex;

type Result<T> = std::result::Result<T, String>;
const ISSUER: &str = "https://auth.openai.com";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const REDIRECT: &str = "http://127.0.0.1:1455/auth/callback";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
static CONNECTION: Mutex<()> = Mutex::const_new(());
static FLOW: Mutex<Option<Pending>> = Mutex::const_new(None);
static FLOW_ERROR: Mutex<Option<String>> = Mutex::const_new(None);
static LISTENER: OnceLock<bool> = OnceLock::new();

#[derive(Default, Serialize, Deserialize)]
struct Account {
    host_id: String,
    #[serde(default)] client_id: String,
    #[serde(default)] subject: String,
    #[serde(default)] email: String,
    #[serde(default)] access_token: String,
    #[serde(default)] refresh_token: String,
    #[serde(default)] id_token: String,
    #[serde(default)] scopes: Vec<String>,
    #[serde(default)] expires_at: u64,
    #[serde(default)] needs_sign_in: bool,
}
struct Pending {
    state: String, nonce: String, verifier: String, expires_at: u64,
    client_id: String, subject: String,
}
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
fn path() -> PathBuf { store::data_dir().join("chatgpt.json") }
fn load() -> Result<Account> {
    match std::fs::read(path()) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| "Could not read saved ChatGPT connection".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Account::default()),
        Err(_) => Err("Could not read saved ChatGPT connection".into()),
    }
}
fn save(account: &Account) -> Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(store::data_dir()).map_err(|e| e.to_string())?;
    let tmp = path().with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?; }
    file.write_all(&serde_json::to_vec_pretty(account).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path()).map_err(|e| e.to_string())
}
fn random() -> Result<String> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "Could not generate secure sign-in values")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn host_id() -> Result<String> {
    let mut b = [0; 16];
    SystemRandom::new().fill(&mut b).map_err(|_| "Could not generate host ID")?;
    b[6] = (b[6] & 15) | 64; b[8] = (b[8] & 63) | 128;
    let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
    Ok(format!("urn:uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
}
fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).build().expect("HTTP client")
}

pub async fn status() -> Value {
    let _guard = CONNECTION.lock().await;
    match load() {
        Ok(a) => {
            let mut flow = FLOW.lock().await;
            if flow.as_ref().is_some_and(|p| p.expires_at <= now()) {
                *flow = None;
                *FLOW_ERROR.lock().await = Some("Sign-in expired. Please try again.".into());
            }
            json!({"connected": !a.id_token.is_empty() && !a.needs_sign_in,
                "email": a.email, "plan_enabled": a.scopes.iter().any(|s| s == PLAN_SCOPE),
                "needs_sign_in": a.needs_sign_in, "pending": flow.is_some(),
                "error": FLOW_ERROR.lock().await.clone(), "callback_available": LISTENER.get() == Some(&true)})
        }
        Err(e) => json!({"connected": false, "plan_enabled": false, "pending": false, "error": e, "callback_available": LISTENER.get() == Some(&true)}),
    }
}

pub async fn start() -> Result<Value> {
    if LISTENER.get() != Some(&true) { return Err("ChatGPT callback listener is unavailable. Check port 1455 and restart Quantrig.".into()); }
    let _guard = CONNECTION.lock().await;
    if FLOW.lock().await.as_ref().is_some_and(|p| p.expires_at > now()) {
        return Err("A sign-in is already pending. Finish or cancel it first.".into());
    }
    let mut a = load()?;
    if a.host_id.is_empty() { a.host_id = host_id()?; save(&a)?; }
    let p = Pending { state: random()?, nonce: random()?, verifier: random()?,
        expires_at: now() + 600, client_id: a.client_id.clone(), subject: a.subject.clone() };
    let mut url = reqwest::Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
    let challenge = URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, p.verifier.as_bytes()));
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", if p.client_id.is_empty() { "dynamic_agent_client" } else { &p.client_id })
            .append_pair("ext_agent_host_id", &a.host_id).append_pair("response_type", "code")
            .append_pair("redirect_uri", REDIRECT).append_pair("scope", "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct")
            .append_pair("resource", RESOURCE).append_pair("state", &p.state).append_pair("nonce", &p.nonce)
            .append_pair("code_challenge_method", "S256").append_pair("code_challenge", &challenge);
        if p.client_id.is_empty() { q.append_pair("agent_name_hint", "Quantrig"); }
        // Avoid sending retained ID tokens to the browser in the returned URL.
        if !a.email.is_empty() { q.append_pair("login_hint", &a.email); }
        if !a.client_id.is_empty() && !a.scopes.iter().any(|s| s == PLAN_SCOPE) { q.append_pair("prompt", "consent"); }
    }
    *FLOW.lock().await = Some(p);
    *FLOW_ERROR.lock().await = None;
    Ok(json!({"url": url.as_str(), "expires_in": 600}))
}

#[derive(Deserialize)]
struct Callback { state: Option<String>, code: Option<String>, client_id: Option<String>, error: Option<String> }
fn validate_callback(p: &Pending, q: &Callback) -> Result<String> {
    if p.expires_at <= now() { return Err("Sign-in expired. Start again in Settings.".into()); }
    if q.state.as_deref() != Some(&p.state) { return Err("Sign-in state did not match.".into()); }
    if q.error.is_some() { return Err("ChatGPT sign-in was declined. Try again when ready.".into()); }
    if q.code.as_deref().is_none_or(str::is_empty) { return Err("No authorization code was returned.".into()); }
    let id = q.client_id.as_deref().unwrap_or(&p.client_id);
    if id.is_empty() || id == "dynamic_agent_client" || (!p.client_id.is_empty() && id != p.client_id) {
        return Err("ChatGPT registration did not return the expected client ID.".into());
    }
    Ok(id.to_string())
}

#[derive(Clone, Deserialize)]
struct Identity { sub: String, nonce: String, #[serde(default)] email: String, iat: u64 }
fn verify_identity(token: &str, keys: &JwkSet, client: &str, nonce: &str) -> Result<Identity> {
    let header = decode_header(token).map_err(|_| "Invalid ChatGPT identity token")?;
    if header.alg != Algorithm::RS256 { return Err("Unsupported identity signature".into()); }
    let key = keys.find(header.kid.as_deref().ok_or("Identity token has no key ID")?).ok_or("Identity signing key not found")?;
    let mut v = Validation::new(Algorithm::RS256);
    v.set_issuer(&[ISSUER]); v.set_audience(&[client]); v.leeway = 5;
    v.set_required_spec_claims(&["exp", "iss", "aud", "sub", "iat"]);
    let claims = decode::<Identity>(token, &DecodingKey::from_jwk(key).map_err(|_| "Invalid signing key")?, &v)
        .map_err(|_| "ChatGPT identity verification failed")?.claims;
    if claims.nonce != nonce || claims.sub.is_empty() || claims.iat > now() + 5 { return Err("ChatGPT identity claims did not match".into()); }
    Ok(claims)
}
async fn token_request(form: &[(&str, &str)]) -> Result<Value> {
    let res = http().post(TOKEN).form(form).send().await.map_err(|_| "Could not reach ChatGPT sign-in. Try again.")?;
    let status = res.status();
    let body: Value = res.json().await.map_err(|_| "Unexpected ChatGPT token response")?;
    if !status.is_success() {
        return Err(if matches!(body["error"].as_str(), Some("invalid_grant" | "invalid_refresh_token" | "token_expired" | "refresh_token_expired" | "refresh_token_invalidated" | "refresh_token_reused")) { "ChatGPT session expired or was revoked. Sign in again." } else { "ChatGPT token request failed. Please sign in again." }.into());
    }
    Ok(body)
}
fn apply_tokens(a: &mut Account, body: &Value, refresh: bool) -> Result<()> {
    let scopes = body["scope"].as_str().map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_else(|| if refresh { a.scopes.clone() } else { Vec::new() });
    let access = body["access_token"].as_str().filter(|s| !s.is_empty());
    if scopes.iter().any(|s| s == PLAN_SCOPE) && access.is_none() { return Err("ChatGPT did not return a plan access token".into()); }
    let expires = if access.is_some() {
        if !body["token_type"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("bearer")) { return Err("Unexpected token type".into()); }
        body["expires_in"].as_u64().filter(|n| *n > 0 && *n <= 86400 * 30).ok_or("ChatGPT did not return a valid token expiry")?
    } else { 0 };
    a.access_token = access.unwrap_or("").into(); a.expires_at = now() + expires;
    a.needs_sign_in = false; a.scopes = scopes;
    if let Some(token) = body["refresh_token"].as_str().filter(|s| !s.is_empty()) { a.refresh_token = token.into(); }
    else if !refresh { a.refresh_token.clear(); }
    Ok(())
}
async fn complete(q: Callback) -> Result<()> {
    let _guard = CONNECTION.lock().await;
    let mut flow = FLOW.lock().await;
    let p = flow.as_ref().ok_or("No pending sign-in. Start again in Settings.")?;
    // A stray callback must not consume someone else's pending attempt.
    if q.state.as_deref() != Some(&p.state) { return Err("Sign-in state did not match.".into()); }
    let p = flow.take().unwrap(); drop(flow);
    let client = validate_callback(&p, &q)?;
    let body = token_request(&[("grant_type", "authorization_code"), ("client_id", &client),
        ("code", q.code.as_deref().unwrap()), ("code_verifier", &p.verifier), ("redirect_uri", REDIRECT), ("resource", RESOURCE)]).await?;
    let id_token = body["id_token"].as_str().ok_or("ChatGPT did not return an identity token")?;
    let keys: JwkSet = http().get(format!("{ISSUER}/.well-known/jwks.json")).send().await.map_err(|_| "Could not load ChatGPT signing keys")?
        .error_for_status().map_err(|_| "Could not load ChatGPT signing keys")?.json().await.map_err(|_| "Invalid ChatGPT signing keys")?;
    let identity = verify_identity(id_token, &keys, &client, &p.nonce)?;
    if !p.subject.is_empty() && identity.sub != p.subject { return Err("This sign-in belongs to a different account. Reconnect with the original account.".into()); }
    let mut a = load()?;
    a.client_id = client; a.subject = identity.sub; a.email = identity.email; a.id_token = id_token.into();
    apply_tokens(&mut a, &body, false)?;
    save(&a)
}
async fn callback(Query(q): Query<Callback>) -> Html<&'static str> {
    match complete(q).await {
        Ok(()) => { *FLOW_ERROR.lock().await = None; Html("<!doctype html><title>Quantrig</title><body style='background:#171717;color:#eee;font:16px system-ui;padding:48px'><h1>ChatGPT connected</h1><p>Return to Quantrig Settings. You can close this tab.</p></body>") }
        Err(e) => { *FLOW_ERROR.lock().await = Some(e); Html("<!doctype html><title>Quantrig</title><body style='background:#171717;color:#eee;font:16px system-ui;padding:48px'><h1>Sign-in could not finish</h1><p>Return to Quantrig Settings for details and try again.</p></body>") }
    }
}
pub async fn listen() {
    let addr = std::env::var("QUANTRIG_CHATGPT_CALLBACK_ADDR").unwrap_or_else(|_| "127.0.0.1:1455".into());
    match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => { let _ = LISTENER.set(true); tokio::spawn(async move {
            let _ = axum::serve(listener, Router::new().route("/auth/callback", get(callback))).await;
        }); }
        Err(_) => { let _ = LISTENER.set(false); eprintln!("ChatGPT callback port unavailable; other connectors remain available"); }
    }
}
pub async fn access_token() -> Result<String> {
    let _guard = CONNECTION.lock().await;
    let mut a = load()?;
    if a.id_token.is_empty() || a.needs_sign_in { return Err("Connect ChatGPT in Settings first.".into()); }
    if !a.scopes.iter().any(|s| s == PLAN_SCOPE) { return Err("ChatGPT plan usage was not enabled. Continue with ChatGPT in Settings and allow plan usage.".into()); }
    if a.expires_at <= now() + 60 {
        let body = token_request(&[("grant_type", "refresh_token"), ("client_id", &a.client_id), ("refresh_token", &a.refresh_token), ("resource", RESOURCE)]).await;
        match body {
            Ok(body) => { apply_tokens(&mut a, &body, true)?; save(&a)?; }
            Err(e) => {
                if e.contains("expired or was revoked") { a.needs_sign_in = true; a.access_token.clear(); a.refresh_token.clear(); a.id_token.clear(); save(&a)?; }
                return Err(e);
            }
        }
    }
    Ok(a.access_token)
}
pub async fn cancel() {
    let _guard = CONNECTION.lock().await;
    *FLOW.lock().await = None;
    *FLOW_ERROR.lock().await = None;
}
pub async fn disconnect() -> Result<Value> {
    let _guard = CONNECTION.lock().await;
    let mut a = load()?;
    let confirmed = if a.refresh_token.is_empty() { true } else {
        async {
            let discovery: Value = http().get(format!("{ISSUER}/.well-known/openid-configuration")).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
            let endpoint = discovery["revocation_endpoint"].as_str()?;
            let url = reqwest::Url::parse(endpoint).ok()?;
            if url.scheme() != "https" || url.host_str() != Some("auth.openai.com") { return None; }
            let res = http().post(url).form(&[("token", &a.refresh_token), ("token_type_hint", &"refresh_token".into()), ("client_id", &a.client_id)]).send().await.ok()?;
            Some(res.status() == reqwest::StatusCode::OK)
        }.await.unwrap_or(false)
    };
    a.access_token.clear(); a.refresh_token.clear(); a.id_token.clear(); a.scopes.clear(); a.expires_at = 0; a.needs_sign_in = false;
    save(&a)?; *FLOW.lock().await = None; *FLOW_ERROR.lock().await = None;
    Ok(json!({"revoked": confirmed, "warning": if confirmed { Value::Null } else { json!("Disconnected locally. Remote revocation was not confirmed; remove Quantrig access in ChatGPT Settings.") }}))
}
pub async fn invalidate(token: &str) {
    let _guard = CONNECTION.lock().await;
    if let Ok(mut a) = load() {
        if a.access_token == token { a.needs_sign_in = true; let _ = save(&a); }
    }
}

pub async fn models() -> Result<Value> {
    let key = access_token().await?;
    let res = http().get(format!("{RESOURCE}/models")).bearer_auth(&key).send().await.map_err(|_| "Could not load ChatGPT models")?;
    if res.status() == reqwest::StatusCode::UNAUTHORIZED { invalidate(&key).await; }
    if !res.status().is_success() { return Err("ChatGPT model access failed. Check plan permissions or sign in again.".into()); }
    let body: Value = res.json().await.map_err(|_| "Invalid ChatGPT model catalogue")?;
    let models: Vec<Value> = body["models"].as_array().ok_or("ChatGPT model catalogue was missing")?.iter()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| Some(json!({"id": m["slug"].as_str()?, "name": m["display_name"].as_str().unwrap_or(m["slug"].as_str()?)}))).collect();
    Ok(json!({"data": models}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn identity_token(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256); header.kid = Some("test-key".into());
        // Public test fixture only: never used by the app or any real account.
        encode(&header, claims, &EncodingKey::from_rsa_pem(include_bytes!("testdata/chatgpt-test-key.pem")).unwrap()).unwrap()
    }
    #[test]
    fn identity_requires_signature_issuer_audience_expiry_nonce_and_subject() {
        let keys: JwkSet = serde_json::from_str(include_str!("testdata/chatgpt-test-jwks.json")).unwrap();
        let claims = json!({"iss": ISSUER, "aud": "oaiapp_test", "sub": "test-user", "email": "test@example.com", "nonce": "nonce", "iat": now(), "exp": now()+300});
        assert_eq!(verify_identity(&identity_token(&claims), &keys, "oaiapp_test", "nonce").unwrap().sub, "test-user");
        for (field, value) in [("iss", json!("https://attacker.example")), ("aud", json!("other-app")),
            ("nonce", json!("wrong")), ("sub", json!("")), ("exp", json!(now()-100)), ("iat", json!(now()+300))] {
            let mut bad = claims.clone(); bad[field] = value;
            assert!(verify_identity(&identity_token(&bad), &keys, "oaiapp_test", "nonce").is_err(), "accepted bad {field}");
        }
        let signed = identity_token(&claims);
        let mut segments: Vec<String> = signed.split('.').map(str::to_string).collect();
        let mut altered = claims.clone(); altered["sub"] = json!("attacker");
        segments[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&altered).unwrap());
        assert!(verify_identity(&segments.join("."), &keys, "oaiapp_test", "nonce").is_err());
        assert!(verify_identity(&signed, &keys, "oaiapp_test", "different-nonce").is_err());
    }
    fn pending(client: &str) -> Pending { Pending { state: "state".into(), nonce: "nonce".into(), verifier: "verifier".into(), expires_at: now()+600, client_id: client.into(), subject: String::new() } }
    fn callback_query() -> Callback { Callback { state: Some("state".into()), code: Some("code".into()), client_id: Some("oaiapp_test".into()), error: None } }
    #[test]
    fn callback_checks_state_expiry_denial_and_registration_binding() {
        assert_eq!(validate_callback(&pending(""), &callback_query()).unwrap(), "oaiapp_test");
        let mut q = callback_query(); q.state = Some("wrong".into()); assert!(validate_callback(&pending(""), &q).is_err());
        let mut q = callback_query(); q.error = Some("access_denied".into()); assert!(validate_callback(&pending(""), &q).is_err());
        let mut q = callback_query(); q.code = None; assert!(validate_callback(&pending(""), &q).is_err());
        let mut q = callback_query(); q.client_id = None; assert!(validate_callback(&pending(""), &q).is_err());
        assert_eq!(validate_callback(&pending("oaiapp_test"), &q).unwrap(), "oaiapp_test");
        assert!(validate_callback(&pending("different-client"), &callback_query()).is_err());
        let mut p = pending(""); p.expires_at = now()-1; assert!(validate_callback(&p, &callback_query()).is_err());
    }
    #[tokio::test]
    async fn wrong_state_does_not_consume_flow_and_denial_is_one_time() {
        *FLOW.lock().await = Some(pending(""));
        let mut q = callback_query(); q.state = Some("wrong".into());
        assert!(complete(q).await.is_err()); assert!(FLOW.lock().await.is_some());
        let mut q = callback_query(); q.error = Some("access_denied".into());
        assert!(complete(q).await.is_err()); assert!(FLOW.lock().await.is_none());
        assert!(complete(callback_query()).await.is_err());
    }
    #[test]
    fn protected_credentials_round_trip_and_refresh_keeps_registration() {
        let _guard = store::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("qr-chatgpt-test-{}", std::process::id()));
        unsafe { std::env::set_var("QUANTRIG_DATA", &dir) };
        let mut a = Account { host_id: host_id().unwrap(), client_id: "oaiapp_test".into(), subject: "test-user".into(), ..Default::default() };
        apply_tokens(&mut a, &json!({"access_token": "test-access", "refresh_token": "test-refresh", "scope": PLAN_SCOPE, "token_type": "Bearer", "expires_in": 3600}), false).unwrap();
        save(&a).unwrap();
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(path()).unwrap().permissions().mode() & 0o777, 0o600); }
        let mut saved = load().unwrap();
        apply_tokens(&mut saved, &json!({"access_token": "replacement", "refresh_token": "rotated", "token_type": "bearer", "expires_in": 1800}), true).unwrap();
        save(&saved).unwrap();
        let saved = load().unwrap();
        assert_eq!(saved.client_id, "oaiapp_test"); assert_eq!(saved.subject, "test-user"); assert_eq!(saved.host_id, a.host_id);
        assert_eq!(saved.access_token, "replacement"); assert_eq!(saved.refresh_token, "rotated"); assert_eq!(saved.scopes, vec![PLAN_SCOPE]);
        let mut a = Account::default();
        apply_tokens(&mut a, &json!({"access_token": "identity-only", "token_type": "Bearer", "expires_in": 3600}), false).unwrap();
        assert!(!a.scopes.iter().any(|s| s == PLAN_SCOPE));
        assert!(apply_tokens(&mut a, &json!({"access_token": "a", "token_type": "Bearer"}), false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        unsafe { std::env::remove_var("QUANTRIG_DATA") };
    }
}
