//! Mail accounts: adding, editing, removing, and one sync engine per account.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::{
    Error, Result,
    config::{ImapAuth, ImapConfig},
    mail::imap,
    oauth::{self, DeviceCode, OAuthApp, Poll, Provider},
    store::{Account, AccountUpdate, Database, NewAccount},
    sync::{AccountEvent, SyncEngine, SyncEvent, SyncState, SyncStatus},
};

/// An account plus what the UI shows next to it.
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    #[serde(flatten)]
    pub account: Account,
    pub status: SyncStatus,
    /// Unread messages in the inbox.
    pub unread: u32,
}

pub struct AccountManager {
    db: Database,
    engines: RwLock<HashMap<i64, Arc<SyncEngine>>>,
    events: broadcast::Sender<AccountEvent>,
    /// OAuth2 sign-ins in progress, by flow id.
    sign_ins: Mutex<HashMap<String, SignIn>>,
}

/// An OAuth2 sign-in in progress.
struct SignIn {
    app: OAuthApp,
    target: Target,
    method: Method,
    expires: Instant,
}

/// What the sign-in is for: a new account, or reconnecting one.
#[derive(Clone)]
struct Target {
    name: String,
    email: String,
    /// Your name for recipients (empty: the one from the sign-in).
    sender_name: String,
    color: String,
    /// Reconnecting this existing account instead of adding one.
    reconnect: Option<i64>,
    /// The IMAP server to open with the token.
    host: String,
    port: u16,
    /// Empty: the address you sign in with.
    user: String,
}

enum Method {
    /// The provider's page in a pop-up window, like Thunderbird. It sends
    /// the browser back to `redirect_uri` (our `/api/oauth/{provider}/callback`).
    Browser {
        /// PKCE secret: proves the code is redeemed by whoever started.
        verifier: String,
        redirect_uri: String,
        /// `None` until the provider sends the browser back.
        state: Option<SignInState>,
    },
    /// Microsoft: a code to type at microsoft.com/devicelogin (works from anywhere).
    Device { device: DeviceCode, next_poll: Instant },
}

/// The add-account form, with "Authentication: OAuth2" chosen. Like
/// Thunderbird, the server settings stay editable, and the server decides
/// the provider (`imap.gmail.com` → Google); empty = the provider's.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct OAuthRequest {
    pub name: String,
    pub email: String,
    pub sender_name: String,
    pub color: String,
    pub imap_host: String,
    pub imap_port: Option<u16>,
    pub imap_user: String,
    /// Id of an existing account to sign in again (the rest is ignored).
    pub reconnect: Option<i64>,
    /// Who signs you in. Default: guessed from `imap_host`.
    pub provider: Option<Provider>,
    /// `browser` (default: the provider's page in a window) or `code` (Microsoft).
    pub mode: String,
    /// Where the web UI is open, e.g. `http://localhost:9988`: the provider
    /// sends the browser back there. `HAVRE_PUBLIC_URL` overrides it.
    pub origin: String,
}

/// What the UI gets when it starts an OAuth2 sign-in.
#[derive(Debug, Clone, Serialize)]
pub struct SignInStart {
    /// To poll `/api/accounts/oauth/{flow}`.
    pub provider: Provider,
    pub flow: String,
    /// Browser mode: the sign-in page to open in a window…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorize_url: Option<String>,
    /// …and where it comes back (must be in the app registration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
    /// Code mode: what to type, and where.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub code: Option<DeviceCode>,
}

/// Where an OAuth2 sign-in is at.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum SignInState {
    /// Waiting for you on the provider's page.
    Pending,
    /// The provider said yes; opening the mailbox to check.
    Connecting,
    Done {
        // Boxed: an `Account` is much bigger than the other variants.
        account: Box<Account>,
    },
    Failed {
        message: String,
    },
}

/// The path the provider sends the browser back to.
pub fn callback_path(provider: Provider) -> String {
    format!("/api/oauth/{}/callback", provider.as_str())
}

impl AccountManager {
    /// Starts syncing every account in the database.
    pub async fn start(db: Database) -> Result<Arc<Self>> {
        let manager = Arc::new(Self {
            db,
            engines: RwLock::new(HashMap::new()),
            events: broadcast::channel(256).0,
            sign_ins: Mutex::new(HashMap::new()),
        });
        for account in manager.db.accounts().await? {
            if let Err(err) = manager.start_engine(account.id).await {
                tracing::warn!(account = account.email, "cannot start syncing: {err}");
            }
        }
        // Wakes snoozed messages up when their time comes. A `Weak` pointer:
        // the loop must not keep the manager alive on its own.
        tokio::spawn(snooze_loop(Arc::downgrade(&manager)));
        Ok(manager)
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// Every change, of every account.
    /// For other parts of Havre (the Agenda) to announce their changes too.
    pub fn sender(&self) -> broadcast::Sender<AccountEvent> {
        self.events.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AccountEvent> {
        self.events.subscribe()
    }

    async fn start_engine(&self, id: i64) -> Result<()> {
        let config = self.db.imap_config(id).await?;
        let engine = SyncEngine::start(id, config, self.db.store(id), self.events.clone());
        if let Some(old) = self.engines.write().await.insert(id, engine) {
            old.stop();
        }
        Ok(())
    }

    /// The sync engine of an account.
    pub async fn engine(&self, id: i64) -> Result<Arc<SyncEngine>> {
        self.engines.read().await.get(&id).cloned().ok_or_else(|| {
            Error::NotFound(format!("account {id} (or its password can't be read: re-enter it in Settings)"))
        })
    }

    pub async fn accounts(&self) -> Result<Vec<AccountView>> {
        let engines = self.engines.read().await;
        let mut views = Vec::new();
        for account in self.db.accounts().await? {
            let engine = engines.get(&account.id);
            let status = engine.map(|e| e.status()).unwrap_or(SyncStatus {
                state: SyncState::Error,
                last_success: None,
                error: Some(match Provider::parse(&account.auth) {
                    Some(p) if self.db.oauth_app(p).is_none() => {
                        format!("not syncing: OAuth2 with {} isn't set up in .env", p.name())
                    }
                    Some(_) => "not syncing: reconnect this account".into(),
                    None => "not syncing: re-enter the password in Settings".into(),
                }),
                backfilling: false,
                push: false,
            });
            let unread = match engine {
                Some(e) => e
                    .store()
                    .folders()
                    .await?
                    .iter()
                    .find(|f| f.folder.role == crate::mail::FolderRole::Inbox)
                    .map_or(0, |f| f.unread),
                None => 0,
            };
            views.push(AccountView { account, status, unread });
        }
        Ok(views)
    }

    /// Checks the connection, saves the account, starts syncing it.
    pub async fn add(&self, new: NewAccount) -> Result<Account> {
        validate(&new.name, &new.email, &new.imap_host)?;
        let user = if new.imap_user.trim().is_empty() { new.email.trim() } else { new.imap_user.trim() };
        test_connection(&ImapConfig {
            host: new.imap_host.trim().to_owned(),
            port: new.imap_port,
            username: user.to_owned(),
            auth: ImapAuth::Password(new.password.clone()),
            ca_file: std::env::var("HAVRE_IMAP_CA_FILE").ok().filter(|v| !v.trim().is_empty()),
        })
        .await?;
        let account = self.db.insert_account(&new).await?;
        self.start_engine(account.id).await?;
        self.announce(account.id);
        Ok(account)
    }

    /// Edits an account. If the connection settings change, they are tested
    /// first, and syncing restarts with them.
    pub async fn update(&self, id: i64, update: AccountUpdate) -> Result<Account> {
        let current = self.db.account(id).await?;
        validate(
            update.name.as_deref().unwrap_or(&current.name),
            update.email.as_deref().unwrap_or(&current.email),
            update.imap_host.as_deref().unwrap_or(&current.imap_host),
        )?;
        if update.touches_connection() {
            if current.auth != "password" {
                return Err(Error::Config(
                    "this account signs in with OAuth2: use “Reconnect” instead of a password".into(),
                ));
            }
            let password = match &update.password {
                Some(p) => p.clone(),
                None => match self.db.imap_config(id).await?.auth {
                    ImapAuth::Password(p) => p,
                    _ => String::new(),
                },
            };
            test_connection(&ImapConfig {
                host: update.imap_host.clone().unwrap_or(current.imap_host).trim().to_owned(),
                port: update.imap_port.unwrap_or(current.imap_port),
                username: update.imap_user.clone().unwrap_or(current.imap_user).trim().to_owned(),
                auth: ImapAuth::Password(password),
                ca_file: std::env::var("HAVRE_IMAP_CA_FILE").ok().filter(|v| !v.trim().is_empty()),
            })
            .await?;
        }
        let account = self.db.update_account(id, &update).await?;
        if update.touches_connection() {
            self.start_engine(id).await?;
        }
        self.announce(id);
        Ok(account)
    }

    /// Stops syncing and deletes the account with its local copy.
    /// (Nothing is deleted on the mail server.)
    pub async fn remove(&self, id: i64) -> Result<()> {
        if let Some(engine) = self.engines.write().await.remove(&id) {
            engine.stop();
        }
        self.db.delete_account(id).await?;
        self.announce(id);
        Ok(())
    }

    /// Adds the account from the `.env` file, the first time only, so
    /// existing setups keep working after the move to accounts in Settings.
    pub async fn import_from_env(&self, config: ImapConfig) -> Result<()> {
        if !self.db.accounts().await?.is_empty() {
            return Ok(());
        }
        tracing::info!(user = config.username, "importing the account from .env");
        let ImapAuth::Password(password) = config.auth else {
            return Err(Error::Config("only password accounts can be imported from .env".into()));
        };
        let email = if config.username.contains('@') { config.username.clone() } else { String::new() };
        let account = self
            .db
            .insert_account(&NewAccount {
                name: email.split('@').nth(1).unwrap_or("Mail").to_owned(),
                email: if email.is_empty() { config.username.clone() } else { email },
                color: "work".into(),
                imap_host: config.host,
                imap_port: config.port,
                imap_user: config.username,
                password,
                sender_name: String::new(),
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
            })
            .await?;
        self.start_engine(account.id).await
    }

    // ---- OAuth2 (Google, Microsoft) --------------------------------------------------

    /// Starts a sign-in: the provider's page to open in a window (`browser`),
    /// or (Microsoft) a code to type at microsoft.com/devicelogin (`code`).
    pub async fn start_oauth(&self, request: OAuthRequest) -> Result<SignInStart> {
        let (provider, host, port, user) = match request.reconnect {
            // Reconnecting: same provider, server and user as before.
            Some(id) => {
                let account = self.db.account(id).await?;
                let provider = Provider::parse(&account.auth)
                    .ok_or_else(|| Error::Config("this account uses a password, not OAuth2".into()))?;
                (provider, account.imap_host, account.imap_port, account.imap_user)
            }
            None => {
                let host = request.imap_host.trim();
                if host.contains(' ') {
                    return Err(Error::Config("enter the IMAP server, e.g. imap.gmail.com".into()));
                }
                let provider = request.provider.or_else(|| Provider::for_host(host)).ok_or_else(|| {
                    Error::Config(
                        "OAuth2 works with Google (imap.gmail.com) and Microsoft (outlook.office365.com) \
                         servers. For this server, use a normal password."
                            .into(),
                    )
                })?;
                let default = self.db.oauth_app(provider);
                (
                    provider,
                    if host.is_empty() {
                        default.map(|a| a.imap_host.clone()).unwrap_or_default()
                    } else {
                        host.to_owned()
                    },
                    request.imap_port.or(default.map(|a| a.imap_port)).unwrap_or(993),
                    request.imap_user.trim().to_owned(),
                )
            }
        };
        let app = self.db.oauth_app(provider).cloned().ok_or_else(|| {
            let vars = match provider {
                Provider::Microsoft => "HAVRE_MICROSOFT_CLIENT_ID",
                Provider::Google => "HAVRE_GOOGLE_CLIENT_ID and HAVRE_GOOGLE_CLIENT_SECRET",
            };
            Error::Config(format!(
                "OAuth2 with {} isn't set up on this Havre: add {vars} to .env (see docs/providers.md)",
                provider.name()
            ))
        })?;
        let target = Target {
            name: request.name.trim().to_owned(),
            email: request.email.trim().to_owned(),
            sender_name: request.sender_name.trim().to_owned(),
            color: if request.color.is_empty() { "work".into() } else { request.color.clone() },
            reconnect: request.reconnect,
            host,
            port,
            user,
        };
        let flow = random_id();
        let now = Instant::now();
        let (method, expires, start) = if request.mode == "code" {
            if !app.supports_device_code() {
                return Err(Error::Config(format!("{} has no sign-in with a code", provider.name())));
            }
            let device = oauth::start_device_flow(&app).await?;
            let start = SignInStart {
                flow: flow.clone(),
                provider,
                authorize_url: None,
                redirect_uri: None,
                code: Some(device.clone()),
            };
            let expires = now + Duration::from_secs(device.expires_in);
            let next_poll = now + Duration::from_secs(device.interval);
            (Method::Device { device, next_poll }, expires, start)
        } else {
            let redirect_uri = format!("{}{}", public_url(&request.origin)?, callback_path(provider));
            let pkce = oauth::Pkce::new();
            // Whose address to suggest on the sign-in page.
            let hint = if target.user.contains('@') { &target.user } else { &target.email };
            let authorize_url =
                oauth::authorize_url(&app, &redirect_uri, &flow, &pkce.challenge, Some(hint.as_str()));
            let start = SignInStart {
                flow: flow.clone(),
                provider,
                authorize_url: Some(authorize_url),
                redirect_uri: Some(redirect_uri.clone()),
                code: None,
            };
            let method = Method::Browser { verifier: pkce.verifier, redirect_uri, state: None };
            (method, now + Duration::from_secs(15 * 60), start)
        };
        let mut sign_ins = self.sign_ins.lock().await;
        sign_ins.retain(|_, s| s.expires > now); // forget abandoned ones
        sign_ins.insert(flow, SignIn { app, target, method, expires });
        Ok(start)
    }

    /// The provider sent the browser back with `?code=…&state=…` (or an
    /// error). Redeems the code, then opens the mailbox in the background;
    /// the UI learns the outcome by polling. The error is for the pop-up.
    pub async fn oauth_callback(
        self: &Arc<Self>,
        flow: &str,
        code: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let (app, verifier, redirect_uri) = {
            let sign_ins = self.sign_ins.lock().await;
            match sign_ins.get(flow) {
                Some(SignIn {
                    app, method: Method::Browser { verifier, redirect_uri, state: None }, ..
                }) => (app.clone(), verifier.clone(), redirect_uri.clone()),
                _ => return Err(Error::SignIn("This sign-in is over. Start again from Havre.".into())),
            }
        };
        let outcome = match (code, error) {
            (Some(code), None) => oauth::exchange_code(&app, code, &redirect_uri, &verifier).await,
            (_, Some(error)) => Err(Error::SignIn(error.to_owned())),
            (None, None) => {
                Err(Error::SignIn(format!("{} didn't send a sign-in code.", app.provider.name())))
            }
        };
        let tokens = match outcome {
            Ok(tokens) => tokens,
            Err(err) => {
                self.set_state(flow, SignInState::Failed { message: err.to_string() }).await;
                return Err(err);
            }
        };
        self.set_state(flow, SignInState::Connecting).await;
        // Testing the mailbox can take a while: don't keep the window waiting.
        let manager = Arc::clone(self);
        let flow = flow.to_owned();
        tokio::spawn(async move {
            let target = match manager.sign_ins.lock().await.get(&flow) {
                Some(sign_in) => sign_in.target.clone(),
                None => return,
            };
            let state = match manager.finish_oauth(&app, &target, tokens).await {
                Ok(account) => SignInState::Done { account: Box::new(account) },
                Err(err) => SignInState::Failed { message: err.to_string() },
            };
            manager.set_state(&flow, state).await;
        });
        Ok(())
    }

    async fn set_state(&self, flow: &str, new: SignInState) {
        if let Some(SignIn { method: Method::Browser { state, .. }, .. }) =
            self.sign_ins.lock().await.get_mut(flow)
        {
            *state = Some(new);
        }
    }

    /// Called by the UI every few seconds. For a code, only asks Microsoft
    /// when the interval it asked for has passed.
    pub async fn poll_oauth(&self, flow: &str) -> Result<SignInState> {
        let mut sign_ins = self.sign_ins.lock().await;
        let Some(sign_in) = sign_ins.get_mut(flow) else {
            return Ok(SignInState::Failed { message: "This sign-in is over. Start again.".into() });
        };
        let now = Instant::now();
        if now > sign_in.expires {
            sign_ins.remove(flow);
            return Ok(SignInState::Failed { message: "The sign-in took too long. Start again.".into() });
        }
        let (device, next_poll) = match &mut sign_in.method {
            Method::Browser { state, .. } => {
                let current = state.clone().unwrap_or(SignInState::Pending);
                if matches!(current, SignInState::Done { .. } | SignInState::Failed { .. }) {
                    sign_ins.remove(flow);
                }
                return Ok(current);
            }
            Method::Device { device, next_poll } => (device, next_poll),
        };
        if now < *next_poll {
            return Ok(SignInState::Pending);
        }
        match oauth::poll_device_flow(&sign_in.app, &device.device_code).await {
            Ok(Poll::Pending) | Err(_) => {
                *next_poll = now + Duration::from_secs(device.interval);
                Ok(SignInState::Pending)
            }
            Ok(Poll::SlowDown) => {
                device.interval += 5;
                *next_poll = now + Duration::from_secs(device.interval);
                Ok(SignInState::Pending)
            }
            Ok(Poll::Failed(message)) => {
                sign_ins.remove(flow);
                Ok(SignInState::Failed { message })
            }
            Ok(Poll::Done(tokens)) => {
                let sign_in = sign_ins.remove(flow).expect("present");
                drop(sign_ins); // don't block other polls while connecting
                Ok(match self.finish_oauth(&sign_in.app, &sign_in.target, tokens).await {
                    Ok(account) => SignInState::Done { account: Box::new(account) },
                    Err(err) => SignInState::Failed { message: err.to_string() },
                })
            }
        }
    }

    /// The provider said yes: check that IMAP accepts the token, then save.
    async fn finish_oauth(&self, app: &OAuthApp, target: &Target, tokens: oauth::Tokens) -> Result<Account> {
        let name = app.provider.name();
        let refresh = tokens
            .refresh_token
            .clone()
            .ok_or_else(|| Error::SignIn(format!("{name} didn't give a refresh token. Try again.")))?;
        let signed_in =
            tokens.id_token.as_deref().and_then(oauth::email_from_id_token).ok_or_else(|| {
                Error::SignIn(format!("{name} didn't say which address you signed in with"))
            })?;
        // The IMAP user name: the one typed in the form, else the address
        // you signed in with.
        let user = if target.user.is_empty() { signed_in.clone() } else { target.user.clone() };
        if target.reconnect.is_some() && !user.eq_ignore_ascii_case(&signed_in) {
            return Err(Error::SignIn(format!(
                "You signed in as {signed_in}, but this account is {user}. Sign in with that address."
            )));
        }
        test_connection(&ImapConfig {
            host: target.host.clone(),
            port: target.port,
            username: user.clone(),
            auth: ImapAuth::Bearer(tokens.access_token),
            ca_file: std::env::var("HAVRE_IMAP_CA_FILE").ok().filter(|v| !v.trim().is_empty()),
        })
        .await?;
        let account = match target.reconnect {
            Some(id) => {
                self.db.set_oauth_sign_in(id, &refresh, app.scopes()).await?;
                self.db.account(id).await?
            }
            None => {
                let email = if target.email.contains('@') { target.email.clone() } else { signed_in };
                let name = if target.name.is_empty() {
                    email.split('@').nth(1).unwrap_or("Mail").to_owned()
                } else {
                    target.name.clone()
                };
                let new = NewAccount {
                    name,
                    email,
                    color: target.color.clone(),
                    imap_host: target.host.clone(),
                    imap_port: target.port,
                    imap_user: user,
                    password: String::new(),
                    sender_name: if target.sender_name.is_empty() {
                        tokens.id_token.as_deref().and_then(oauth::name_from_id_token).unwrap_or_default()
                    } else {
                        target.sender_name.clone()
                    },
                    smtp_host: None,
                    smtp_port: None,
                    smtp_security: None,
                };
                self.db.insert_oauth_account(&new, app.provider, &refresh, app.scopes()).await?
            }
        };
        self.start_engine(account.id).await?;
        self.announce(account.id);
        Ok(account)
    }

    fn announce(&self, id: i64) {
        let _ = self.events.send(AccountEvent { account: id, event: SyncEvent::Accounts });
    }
}

fn validate(name: &str, email: &str, host: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::Config("give the account a name".into()));
    }
    if !email.contains('@') {
        return Err(Error::Config("enter a valid email address".into()));
    }
    if host.trim().is_empty() || host.contains(' ') {
        return Err(Error::Config("enter the IMAP server, e.g. imap.example.com".into()));
    }
    Ok(())
}

/// Havre's address as the browser sees it: `HAVRE_PUBLIC_URL`, or the
/// origin the web UI sent (`http://localhost:9988`).
fn public_url(origin: &str) -> Result<String> {
    let configured = std::env::var("HAVRE_PUBLIC_URL").ok().filter(|v| !v.trim().is_empty());
    let url = configured.as_deref().unwrap_or(origin).trim().trim_end_matches('/');
    let valid = (url.starts_with("http://") || url.starts_with("https://"))
        && !url.contains(['?', '#', ' '])
        && url.len() > "https://".len();
    if valid {
        Ok(url.to_owned())
    } else {
        Err(Error::Config("can't tell Havre's address: set HAVRE_PUBLIC_URL in .env".into()))
    }
}

/// 128 random bits, as hex: unguessable ids for sign-ins in progress.
fn random_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Connects and logs in, then logs out. Errors are rephrased for humans.
pub async fn test_connection(config: &ImapConfig) -> Result<()> {
    let attempt = async {
        let mut session = imap::connect(config).await?;
        session.logout().await?;
        Ok::<_, Error>(())
    };
    match tokio::time::timeout(Duration::from_secs(20), attempt).await {
        Err(_) => {
            Err(Error::Config(format!("{}:{} did not answer within 20 seconds", config.host, config.port)))
        }
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(Error::Config(explain(config, &err))),
    }
}

fn explain(config: &ImapConfig, err: &Error) -> String {
    let text = err.to_string();
    let lower = text.to_lowercase();
    if let Error::SignIn(message) = err {
        message.clone()
    } else if matches!(err, Error::Imap(async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_))) {
        if config.host.ends_with("office365.com") || config.host.ends_with("outlook.com") {
            "Microsoft refused the password: Microsoft 365 and Outlook.com mailboxes no longer accept \
             passwords over IMAP. Choose Authentication: OAuth2 instead."
                .into()
        } else if config.host.ends_with("gmail.com") {
            "Google refused the password. Choose Authentication: OAuth2, or use an app password \
             (Google Account › Security › App passwords)."
                .into()
        } else {
            "The server refused the user name or password. Gmail, iCloud and Yahoo need an app \
             password instead of your usual password."
                .into()
        }
    } else if lower.contains("lookup") || lower.contains("not known") || lower.contains("no such host") {
        format!("Server “{}” not found. Check the spelling.", config.host)
    } else if lower.contains("refused") {
        format!(
            "{} refused the connection on port {}. IMAP over TLS normally uses 993.",
            config.host, config.port
        )
    } else if lower.contains("certificate")
        || lower.contains("invalid peer")
        || lower.contains("unknown issuer")
    {
        format!("The certificate of {} is not trusted: {text}", config.host)
    } else {
        text
    }
}

/// Every 30 seconds: snoozed messages whose time has come are marked unread
/// and shown again (on top of All inboxes).
async fn snooze_loop(manager: std::sync::Weak<AccountManager>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let Some(manager) = manager.upgrade() else { return };
        let now = chrono::Utc::now().timestamp();
        let due = match manager.db.due_snoozes(now).await {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "reading snoozed messages");
                continue;
            }
        };
        for (account, folder, uid) in due {
            let result = match manager.engine(account).await {
                Ok(engine) => engine.wake(&folder, uid).await,
                Err(err) => Err(err),
            };
            if let Err(err) = result {
                tracing::warn!(%err, account, folder, uid, "waking a snoozed message");
            }
        }
    }
}
