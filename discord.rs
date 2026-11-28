//! Discord voice activity bridge adapted from Overlayed 0.6.2 (AGPL-3.0).
//! This module only reads Discord RPC metadata/events. It never opens a voice
//! media connection and never captures, transmits, records or analyses audio.
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use serde_json::{json, Value};
use tungstenite::{connect, Message};
use tungstenite::client::IntoClientRequest;

const APP_ID: &str = "905987126099836938";
const RPC_URL: &str = "ws://127.0.0.1:6463/?v=1&client_id=905987126099836938";
const TOKEN_URL: &str = "https://api.overlayed.dev/token";

#[derive(Clone, Debug, Default)]
pub struct VoiceSnapshot {
    pub channel: String,
    pub guild: String,
    pub users: Vec<VoiceUser>,
    pub connected: bool,
    pub error: String,
}
#[derive(Clone, Debug, Default)]
pub struct VoiceUser {
    pub name: String,
    pub avatar_hash: String,
    pub user_id: String,
    pub speaking: bool,
    pub muted: bool,
    pub deafened: bool,
    pub self_muted: bool,
    pub self_deafened: bool,
}
static STATE: OnceLock<Mutex<VoiceSnapshot>> = OnceLock::new();
fn state() -> &'static Mutex<VoiceSnapshot> { STATE.get_or_init(|| Mutex::new(VoiceSnapshot::default())) }
pub fn snapshot() -> VoiceSnapshot { state().lock().map(|s| s.clone()).unwrap_or_default() }
pub fn start() {
    #[cfg(windows)]
    { let _ = thread::Builder::new().name("trofeo-discord-rpc".into()).spawn(run_loop); }
}
fn token_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    base.join("TrofeoLCD").join("discord_access_token.txt")
}
fn log_line(message: &str) {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let dir = base.join("TrofeoLCD"); let _ = fs::create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("discord_rpc.log")) {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), message);
    }
}
fn set_error(s: impl Into<String>) {
    let message = s.into();
    log_line(&message);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("erro.txt")) {
                let _ = writeln!(f, "[{}] Discord Voice: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), message);
            }
        }
    }
    if let Ok(mut v) = state().lock() { v.connected = false; v.error = message; }
}
fn send<S>(socket: &mut tungstenite::WebSocket<S>, cmd: &str, args: Value, evt: Option<&str>)
where
    S: std::io::Read + std::io::Write,
{
    let mut p = json!({"cmd":cmd,"args":args,"nonce":format!("trofeo-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))});
    if let Some(e) = evt { p["evt"] = json!(e); }
    let _ = socket.send(Message::Text(p.to_string()));
}
// LCD fonts do not contain Discord's custom emoji and many pictographic glyphs.
// Keep readable letters, numbers, spaces and ordinary ASCII punctuation only.
// This prevents unsupported room/user icons from being rendered as "??".
fn clean_display_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_space = false;
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            if pending_space && !out.is_empty() { out.push(' '); }
            pending_space = false;
            out.push(ch);
        } else if ch.is_whitespace() {
            pending_space = true;
        } else if ch.is_ascii_punctuation() && ch != '?' {
            if pending_space && !out.is_empty() { out.push(' '); }
            pending_space = false;
            out.push(ch);
        }
        // Emoji, pictograms, decorative symbols and custom icon glyphs are ignored.
    }
    out.trim().to_string()
}

fn name_from_voice(data: &Value) -> VoiceUser {
    let user = &data["user"];
    let vs = &data["voice_state"];
    let raw_name = data["nick"].as_str().filter(|s| !s.is_empty()).or(user["global_name"].as_str()).or(user["username"].as_str()).unwrap_or("Discord user");
    let cleaned_name = clean_display_name(raw_name);
    VoiceUser {
        name: if cleaned_name.is_empty() { "Discord user".to_string() } else { cleaned_name },
        avatar_hash: user["avatar"].as_str().unwrap_or("").to_string(),
        user_id: user["id"].as_str().unwrap_or("").to_string(),
        speaking: false,
        muted: vs["mute"].as_bool().unwrap_or(false),
        deafened: vs["deaf"].as_bool().unwrap_or(false),
        self_muted: vs["self_mute"].as_bool().unwrap_or(false),
        self_deafened: vs["self_deaf"].as_bool().unwrap_or(false),
    }
}
fn update_users(data: &Value, replace: bool) {
    if let Ok(mut s) = state().lock() {
        if replace { s.users.clear(); }
        if let Some(items) = data.as_array() {
            for item in items { let u = name_from_voice(item); if !u.user_id.is_empty() { s.users.retain(|x| x.user_id != u.user_id); s.users.push(u); } }
        } else if data.is_object() {
            let u = name_from_voice(data); if !u.user_id.is_empty() { s.users.retain(|x| x.user_id != u.user_id); s.users.push(u); }
        }
        s.users.sort_by(|a,b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
}
fn run_loop() {
    loop {
        if let Err(e) = run_once() { set_error(format!("Discord RPC: {e}")); }
        thread::sleep(Duration::from_secs(5));
    }
}
fn run_once() -> anyhow::Result<()> {
    let mut request = RPC_URL.into_client_request()?;
    request.headers_mut().insert("origin", "https://api.overlayed.dev".parse()?);
    let (mut socket, _) = connect(request)?;
    if let Ok(mut s) = state().lock() { s.connected = true; s.error.clear(); }
    let mut authenticated = false;
    let mut authorized = false;
    let mut subscribed_channel = String::new();
    loop {
        let msg = socket.read()?;
        let Message::Text(text) = msg else { if msg.is_close() { anyhow::bail!("Discord fechou a conexão RPC"); } continue; };
        let p: Value = serde_json::from_str(&text)?;
        let cmd = p["cmd"].as_str().unwrap_or("");
        let evt = p["evt"].as_str().unwrap_or("");
        if evt == "READY" && !authorized {
            let token = fs::read_to_string(token_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            if let Some(token) = token {
                send(&mut socket, "AUTHENTICATE", json!({"access_token":token}), None);
            } else {
                send(&mut socket, "AUTHORIZE", json!({"client_id":APP_ID,"scopes":["identify","rpc"]}), None);
            }
            authorized = true;
        }
        if cmd == "AUTHORIZE" && evt.is_empty() {
            if let Some(code) = p["data"]["code"].as_str() {
                let body = json!({"code":code}).to_string();
                let response = ureq::post(TOKEN_URL).set("Content-Type","application/json").send_string(&body)?.into_string()?;
                let response: Value = serde_json::from_str(&response)?;
                let token = response["access_token"].as_str().ok_or_else(|| anyhow::anyhow!("API Overlayed não retornou access_token"))?;
                if let Some(parent) = token_path().parent() { fs::create_dir_all(parent)?; }
                fs::write(token_path(), token)?;
                send(&mut socket, "AUTHENTICATE", json!({"access_token":token}), None);
            }
        }
        if cmd == "AUTHENTICATE" && evt == "ERROR" {
            let code = p["data"]["code"].as_i64().unwrap_or_default();
            if code == 4003 || code == 4004 { let _ = fs::remove_file(token_path()); }
            anyhow::bail!("Discord RPC autenticação recusada: {}", p["data"]["message"].as_str().unwrap_or("erro desconhecido"));
        }
        if cmd == "AUTHENTICATE" && evt.is_empty() {
            authenticated = true;
            log_line("Autenticado no Discord RPC; monitoramento de eventos de voz ativo.");
            send(&mut socket,"SUBSCRIBE",json!({}),Some("VOICE_CHANNEL_SELECT"));
            send(&mut socket,"GET_SELECTED_VOICE_CHANNEL",json!({}),None);
        }
        if !authenticated { continue; }
        if cmd == "GET_SELECTED_VOICE_CHANNEL" {
            let ch = &p["data"];
            if let Ok(mut s) = state().lock() {
                s.channel = clean_display_name(ch["name"].as_str().unwrap_or(""));
                s.guild = ch["guild_id"].as_str().unwrap_or("").to_string();
                s.connected = true;
            }
            if let Some(id) = ch["id"].as_str() {
                if !subscribed_channel.is_empty() && subscribed_channel != id {
                    for e in ["SPEAKING_START","SPEAKING_STOP","VOICE_STATE_CREATE","VOICE_STATE_DELETE","VOICE_STATE_UPDATE"] {
                        send(&mut socket,"UNSUBSCRIBE",json!({"channel_id":subscribed_channel}),Some(e));
                    }
                }
                if subscribed_channel != id {
                    for e in ["SPEAKING_START","SPEAKING_STOP","VOICE_STATE_CREATE","VOICE_STATE_DELETE","VOICE_STATE_UPDATE"] {
                        send(&mut socket,"SUBSCRIBE",json!({"channel_id":id}),Some(e));
                    }
                }
                subscribed_channel = id.to_string();
                update_users(&ch["voice_states"], true);
            } else {
                if !subscribed_channel.is_empty() {
                    for e in ["SPEAKING_START","SPEAKING_STOP","VOICE_STATE_CREATE","VOICE_STATE_DELETE","VOICE_STATE_UPDATE"] {
                        send(&mut socket,"UNSUBSCRIBE",json!({"channel_id":subscribed_channel}),Some(e));
                    }
                    subscribed_channel.clear();
                }
                if let Ok(mut s) = state().lock() { s.channel.clear(); s.users.clear(); }
            }
        }
        if evt == "VOICE_CHANNEL_SELECT" {
            send(&mut socket,"GET_SELECTED_VOICE_CHANNEL",json!({}),None);
        }
        if evt == "VOICE_STATE_CREATE" { update_users(&p["data"], false); }
        if evt == "VOICE_STATE_UPDATE" { update_users(&p["data"], false); }
        if evt == "VOICE_STATE_DELETE" {
            let id = p["data"]["user"]["id"].as_str().unwrap_or("");
            if let Ok(mut s) = state().lock() { s.users.retain(|u| u.user_id != id); }
        }
        if evt == "SPEAKING_START" || evt == "SPEAKING_STOP" {
            let id = p["data"]["user_id"].as_str().unwrap_or("");
            if let Ok(mut s) = state().lock() { if let Some(u) = s.users.iter_mut().find(|u| u.user_id == id) { u.speaking = evt == "SPEAKING_START"; } }
        }
    }
}
