# Havre Desktop

A window onto your Havre server, with what a browser tab can't do:

- a **tray icon** (menu bar on a Mac) with the unread count, *New message*,
  *Open at login*, *Change server…*; closing the window keeps Havre there;
- **notifications** for new mail when the window isn't in front;
- **`mailto:` links** open a new message in Havre (once you choose it as
  your mail app);
- sign-ins with Google or Microsoft happen **in your browser**, where your
  passwords and passkeys are; links in emails open there too;
- attachments go to your **Downloads** folder.

Nothing is stored in the app but your server's address: mail is synced by
the server, so the app, the browser and your phone all show the same thing.

## Install

Download the installer for your system from the **Desktop** workflow's
latest run (GitHub › Actions › Desktop › Artifacts), or a release:

| System | File | First launch |
|---|---|---|
| Windows 10/11 | `Havre_…_x64-setup.exe` | Not signed yet: SmartScreen › *More info* › *Run anyway*. Installs for you only, no admin rights. |
| macOS 11+ | `Havre_…_aarch64.dmg` (Apple silicon) or `…_x64.dmg` (Intel) | Not notarized yet: right-click the app › *Open*, once. |
| Linux | `.deb` (Debian, Ubuntu), `.rpm` (Fedora), `.AppImage` (any) | — |

Then type your Havre's address — the one you open in a browser, e.g.
`http://192.168.1.20:8080`, or `http://localhost:9988` with Docker on this
computer — and sign in as usual.

### Havre as your mail app

Tray menu › **Make Havre the default mail app**:

- **Windows** opens *Settings › Default apps*: choose Havre for *MAILTO*.
- **Linux** makes Havre the handler right away (`xdg-mime`).
- **macOS**: in Mail › Settings › General, *Default email reader* › Havre.

## Build it yourself

Needs Rust, Node 22 and pnpm, plus the system's web view:

- **Windows**: WebView2 (already there on Windows 10/11) and the *Desktop
  development with C++* workload of Visual Studio Build Tools.
- **macOS**: Xcode command line tools (`xcode-select --install`).
- **Linux** (Debian/Ubuntu):
  `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev`

```sh
cd desktop
pnpm install
pnpm dev        # runs the app (debug), connects to the server you type
pnpm build      # installers in src-tauri/target/release/bundle/
pnpm icons      # after changing icon.svg: regenerates src-tauri/icons/
```

`cargo test` and `cargo clippy` work in `desktop/src-tauri/`. It is a
separate Cargo workspace on purpose: the server builds without any web view
libraries.

## How it works

```
desktop/
  ui/                 the "Connect to your Havre" page, bundled in the app
  src-tauri/
    src/lib.rs        window, tray, commands, mailto:, which sites stay in the window
    src/config.rs     the server address, saved as JSON in the config folder
    capabilities/     what the bundled page may call
    tauri.conf.json   name, icons, installers, the mailto: scheme
    linux/, windows/  .desktop file and installer hooks for mailto:
```

The window shows the server's own pages. They detect the app
(`web/app/composables/useDesktop.ts`) and `DesktopBridge.vue` sends the
unread count and new-mail notifications through four commands —
`desktop_info`, `notify`, `set_unread`, `open_external` — the only ones the
server's address is allowed to call (`server_capability` in `lib.rs`). The
bundled page alone can call `connect`, which chooses the server.

A `mailto:` link starts the app (or reaches the running one through
`tauri-plugin-single-instance`), which opens `/mail?compose=mailto:…` on the
server; `DesktopBridge.vue` turns it into a draft.
