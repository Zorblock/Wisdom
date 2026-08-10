use crate::storage::{AuthState, http, load_auth, save_auth};
use anyhow::{Context, Result, bail};
use chrono::{Duration, Utc};
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration as StdDuration;

#[derive(Debug, Deserialize)]
struct DeviceCode { device_code: String, user_code: String, verification_uri: String, interval: Option<u64> }
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
struct MinecraftProfile { id: String, name: String }

pub fn login(
    client_id: &str,
    report: &(dyn Fn(String) + Send + Sync),
    present_code: &(dyn Fn(String) + Send + Sync),
    cancelled: &AtomicBool,
) -> Result<AuthState> {
    let client = http()?;
    report("Microsoft-Anmeldung wird geöffnet …".into());
    let device: DeviceCode = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
        .form(&[("client_id", client_id), ("scope", "XboxLive.signin offline_access")])
        .send()?.error_for_status()?.json()?;
    present_code(device.user_code.clone());
    report("Browser geöffnet. Anmeldung läuft …".into());
    let _ = open::that(&device.verification_uri);
    let token = poll_device_code(&client, client_id, &device, cancelled)?;
    let auth = minecraft_authenticate(&client, &token.access_token, &token.refresh_token)?;
    save_auth(&auth)?;
    Ok(auth)
}

pub fn ensure_session(client_id: &str) -> Result<AuthState> {
    let auth = load_auth().context("Melde dich zuerst mit Microsoft an")?;
    let client = http()?;
    let valid = client.get("https://api.minecraftservices.com/minecraft/profile")
        .header(AUTHORIZATION, format!("Bearer {}", auth.minecraft_access_token)).send()
        .is_ok_and(|response| response.status().is_success());
    if valid && auth.expires_at > Utc::now() + Duration::seconds(60) { return Ok(auth); }
    let refreshed = refresh_microsoft_session(client_id, &auth).context("Microsoft-Sitzung konnte nicht erneuert werden")?;
    save_auth(&refreshed)?;
    Ok(refreshed)
}

fn poll_device_code(client: &Client, client_id: &str, device: &DeviceCode, cancelled: &AtomicBool) -> Result<OAuthToken> {
    let interval = device.interval.unwrap_or(5).max(2);
    for _ in 0..180 {
        for _ in 0..(interval * 5) {
            if cancelled.load(Ordering::Relaxed) { bail!("Anmeldung abgebrochen") }
            thread::sleep(StdDuration::from_millis(200));
        }
        let response = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[("client_id", client_id), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", &device.device_code)])
            .send()?;
        let status = response.status();
        if status.is_success() { return Ok(response.json()?); }
        let body: Value = response.json().unwrap_or_else(|_| json!({}));
        match body["error"].as_str() {
            Some("authorization_pending") | Some("slow_down") => continue,
            Some(error) => bail!("Microsoft: {}", body["error_description"].as_str().unwrap_or(error)),
            None => bail!("Microsoft antwortete mit {status}"),
        }
    }
    bail!("Der Microsoft-Anmeldecode ist abgelaufen")
}

fn refresh_microsoft_session(client_id: &str, auth: &AuthState) -> Result<AuthState> {
    let client = http()?;
    let token: OAuthToken = client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", &auth.microsoft_refresh_token), ("scope", "XboxLive.signin offline_access")])
        .send()?.error_for_status()?.json()?;
    minecraft_authenticate(&client, &token.access_token, &token.refresh_token)
}

fn minecraft_authenticate(client: &Client, microsoft_token: &str, refresh_token: &str) -> Result<AuthState> {
    let xbl: XboxResponse = post_json(client, "https://user.auth.xboxlive.com/user/authenticate", json!({
        "Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}")}, "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"
    }))?;
    let xsts: XboxResponse = post_json(client, "https://xsts.auth.xboxlive.com/xsts/authorize", json!({
        "Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl.token]}, "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"
    }))?;
    let uhs = xsts.display_claims.xui.first().context("Xbox-Konto enthält keine Benutzerkennung")?.uhs.clone();
    let minecraft: MinecraftLogin = post_json(client, "https://api.minecraftservices.com/authentication/login_with_xbox", json!({"identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)}))?;
    let entitlement: Value = client.get("https://api.minecraftservices.com/entitlements/mcstore").header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    if entitlement["items"].as_array().is_none_or(|items| items.is_empty()) { bail!("Dieses Microsoft-Konto besitzt keine Minecraft-Java-Lizenz") }
    let profile: MinecraftProfile = client.get("https://api.minecraftservices.com/minecraft/profile").header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token)).send()?.error_for_status()?.json()?;
    Ok(AuthState { minecraft_access_token: minecraft.access_token, microsoft_refresh_token: refresh_token.to_owned(), expires_at: Utc::now() + Duration::seconds(minecraft.expires_in), player_name: profile.name, player_uuid: profile.id })
}

fn post_json<T: for<'a> Deserialize<'a>>(client: &Client, url: &str, body: Value) -> Result<T> {
    let response = client.post(url).json(&body).send()?;
    if !response.status().is_success() { bail!("Dienst antwortete mit {}", response.status()); }
    Ok(response.json()?)
}
