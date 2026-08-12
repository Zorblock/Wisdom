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

#[derive(Debug, Deserialize)]
struct OAuthToken {
    access_token: String,
    refresh_token: Option<String>,
}
#[derive(Debug, Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XboxClaims,
}
#[derive(Debug, Deserialize)]
struct XboxClaims {
    xui: Vec<XboxUser>,
}
#[derive(Debug, Deserialize)]
struct XboxUser {
    uhs: String,
}
#[derive(Debug, Deserialize)]
struct MinecraftLogin {
    access_token: String,
    expires_in: i64,
}
#[derive(Debug, Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<MinecraftSkin>,
}
#[derive(Debug, Deserialize)]
struct MinecraftSkin {
    url: String,
}

const CALLBACK_SUCCESS_PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <meta name="theme-color" content="#070709">
  <title>Signed in · Wisdom</title>
  <style>
    :root { color-scheme: dark; --accent: #0078d4; --line: #1e1e20; }
    * { box-sizing: border-box; }
    html, body { min-height: 100%; }
    body { margin: 0; display: grid; place-items: center; padding: 24px; color: #f4f4f5; background: #070709; font: 14px/1.5 "Segoe UI Variable", "Segoe UI", sans-serif; }
    main { width: min(100%, 420px); padding: 32px; border: 1px solid var(--line); border-radius: 10px; background: #000; }
    .mark { display: grid; place-items: center; width: 38px; height: 38px; margin-bottom: 24px; border: 1px solid color-mix(in srgb, AccentColor 55%, var(--line)); border-radius: 8px; color: AccentColor; background: color-mix(in srgb, AccentColor 10%, #000); }
    svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
    h1 { margin: 0 0 7px; font-size: 22px; font-weight: 650; letter-spacing: -.02em; }
    p { margin: 0; color: #94949c; }
    .app { margin-top: 26px; padding-top: 18px; border-top: 1px solid var(--line); color: #62626a; font-size: 12px; }
    @supports not (color: AccentColor) { .mark { color: var(--accent); border-color: #18364e; background: #06121b; } }
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></svg></div>
    <h1>Signed in</h1>
    <p>Your Minecraft account is now connected. You can return to Wisdom and close this tab.</p>
    <div class="app">Wisdom Launcher</div>
  </main>
</body>
</html>"##;

const CALLBACK_ERROR_PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="dark">
  <meta name="theme-color" content="#070709">
  <title>Sign-in failed · Wisdom</title>
  <style>
    :root { color-scheme: dark; --line: #1e1e20; }
    * { box-sizing: border-box; }
    html, body { min-height: 100%; }
    body { margin: 0; display: grid; place-items: center; padding: 24px; color: #f4f4f5; background: #070709; font: 14px/1.5 "Segoe UI Variable", "Segoe UI", sans-serif; }
    main { width: min(100%, 420px); padding: 32px; border: 1px solid var(--line); border-radius: 10px; background: #000; }
    .mark { display: grid; place-items: center; width: 38px; height: 38px; margin-bottom: 24px; border: 1px solid #482124; border-radius: 8px; color: #ff6961; background: #19090a; }
    svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; }
    h1 { margin: 0 0 7px; font-size: 22px; font-weight: 650; letter-spacing: -.02em; }
    p { margin: 0; color: #94949c; }
    .app { margin-top: 26px; padding-top: 18px; border-top: 1px solid var(--line); color: #62626a; font-size: 12px; }
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M7 7l10 10M17 7 7 17"/></svg></div>
    <h1>Sign-in failed</h1>
    <p>Wisdom could not complete the sign-in. You can close this tab and try again in the launcher.</p>
    <div class="app">Wisdom Launcher</div>
  </main>
</body>
</html>"##;

pub fn login(
    client_id: &str,
    report: &(dyn Fn(String) + Send + Sync),
    cancelled: &AtomicBool,
) -> Result<AuthState> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("Could not open the local sign-in port")?;
    listener.set_nonblocking(true)?;
    let redirect_uri = format!("http://localhost:{}", listener.local_addr()?.port());
    let verifier = random_token(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(24);
    let authorization_url = format!(
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query&scope={}&code_challenge={}&code_challenge_method=S256&state={}&prompt=select_account",
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode("XboxLive.signin offline_access"),
        urlencoding::encode(&challenge),
        urlencoding::encode(&state)
    );
    report("Complete sign-in in your browser...".into());
    open::that(authorization_url).context("Could not open the browser")?;
    let code = wait_for_redirect(&listener, &state, cancelled)?;
    report("Checking Xbox Live account...".into());
    let client = http()?;
    let token: OAuthToken = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[
            ("client_id", client_id),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", &verifier),
        ])
        .send()?
        .error_for_status()?
        .json()?;
    let refresh_token = token
        .refresh_token
        .context("Microsoft did not return a persistent sign-in token")?;
    let auth = minecraft_authenticate(&client, &token.access_token, &refresh_token, report)?;
    save_auth(&auth)?;
    Ok(auth)
}

pub fn ensure_session(client_id: &str) -> Result<AuthState> {
    let auth = load_auth().context("Sign in with Microsoft first")?;
    if auth.expires_at > Utc::now() + Duration::minutes(5) {
        return Ok(auth);
    }
    let client = http()?;
    let token: OAuthToken = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", &auth.microsoft_refresh_token),
            ("scope", "XboxLive.signin offline_access"),
        ])
        .send()?
        .error_for_status()?
        .json()?;
    let refresh_token = token
        .refresh_token
        .as_deref()
        .unwrap_or(&auth.microsoft_refresh_token);
    let refreshed = minecraft_authenticate(&client, &token.access_token, refresh_token, &|_| {})
        .context("Could not refresh the Microsoft session")?;
    save_auth(&refreshed)?;
    Ok(refreshed)
}

fn wait_for_redirect(
    listener: &TcpListener,
    expected_state: &str,
    cancelled: &AtomicBool,
) -> Result<String> {
    let deadline = Instant::now() + StdDuration::from_secs(300);
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Sign-in cancelled")
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut request = [0u8; 8192];
                let length = stream.read(&mut request)?;
                let request_text = String::from_utf8_lossy(&request[..length]);
                let path = request_text
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("");
                let params = parse_query(path);
                let (status, response) = if params
                    .get("state")
                    .is_some_and(|value| value == expected_state)
                    && params.contains_key("code")
                {
                    ("200 OK", CALLBACK_SUCCESS_PAGE)
                } else {
                    ("400 Bad Request", CALLBACK_ERROR_PAGE)
                };
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                stream.write_all(headers.as_bytes())?;
                stream.write_all(response.as_bytes())?;
                if let Some(error) = params.get("error") {
                    bail!(
                        "Microsoft: {}",
                        params.get("error_description").unwrap_or(error)
                    );
                }
                if params
                    .get("state")
                    .is_none_or(|value| value != expected_state)
                {
                    bail!("Sign-in security check failed")
                }
                return params
                    .get("code")
                    .cloned()
                    .context("Microsoft did not return a sign-in code");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(StdDuration::from_millis(100))
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Sign-in timed out")
}

fn parse_query(path: &str) -> std::collections::HashMap<String, String> {
    path.split_once('?')
        .map(|(_, query)| {
            query
                .split('&')
                .filter_map(|item| {
                    item.split_once('=').and_then(|(key, value)| {
                        Some((
                            urlencoding::decode(key).ok()?.into_owned(),
                            urlencoding::decode(value).ok()?.into_owned(),
                        ))
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn random_token(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn minecraft_authenticate(
    client: &Client,
    microsoft_token: &str,
    refresh_token: &str,
    report: &(dyn Fn(String) + Send + Sync),
) -> Result<AuthState> {
    let xbl: XboxResponse = post_json(
        client,
        "Xbox Live",
        "https://user.auth.xboxlive.com/user/authenticate",
        json!({"Properties": {"AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={microsoft_token}")}, "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT"}),
    )?;
    report("Checking Xbox permissions...".into());
    let xsts: XboxResponse = post_json(
        client,
        "Xbox permissions",
        "https://xsts.auth.xboxlive.com/xsts/authorize",
        json!({"Properties": {"SandboxId": "RETAIL", "UserTokens": [xbl.token]}, "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT"}),
    )?;
    let uhs = xsts
        .display_claims
        .xui
        .first()
        .context("The Xbox account has no user identifier")?
        .uhs
        .clone();
    report("Signing in to Minecraft...".into());
    let minecraft: MinecraftLogin = post_json(
        client,
        "Minecraft",
        "https://api.minecraftservices.com/authentication/login_with_xbox",
        json!({"identityToken": format!("XBL3.0 x={uhs};{}", xsts.token)}),
    )?;
    let entitlement: Value = client
        .get("https://api.minecraftservices.com/entitlements/mcstore")
        .header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token))
        .send()?
        .error_for_status()?
        .json()?;
    if entitlement["items"]
        .as_array()
        .is_none_or(|items| items.is_empty())
    {
        bail!("This Microsoft account does not own Minecraft: Java Edition")
    }
    let profile: MinecraftProfile = client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .header(AUTHORIZATION, format!("Bearer {}", minecraft.access_token))
        .send()?
        .error_for_status()?
        .json()?;
    Ok(AuthState {
        minecraft_access_token: minecraft.access_token,
        microsoft_refresh_token: refresh_token.to_owned(),
        expires_at: Utc::now() + Duration::seconds(minecraft.expires_in),
        player_name: profile.name,
        player_uuid: profile.id,
        skin_url: profile.skins.first().map(|skin| skin.url.clone()),
    })
}

fn post_json<T: for<'a> Deserialize<'a>>(
    client: &Client,
    service: &str,
    url: &str,
    body: Value,
) -> Result<T> {
    let response = client.post(url).json(&body).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let error: Value = response.json().unwrap_or_default();
        if service == "Xbox permissions" {
            match error["XErr"].as_i64() {
                Some(2148916233) => {
                    bail!(
                        "This account has no Xbox profile. Sign in at xbox.com once and create a gamertag."
                    )
                }
                Some(2148916238) => {
                    bail!("Xbox family settings are blocking this account.")
                }
                Some(2148916235) => bail!("This Xbox account is not available in your country."),
                _ => {}
            }
        }
        bail!(
            "{service} returned {status}: {}",
            error["Message"]
                .as_str()
                .or_else(|| error["errorMessage"].as_str())
                .unwrap_or("No additional details")
        );
    }
    Ok(response.json()?)
}
