use crate::storage::{AuthState, http, load_auth, save_auth};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use rand::RngCore;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

const REDIRECT_URI: &str = "http://localhost:3160/auth";

#[derive(Debug, Deserialize)]
struct OAuthToken { access_token: String, refresh_token: String }
#[derive(Debug, Deserialize)]
struct XboxResponse { #[serde(rename = "Token")] token: String, #[serde(rename = "DisplayClaims")] display_claims: XboxClaims }
#[derive(Debug, Deserialize)]
struct XboxClaims { xui: Vec<XboxUser> }
#[derive(Debug, Deserialize)]
struct XboxUser { uhs: String }
#[derive(Debug, Deserialize)]
struct MinecraftLogin { access_token: String, expires_in: i64 }
#[derive(Debug, Deserialize)]
struct MinecraftProfile { id: String, name: String, #[serde(default)] skins: Vec<MinecraftSkin> }
#[derive(Debug, Deserialize)]
struct MinecraftSkin { url: String }

pub fn login(client_id: &str, report: &(dyn Fn(String) + Send + Sync), cancelled: &AtomicBool) -> Result<AuthState> {
    let listener = TcpListener::bind("127.0.0.1:3160").context("Could not open the local sign-in port. Close another launcher and retry")?;
    listener.set_nonblocking(true)?;
    let verifier = random_token(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(24);
    let authorization_url = format!(
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query&scope={}&code_challenge={}&code_challenge_method=S256&state={}&prompt=select_account",
        urlencoding::encode(client_id), urlencoding::encode(REDIRECT_URI), urlencoding::encode("XboxLive.signin offline_access"), urlencoding::encode(&challenge), urlencoding::encode(&state)
    );
    report("Complete sign-in in your browser …".into());
    open::that(authorization_url).context("Could not open your browser")?;
    let code = wait_for_redirect(&listener, &state, cancelled)?;
    report("Checking Xbox Live account …".into());
    let client = http()?;
    let token: OAuthToken = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[("client_id", client_id), ("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", REDIRECT_URI), ("code_verifier", &verifier)])
        .send()?.error_for_status()?.json()?;
    let auth = minecraft_authenticate(&client, &token.access_token, &token.refresh_token, report)?;
    save_auth(&auth)?;
    Ok(auth)
}

pub fn ensure_session(client_id: &str) -> Result<AuthState> {
    let auth = load_auth().context("Sign in with Microsoft first")?;
    let client = http()?;
    let valid = client.get("https://api.minecraftservices.com/minecraft/profile").header(AUTHORIZATION, format!("Bearer {}", auth.minecraft_access_token)).send().is_ok_and(|response| response.status().is_success());
    if valid && auth.expires_at > Utc::now() + Duration::seconds(60) { return Ok(auth); }
    let token: OAuthToken = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", &auth.microsoft_refresh_token), ("scope", "XboxLive.signin offline_access")])
        .send()?.error_for_status()?.json()?;
    let refreshed = minecraft_authenticate(&client, &token.access_token, &token.refresh_token, &|_| {}).context("Could not refresh Microsoft session")?;
    save_auth(&refreshed)?;
    Ok(refreshed)
}

fn wait_for_redirect(listener: &TcpListener, expected_state: &str, cancelled: &AtomicBool) -> Result<String> {
    let deadline = Instant::now() + StdDuration::from_secs(300);
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Relaxed) { bail!("Sign-in cancelled") }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut request = [0u8; 8192];
                let length = stream.read(&mut request)?;
                let request_text = String::from_utf8_lossy(&request[..length]);
                let path = request_text.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
                let params = parse_query(path);
                let response = if params.get("state").is_some_and(|value| value == expected_state) && params.contains_key("code") {
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<h2>Signed in</h2><p>You can return to Wisdom.</p>".as_slice()
                } else {
                    b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<h2>Sign-in failed</h2><p>You can close this tab.</p>".as_slice()
                };
                stream.write_all(response)?;
                if let Some(error) = params.get("error") { bail!("Microsoft: {}", params.get("error_description").unwrap_or(error)); }
                if params.get("state").is_none_or(|value| value != expected_state) { bail!("Sign-in security check failed") }
                return params.get("code").cloned().context("Microsoft did not return a sign-in code");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(StdDuration::from_millis(100)),
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Sign-in timed out")
}

fn parse_query(path: &str) -> std::collections::HashMap<String, String> {
    path.split_once('?').map(|(_, query)| query.split('&').filter_map(|item| item.split_once('=').and_then(|(key, value)| Some((urlencoding::decode(key).ok()?.into_owned(), urlencoding::decode(value).ok()?.into_owned())))).collect()).unwrap_or_default()
}

fn random_token(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn minecraft_authenticate(client: &Client, microsoft_token: &str, refresh_token: &str, report: &(dyn Fn(String) + Send + Sync)) -> Result<AuthState> {
    let xbl: XboxResponse = post_json(client, "Xbox Live", "https://user.auth.xboxlive.com/user/authenticate", json!({"Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}")}, "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"}))?;
    report("Checking Xbox permissions …".into());
    let xsts: XboxResponse = post_json(client, "Xbox permissions", "https://xsts.auth.xboxlive.com/xsts/authorize", json!({"Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl.token]}, "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"}))?;
    let uhs = xsts.display_claims.xui.first().context("Xbox account has no user identifier")?.uhs.clone();
    report("Signing into Minecraft …".into());
    let minecraft: MinecraftLogin = post_json(client, "Minecraft", "https://api.minecraftservices.com/authentication/login_with_xbox", json!({"identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)}))?;
    let entitlement: Value = client.get("https://api.minecraftservices.com/entitlements/mcstore").header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    if entitlement["items"].as_array().is_none_or(|items| items.is_empty()) { bail!("This Microsoft account does not own Minecraft: Java Edition") }
    let profile: MinecraftProfile = client.get("https://api.minecraftservices.com/minecraft/profile").header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    Ok(AuthState { minecraft_access_token: minecraft.access_token, microsoft_refresh_token: refresh_token.to_owned(), expires_at: Utc::now() + Duration::seconds(minecraft.expires_in), player_name: profile.name, player_uuid: profile.id, skin_url: profile.skins.first().map(|skin| skin.url.clone()) })
}

fn post_json<T: for<'a> Deserialize<'a>>(client: &Client, service: &str, url: &str, body: Value) -> Result<T> {
    let response = client.post(url).json(&body).send()?;
    if !response.status().is_success() {
        let status = response.status(); let error: Value = response.json().unwrap_or_default();
        if service == "Xbox permissions" { match error["XErr"].as_i64() { Some(2148916233) => bail!("Xbox profile required. Sign in at xbox.com once and create a gamertag."), Some(2148916238) => bail!("Xbox family settings are blocking this account."), Some(2148916235) => bail!("This Xbox account is not available in your country."), _ => {} } }
        bail!("{service} returned {status}: {}", error["Message"].as_str().or_else(|| error["errorMessage"].as_str()).unwrap_or("No additional details"));
    }
    Ok(response.json()?)
}
