# Moss desktop app

The desktop app is Moss in its own window, for a computer you sit at. It carries the Moss controller inside, starts it for
you, shows the panel in a native window and keeps a Moss icon in your menu bar or system tray. It is for Linux (AppImage
and `.deb`) and macOS (`.dmg`). There is no Windows version.

> [!NOTE]
> The desktop app is new. It is built by a manual build job and has not been through the testing the server install has
> had. If you run Moss on a machine you do not sit at, use the server install in the [README](../README.md) instead.

## Install

Open the **Moss Desktop** workflow in the project's Actions tab, run it, and download the bundle for your system from the
finished run.

- **Linux:** make the `.AppImage` executable and open it, or install the `.deb` with your package manager.
- **macOS 13 or later:** open the `.dmg` and drag Moss to Applications. The app is signed only ad hoc, so the first time you
  open it, right-click it and choose **Open**, then confirm. If macOS still refuses, run
  `xattr -d com.apple.quarantine /Applications/Moss.app` once.

## Using it

1. Open Moss. A window appears straight away and shows the panel as soon as the controller is ready (a few seconds the
   first time).
2. Sign in the same way as with the server install. On a fresh computer the first person to enter the one-time setup
   token becomes the owner. The token is the file `setup-token` in the data folder below.
3. Closing the window only hides it. Moss, and your game servers, keep running. Click the tray icon, or choose **Open
   Moss** from its menu, to bring the window back. Opening the app a second time just shows the window that is already
   there.
4. Choose **Quit** from the tray menu to leave. Moss asks whether to stop the controller too:
   - **Stop controller and quit** shuts Moss down cleanly.
   - **Quit, leave it running** closes the app and leaves the controller running in the background, so scheduled backups
     and restarts still happen. Open the app again to get back to it.
   - **Cancel** keeps everything as it is.

   Game servers keep running in both cases: servers outlive Moss by design.

Links in the panel to other websites open in your normal browser. The window itself only ever shows the Moss panel on your
own computer (`127.0.0.1`); it loads nothing from the internet.

## Where things are

Everything lives in one folder for your user account:

| System | Folder |
|---|---|
| Linux | `~/.local/share/io.github.t1nkieplayz.moss` |
| macOS | `~/Library/Application Support/io.github.t1nkieplayz.moss` |

Inside it: `data/` (servers, worlds, backups, the database and `setup-token`), `config/` (a `.env.local` file here adds your
own settings, see the [README](../README.md#configuration)), `moss.log` (what to look at if Moss does not start) and
`controller.json` (which running controller the app should reuse).

Back up the `data/` folder to back up everything Moss manages.

## How it relates to the server install

It is the same Moss: the same controller, panel, sign-in and features. The differences:

- The app picks a free port on your own computer (`127.0.0.1` only) instead of 8420, and never listens on the network.
- Real sign-in is always on. The desktop app does not have the no-login development mode.
- Data lives in your user folder, not in `/var/lib/moss`. The two installs do not share servers; to move one over, use
  **Import existing server** or a backup restore.
- You can run the desktop app and a server install on the same computer; they use different ports and folders.

To reach the panel from another device, use the server install behind a TLS reverse proxy instead.

## Building it yourself

The app lives in `desktop/`, its own Cargo workspace, so it never affects the normal `cargo build` or `cargo test`. The
**Moss Desktop** workflow (`.github/workflows/desktop.yml`) shows the exact steps: build the web UI and the controller,
copy the controller in as a sidecar, and run `cargo tauri build`. Building needs the Tauri prerequisites for your system
(on Linux the WebKitGTK 4.1 development packages). The logic that starts and stops the controller is a plain library,
`desktop/core`, with its own tests: `cd desktop && cargo test -p moss-desktop-core`.
