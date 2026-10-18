//! OAuth2: signing in to Google and Microsoft mailboxes without a password.
//!
//! Gmail and Microsoft 365 / Outlook.com want a token, not your password,
//! from mail apps (Microsoft refuses passwords outright). Like Thunderbird:
//!
//! 1. **Sign-in window** (authorization code flow with PKCE): the web UI
//!    opens the provider's sign-in page in a pop-up; once you accept, the
//!    provider sends the browser back to Havre
//!    (`/api/oauth/{provider}/callback?code=…`), and Havre trades the code
//!    for an *access token* (valid ~1 hour) and a *refresh token* (valid for
//!    months). The address it comes back to must be registered with the
//!    provider ("redirect URI").
//!
//!    Microsoft only: a **device code** instead (type a short code at
//!    microsoft.com/devicelogin), for when the window can't be used.
//! 2. **XOAUTH2**: to log in over IMAP, Havre sends the access token instead
//!    of a password.
//! 3. When the access token expires, the refresh token gets a new one —
//!    no need to sign in again. Only the refresh token is stored, encrypted.
//!
//! Each provider needs Havre to be registered once (see docs/providers.md): a
//! Microsoft Entra "app registration", a Google Cloud "OAuth client".

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{Error, Result, store::Database};

/// Who signs you in. Stored in `accounts.auth` (`password` otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Microsoft,
    Google,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Microsoft, Provider::Google];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microsoft => "microsoft",
            Self::Google => "google",
        }
    }

    /// For messages: "Microsoft", "Google".
    pub fn name(self) -> &'static str {
        match self {
            Self::Microsoft => "Microsoft",
            Self::Google => "Google",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }

    /// The provider's IMAP server — as in Thunderbird, the server decides
    /// whose sign-in to use.
    pub fn for_host(host: &str) -> Option<Self> {
        let host = host.trim().to_lowercase();
        if host.ends_with("office365.com") || host.ends_with("outlook.com") {
            Some(Self::Microsoft)
        } else if host.ends_with("gmail.com") || host.ends_with("googlemail.com") {
            Some(Self::Google)
        } else {
            None
        }
    }
}

/// How Havre is registered with a provider, from `.env`.
#[derive(Debug, Clone)]
pub struct OAuthApp {
    pub provider: Provider,
    pub client_id: String,
    /// Google only. For an app installed on many servers it isn't really
    /// secret (PKCE is what protects the sign-in), but Google requires it.
    pub client_secret: Option<String>,
    authorize_url: String,
    token_url: String,
    /// Microsoft only: the device code endpoint.
    device_url: Option<String>,
    /// The provider's IMAP server (overridable for tests).
    pub imap_host: String,
    pub imap_port: u16,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

impl OAuthApp {
    /// Every provider configured in `.env`.
    pub fn all_from_env() -> Vec<Self> {
        [Self::microsoft_from_env(), Self::google_from_env()].into_iter().flatten().collect()
    }

    /// `HAVRE_MICROSOFT_CLIENT_ID` (required) and `HAVRE_MICROSOFT_TENANT`:
    /// `common` (default: work and personal accounts), `organizations`, or
    /// your domain / tenant id for a single-tenant registration.
    pub fn microsoft_from_env() -> Option<Self> {
        let client_id = env("HAVRE_MICROSOFT_CLIENT_ID")?;
        let tenant = env("HAVRE_MICROSOFT_TENANT").unwrap_or_else(|| "common".into());
        let login =
            env("HAVRE_MICROSOFT_LOGIN_URL").unwrap_or_else(|| "https://login.microsoftonline.com".into());
        let base = format!("{}/{tenant}/oauth2/v2.0", login.trim_end_matches('/'));
        Some(Self {
            provider: Provider::Microsoft,
            client_id,
            client_secret: None,
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            device_url: Some(format!("{base}/devicecode")),
            imap_host: env("HAVRE_MICROSOFT_IMAP_HOST").unwrap_or_else(|| "outlook.office365.com".into()),
            imap_port: env("HAVRE_MICROSOFT_IMAP_PORT").and_then(|v| v.parse().ok()).unwrap_or(993),
        })
    }

    /// `HAVRE_GOOGLE_CLIENT_ID` and `HAVRE_GOOGLE_CLIENT_SECRET` (both required).
    pub fn google_from_env() -> Option<Self> {
        let client_id = env("HAVRE_GOOGLE_CLIENT_ID")?;
        Some(Self {
            provider: Provider::Google,
            client_id,
            client_secret: env("HAVRE_GOOGLE_CLIENT_SECRET"),
            authorize_url: env("HAVRE_GOOGLE_AUTH_URL")
                .unwrap_or_else(|| "https://accounts.google.com/o/oauth2/v2/auth".into()),
            token_url: env("HAVRE_GOOGLE_TOKEN_URL")
                .unwrap_or_else(|| "https://oauth2.googleapis.com/token".into()),
            device_url: None, // Google's device flow doesn't allow mail access
            imap_host: env("HAVRE_GOOGLE_IMAP_HOST").unwrap_or_else(|| "imap.gmail.com".into()),
            imap_port: env("HAVRE_GOOGLE_IMAP_PORT").and_then(|v| v.parse().ok()).unwrap_or(993),
        })
    }

    /// What we ask permission for: reading mail (IMAP), sending it (SMTP),
    /// and who you are (to fill in the address). Microsoft also needs
    /// `offline_access` for a refresh token; Google uses `access_type=offline`
    /// in the link instead, and its mail scope covers IMAP and SMTP.
    /// The calendar (Agenda) and tasks too: Microsoft Graph (Calendars,
    /// To Do), Google Calendar and Google Tasks.
    pub fn scopes(&self) -> &'static str {
        match self.provider {
            // Mail first: Microsoft gives the first sign-in token for the
            // first resource listed (outlook.office.com), which IMAP needs.
            Provider::Microsoft => {
                "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send \
                 https://graph.microsoft.com/Calendars.ReadWrite \
                 https://graph.microsoft.com/Tasks.ReadWrite offline_access openid email profile"
            }
            Provider::Google => {
                "https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/tasks \
                 openid email"
            }
        }
    }

    /// Microsoft issues one token per "resource" (mail server, Graph): the
    /// scopes to ask for when refreshing a token for `resource`, among those
    /// the sign-in was granted.
    fn scopes_for(&self, granted: Option<&str>, resource: Resource) -> String {
        let granted = granted.unwrap_or(self.legacy_scopes());
        match self.provider {
            Provider::Google => granted.to_owned(),
            Provider::Microsoft => {
                let prefix = match resource {
                    Resource::Mail => "https://outlook.office.com/",
                    Resource::Calendar => "https://graph.microsoft.com/",
                };
                let mut scopes: Vec<&str> =
                    granted.split_whitespace().filter(|s| s.starts_with(prefix)).collect();
                scopes.push("offline_access");
                scopes.join(" ")
            }
        }
    }

    /// Can a sign-in with these scopes use Microsoft To Do / Google Tasks?
    /// (Signed in before Tasks existed: reconnect once.)
    pub fn can_tasks(&self, granted: Option<&str>) -> bool {
        granted.is_some_and(|s| match self.provider {
            Provider::Microsoft => s.contains("Tasks.ReadWrite"),
            Provider::Google => s.contains("auth/tasks"),
        })
    }

    /// Can a sign-in with these scopes use the calendar?
    pub fn can_calendar(&self, granted: Option<&str>) -> bool {
        granted.is_some_and(|s| match self.provider {
            Provider::Microsoft => s.contains("Calendars.ReadWrite"),
            Provider::Google => s.contains("auth/calendar"),
        })
    }

    /// What sign-ins made before sending existed were granted (Microsoft
    /// asks again for anything more: those accounts must reconnect to send).
    fn legacy_scopes(&self) -> &'static str {
        match self.provider {
            Provider::Microsoft => {
                "https://outlook.office.com/IMAP.AccessAsUser.All offline_access openid email profile"
            }
            Provider::Google => "https://mail.google.com/ openid email",
        }
    }

    /// Can a sign-in with these scopes send mail?
    pub fn can_send(&self, granted: Option<&str>) -> bool {
        match self.provider {
            Provider::Microsoft => granted.is_some_and(|s| s.contains("SMTP.Send")),
            Provider::Google => true,
        }
    }

    /// Can it sign in with a code typed on another page?
    pub fn supports_device_code(&self) -> bool {
        self.device_url.is_some()
    }

    /// POSTs a form to the token endpoint, adding the client id (and secret).
    async fn token_request(&self, form: &[(&str, &str)]) -> Result<std::result::Result<Tokens, TokenError>> {
        let mut fields = vec![("client_id", self.client_id.as_str())];
        if let Some(secret) = &self.client_secret {
            fields.push(("client_secret", secret));
        }
        fields.extend_from_slice(form);
        let response =
            http().post(&self.token_url).form(&fields).send().await.map_err(|e| self.network(e))?;
        if response.status().is_success() {
            return Ok(Ok(response.json().await.map_err(|e| self.network(e))?));
        }
        Ok(Err(response.json().await.unwrap_or_default()))
    }

    fn network(&self, err: reqwest::Error) -> Error {
        Error::Config(format!("Can't reach {}: {err}", self.provider.name()))
    }
}

pub(crate) fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("Havre/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client")
    })
}

/// Tokens from the provider.
#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub id_token: Option<String>,
}

// ---- Sign-in window: authorization code + PKCE ----------------------------------------

/// PKCE ("pixy"): a random secret (`verifier`) whose hash (`challenge`) goes
/// in the sign-in link. Only whoever knows the secret can redeem the code,
/// so a code intercepted on the way back is useless.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
        use sha2::Digest;
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verifier = b64.encode(bytes);
        let challenge = b64.encode(sha2::Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// The sign-in page to open. `state` comes back unchanged with the code: it
/// says which sign-in this is.
pub fn authorize_url(
    app: &OAuthApp,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    login_hint: Option<&str>,
) -> String {
    let mut params = vec![
        ("client_id", app.client_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", app.scopes()),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    match app.provider {
        Provider::Microsoft => params.extend([("response_mode", "query"), ("prompt", "select_account")]),
        // `offline` + `consent`: Google only gives a refresh token when the
        // consent screen is shown.
        Provider::Google => params.extend([("access_type", "offline"), ("prompt", "consent select_account")]),
    }
    if let Some(hint) = login_hint.filter(|h| h.contains('@')) {
        params.push(("login_hint", hint));
    }
    reqwest::Url::parse_with_params(&app.authorize_url, &params).map(String::from).unwrap_or_default()
}

/// Trades the code the provider sent back for tokens.
pub async fn exchange_code(app: &OAuthApp, code: &str, redirect_uri: &str, verifier: &str) -> Result<Tokens> {
    // Microsoft: ask for the mail token explicitly (one resource per token).
    let mail_scope = app.scopes_for(Some(app.scopes()), Resource::Mail);
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
    ];
    if app.provider == Provider::Microsoft {
        form.push(("scope", &mail_scope));
    }
    app.token_request(&form).await?.map_err(|e| Error::SignIn(explain(app.provider, &e)))
}

/// An error sent back to the redirect URI (`?error=…&error_description=…`).
pub fn explain_redirect_error(provider: Provider, error: &str, description: &str) -> String {
    if error == "access_denied" && !description.contains("AADSTS65001") {
        return "The sign-in was cancelled.".into();
    }
    explain(provider, &TokenError { error: error.to_owned(), error_description: description.to_owned() })
}

// ---- Device code flow (Microsoft) ------------------------------------------------------

/// What Microsoft gives to start a sign-in with a code.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCode {
    /// Secret: only used by Havre to poll. Never sent to the browser.
    #[serde(skip_serializing)]
    pub device_code: String,
    /// What you type at `verification_uri`, e.g. `F7KQ-2PLM`.
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    /// Seconds to wait between two polls.
    #[serde(default = "five")]
    pub interval: u64,
}

fn five() -> u64 {
    5
}

pub async fn start_device_flow(app: &OAuthApp) -> Result<DeviceCode> {
    let url = app
        .device_url
        .as_ref()
        .ok_or_else(|| Error::Config(format!("{} has no sign-in with a code", app.provider.name())))?;
    let response = http()
        .post(url)
        .form(&[("client_id", app.client_id.as_str()), ("scope", app.scopes())])
        .send()
        .await
        .map_err(|e| app.network(e))?;
    if !response.status().is_success() {
        let error: TokenError = response.json().await.unwrap_or_default();
        return Err(Error::Config(explain(app.provider, &error)));
    }
    response.json().await.map_err(|e| app.network(e))
}

/// The answer to one poll.
#[derive(Debug)]
pub enum Poll {
    /// You haven't approved yet.
    Pending,
    /// Polling too fast: wait longer.
    SlowDown,
    /// Approved.
    Done(Tokens),
    /// Refused, expired, or an error (with a message for humans).
    Failed(String),
}

pub async fn poll_device_flow(app: &OAuthApp, device_code: &str) -> Result<Poll> {
    let form = [("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", device_code)];
    Ok(match app.token_request(&form).await? {
        Ok(tokens) => Poll::Done(tokens),
        Err(error) => match error.error.as_str() {
            "authorization_pending" => Poll::Pending,
            "slow_down" => Poll::SlowDown,
            "authorization_declined" => Poll::Failed("The sign-in was declined.".into()),
            "expired_token" | "code_expired" => Poll::Failed("The code expired. Start again.".into()),
            _ => Poll::Failed(explain(app.provider, &error)),
        },
    })
}

// ---- Refreshing -------------------------------------------------------------------------

/// Gets a new access token from a refresh token. Microsoft usually sends a
/// new refresh token too; Google keeps the same one.
/// `scope` (Microsoft): what to ask for — within what the sign-in was
/// granted, and for one resource only.
pub async fn refresh(app: &OAuthApp, refresh_token: &str, scope: Option<&str>) -> Result<Tokens> {
    let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", refresh_token)];
    if app.provider == Provider::Microsoft {
        form.push(("scope", scope.unwrap_or(app.legacy_scopes())));
    }
    match app.token_request(&form).await? {
        Ok(tokens) => Ok(tokens),
        Err(error) if error.error == "invalid_grant" => Err(Error::SignIn(match app.provider {
            Provider::Microsoft => {
                "Microsoft sign-in expired or was revoked: reconnect this account in Settings.".into()
            }
            Provider::Google => "Google sign-in expired or was revoked: reconnect this account in Settings. \
                 (While the Google Cloud app is in “Testing”, Google ends sign-ins after 7 days: \
                 publish it — see docs/providers.md.)"
                .into(),
        })),
        Err(error) => Err(Error::SignIn(explain(app.provider, &error))),
    }
}

#[derive(Debug, Default, Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Turns the providers' error codes into advice.
fn explain(provider: Provider, error: &TokenError) -> String {
    let d = &error.error_description;
    let first_line = d.lines().next().unwrap_or_default();
    let advice = match provider {
        Provider::Microsoft => {
            if d.contains("AADSTS65001") || d.contains("AADSTS90094") || d.contains("consent") {
                Some(format!(
                    "Your organisation requires an administrator to approve Havre first ({first_line})."
                ))
            } else if d.contains("AADSTS50011") {
                Some(
                    "The redirect URI isn't in the app registration: add the address Havre shows to \
                     Authentication › Mobile and desktop applications."
                        .into(),
                )
            } else if d.contains("AADSTS7000218") {
                Some(
                    "In the app registration, enable “Allow public client flows” (Authentication page)."
                        .into(),
                )
            } else if d.contains("AADSTS700016") || d.contains("AADSTS90002") {
                Some(format!(
                    "Microsoft doesn't know this app or tenant: check HAVRE_MICROSOFT_CLIENT_ID and \
                     HAVRE_MICROSOFT_TENANT ({first_line})."
                ))
            } else if d.contains("AADSTS50194") {
                Some(
                    "This app registration is single-tenant: set HAVRE_MICROSOFT_TENANT to your domain \
                     (e.g. lmcontrol.fr)."
                        .into(),
                )
            } else {
                None
            }
        }
        Provider::Google => match error.error.as_str() {
            "invalid_client" | "unauthorized_client" => Some(
                "Google doesn't recognise this client: check HAVRE_GOOGLE_CLIENT_ID and \
                 HAVRE_GOOGLE_CLIENT_SECRET."
                    .into(),
            ),
            "redirect_uri_mismatch" => Some(
                "The redirect URI isn't in the Google OAuth client: add the address Havre shows to \
                 “Authorized redirect URIs”."
                    .into(),
            ),
            "admin_policy_enforced" => {
                Some("Your Google Workspace administrator doesn't allow Havre to access Gmail.".into())
            }
            "org_internal" => {
                Some("This Google Cloud app is limited to its own organisation (“Internal”).".into())
            }
            _ => None,
        },
    };
    advice.unwrap_or_else(|| {
        if !first_line.is_empty() {
            first_line.to_owned()
        } else if !error.error.is_empty() {
            format!("{} refused: {}", provider.name(), error.error)
        } else {
            format!("{} refused the request.", provider.name())
        }
    })
}

/// Your name inside an ID token, e.g. "Maël Belliard".
pub fn name_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned)
}

/// The email address inside an ID token. The token is only *read* (not
/// verified): it just pre-fills the account; the IMAP login is the real check.
pub fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["email", "preferred_username", "upn"]
        .iter()
        .find_map(|key| claims.get(key).and_then(|v| v.as_str()))
        .filter(|v| v.contains('@'))
        .map(str::to_owned)
}

// ---- Which provider hosts an address? ------------------------------------------------------

/// Gmail and Outlook addresses are known; for a company domain, its mail
/// servers (DNS "MX" records) tell Google Workspace and Microsoft 365 apart.
/// Microsoft's public "user realm" lookup covers the remaining Microsoft 365
/// setups. Any error counts as "don't know".
pub async fn detect_provider(email: &str) -> Option<Provider> {
    let domain = email.rsplit_once('@')?.1.trim().to_lowercase();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => return Some(Provider::Google),
        "outlook.com" | "outlook.fr" | "hotmail.com" | "hotmail.fr" | "live.com" | "live.fr" | "msn.com" => {
            return Some(Provider::Microsoft);
        }
        _ => {}
    }
    if let Some(provider) = provider_from_mx(&domain).await {
        return Some(provider);
    }
    is_microsoft_address(email).await.then_some(Provider::Microsoft)
}

async fn provider_from_mx(domain: &str) -> Option<Provider> {
    use hickory_resolver::TokioAsyncResolver;
    let resolver = TokioAsyncResolver::tokio_from_system_conf().ok()?;
    let lookup = tokio::time::timeout(Duration::from_secs(4), resolver.mx_lookup(domain)).await.ok()?.ok()?;
    lookup.iter().find_map(|mx| {
        let host = mx.exchange().to_utf8().to_lowercase();
        let host = host.trim_end_matches('.');
        if host.ends_with("google.com") || host.ends_with("googlemail.com") {
            Some(Provider::Google)
        } else if host.ends_with("mail.protection.outlook.com") {
            Some(Provider::Microsoft)
        } else {
            None
        }
    })
}

/// Asks Microsoft's public "user realm" endpoint (the one its own sign-in
/// page uses) whether Microsoft 365 knows this address.
async fn is_microsoft_address(email: &str) -> bool {
    #[derive(Deserialize)]
    struct Realm {
        #[serde(rename = "NameSpaceType", default)]
        namespace_type: String,
    }
    let url = "https://login.microsoftonline.com/getuserrealm.srf";
    let request = http().get(url).query(&[("login", email), ("json", "1")]).timeout(Duration::from_secs(4));
    match request.send().await {
        Ok(r) => r
            .json::<Realm>()
            .await
            .map(|realm| matches!(realm.namespace_type.as_str(), "Managed" | "Federated"))
            .unwrap_or(false),
        Err(_) => false,
    }
}

// ---- IMAP login with a token -------------------------------------------------------

/// The SASL XOAUTH2 response: `user=<email>^Aauth=Bearer <token>^A^A`.
pub fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

/// Answers the server during `AUTHENTICATE XOAUTH2`. If the token is
/// refused, the server sends an error challenge and expects an empty reply.
pub struct XOAuth2 {
    response: String,
    sent: bool,
}

impl XOAuth2 {
    pub fn new(user: &str, token: &str) -> Self {
        Self { response: xoauth2(user, token), sent: false }
    }
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        if self.sent {
            String::new()
        } else {
            self.sent = true;
            self.response.clone()
        }
    }
}

// ---- Keeping an account's access token fresh ------------------------------------------

/// Gives a valid access token for one account, refreshing it when needed
/// and saving the rotated refresh token.
pub struct OAuthSession {
    app: OAuthApp,
    account_id: i64,
    db: Database,
    /// The current access token and when it stops being valid.
    cache: Mutex<Option<(String, Instant)>>,
    resource: Resource,
}

/// What a token is for. Microsoft needs a separate token for each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// IMAP and SMTP.
    Mail,
    /// The Agenda and Tasks (Microsoft Graph; Google Calendar and Tasks).
    Calendar,
}

impl OAuthSession {
    /// Tokens for reading and sending mail.
    pub fn new(app: OAuthApp, account_id: i64, db: Database) -> Self {
        Self::for_resource(app, account_id, db, Resource::Mail)
    }

    pub fn for_resource(app: OAuthApp, account_id: i64, db: Database, resource: Resource) -> Self {
        Self { app, account_id, db, cache: Mutex::new(None), resource }
    }

    pub fn provider(&self) -> Provider {
        self.app.provider
    }

    pub async fn access_token(&self) -> Result<String> {
        // The lock also makes concurrent callers wait for a single refresh.
        let mut cache = self.cache.lock().await;
        if let Some((token, valid_until)) = cache.as_ref()
            && Instant::now() < *valid_until
        {
            return Ok(token.clone());
        }
        let refresh_token = self.db.refresh_token(self.account_id).await?;
        let granted = self.db.oauth_scope(self.account_id).await?;
        let scope = self.app.scopes_for(granted.as_deref(), self.resource);
        let tokens = refresh(&self.app, &refresh_token, Some(&scope)).await?;
        if let Some(rotated) = &tokens.refresh_token {
            self.db.set_refresh_token(self.account_id, rotated).await?;
        }
        // Renew 5 minutes before it actually expires.
        let valid_for = Duration::from_secs(tokens.expires_in.saturating_sub(300).max(60));
        *cache = Some((tokens.access_token.clone(), Instant::now() + valid_for));
        Ok(tokens.access_token)
    }

    /// Forget the cached token (the server refused it).
    pub async fn forget(&self) {
        *self.cache.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_xoauth2_and_answers_errors_with_empty_line() {
        assert_eq!(xoauth2("a@b.fr", "T0K"), "user=a@b.fr\x01auth=Bearer T0K\x01\x01");
        let mut auth = XOAuth2::new("a@b.fr", "T0K");
        use async_imap::Authenticator;
        assert_eq!(auth.process(b""), xoauth2("a@b.fr", "T0K"));
        assert_eq!(auth.process(b"{\"status\":\"400\"}"), "");
    }

    #[test]
    fn builds_the_sign_in_link_with_pkce() {
        let base = "https://login.microsoftonline.com/lmcontrol.fr/oauth2/v2.0";
        let app = OAuthApp {
            provider: Provider::Microsoft,
            client_id: "abc".into(),
            client_secret: None,
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            device_url: Some(format!("{base}/devicecode")),
            imap_host: "outlook.office365.com".into(),
            imap_port: 993,
        };
        let pkce = Pkce::new();
        assert_eq!(pkce.verifier.len(), 43);
        assert_ne!(pkce.verifier, pkce.challenge);
        let url = authorize_url(&app, "http://localhost:9988/cb", "s1", &pkce.challenge, Some("a@b.fr"));
        assert!(url.starts_with("https://login.microsoftonline.com/lmcontrol.fr/oauth2/v2.0/authorize?"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A9988%2Fcb"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("login_hint=a%40b.fr"));
        assert!(!url.contains("access_type"));
    }

    #[test]
    fn microsoft_tokens_are_asked_one_resource_at_a_time() {
        let app = OAuthApp {
            provider: Provider::Microsoft,
            client_id: "abc".into(),
            client_secret: None,
            authorize_url: String::new(),
            token_url: String::new(),
            device_url: None,
            imap_host: "outlook.office365.com".into(),
            imap_port: 993,
        };
        let granted = Some(app.scopes());
        let mail = app.scopes_for(granted, Resource::Mail);
        assert!(
            mail.contains("IMAP.AccessAsUser.All") && mail.contains("SMTP.Send") && !mail.contains("graph")
        );
        let calendar = app.scopes_for(granted, Resource::Calendar);
        assert_eq!(
            calendar,
            "https://graph.microsoft.com/Calendars.ReadWrite https://graph.microsoft.com/Tasks.ReadWrite offline_access"
        );
        assert!(app.can_calendar(granted) && app.can_send(granted) && app.can_tasks(granted));
        // Signed in before sending and the Agenda existed.
        assert_eq!(
            app.scopes_for(None, Resource::Mail),
            "https://outlook.office.com/IMAP.AccessAsUser.All offline_access"
        );
        assert!(!app.can_calendar(None) && !app.can_send(None));
    }

    #[test]
    fn google_asks_for_a_refresh_token() {
        let app = OAuthApp {
            provider: Provider::Google,
            client_id: "g".into(),
            client_secret: Some("s".into()),
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: String::new(),
            device_url: None,
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
        };
        let url = authorize_url(&app, "http://localhost:9988/cb", "s1", "c", None);
        assert!(url.contains("access_type=offline"));
        assert!(url.contains(
            "scope=https%3A%2F%2Fmail.google.com%2F+https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar+\
             https%3A%2F%2Fwww.googleapis.com%2Fauth%2Ftasks+openid+email"
        ));
        assert!(!app.supports_device_code());
        assert_eq!(Provider::for_host("imap.gmail.com"), Some(Provider::Google));
        assert_eq!(Provider::for_host("outlook.office365.com"), Some(Provider::Microsoft));
        assert_eq!(Provider::for_host("ssl0.ovh.net"), None);
    }

    #[test]
    fn reads_email_from_id_token() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"preferred_username":"mael.belliard@lmcontrol.fr","name":"Maël"}"#);
        let token = format!("eyJhbGciOiJub25lIn0.{payload}.sig");
        assert_eq!(email_from_id_token(&token).as_deref(), Some("mael.belliard@lmcontrol.fr"));
        assert_eq!(email_from_id_token("not-a-jwt"), None);
    }

    #[test]
    fn explains_common_microsoft_errors() {
        let consent = TokenError {
            error: "invalid_grant".into(),
            error_description: "AADSTS65001: The user or administrator has not consented.\r\nTrace ID: x"
                .into(),
        };
        assert!(explain(Provider::Microsoft, &consent).contains("administrator to approve"));
        let public =
            TokenError { error: "invalid_client".into(), error_description: "AADSTS7000218: …".into() };
        assert!(explain(Provider::Microsoft, &public).contains("Allow public client flows"));
        let google = TokenError { error: "invalid_client".into(), error_description: String::new() };
        assert!(explain(Provider::Google, &google).contains("HAVRE_GOOGLE_CLIENT_SECRET"));
    }
}
