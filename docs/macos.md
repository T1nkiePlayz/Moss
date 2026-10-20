# Moss on macOS

Moss runs natively on macOS 13+ (Apple silicon and Intel). The same single binary and web panel; only the platform layer
(`src/platform/macos.rs`) differs.

## Install

**Script** (verifies the SHA-256 before installing one file, runs nothing else):

```sh
curl -fsSL https://github.com/T1nkiePlayz/Moss/releases/latest/download/install.sh -o install.sh
less install.sh                       # read it first
sh install.sh --prefix "$HOME/.local" # or without --prefix for /usr/local/bin (uses sudo)
```

**Homebrew**: each release ships a `moss.rb` formula with the real checksums.

```sh
brew install --formula ./moss.rb
brew services start moss        # data and logs under $(brew --prefix)/var/moss and /var/log
```

**By hand**: download `moss-<tag>-macos-aarch64.tar.gz` (or `-x86_64`), check it with `shasum -a 256 -c SHA256SUMS.txt`
and `gh attestation verify`, then copy `moss` somewhere on your `PATH`.

### Gatekeeper

Files downloaded with `curl` or the install script are not quarantined. A tarball downloaded in a browser is, and the
unsigned binary is then blocked on first launch ("cannot be opened because the developer cannot be verified"). Remove the
flag once, after you have verified the checksum:

```sh
xattr -d com.apple.quarantine /usr/local/bin/moss
```

(Release binaries are ad-hoc signed by the linker, not notarised.)

## Run

```sh
MOSS_AUTH=dev moss serve          # no sign-in, loopback only: for trying it out
moss doctor                       # checks data directory, port, Java and sign-in settings
```

For real use, sign-in works out of the box: the release includes the sign-in settings in `.env`. Open the panel, sign in, and enter the setup token Moss wrote to `<data dir>/setup-token` to become the owner. To pin the owner by e-mail instead, copy `moss.env.example` and set `MOSS_OWNER_EMAILS`.

### launchd

`io.moss.controller.plist` (in the tarball and in `packaging/macos/`) is a per-user LaunchAgent: `KeepAlive`, working
directory, logs in `~/Library/Logs/Moss`, settings sourced from `~/Library/Application Support/Moss/moss.env`.

```sh
mkdir -p ~/Library/LaunchAgents ~/Library/Logs/Moss "$HOME/Library/Application Support/Moss"
sed "s#REPLACE_ME_HOME#$HOME#g" io.moss.controller.plist > ~/Library/LaunchAgents/io.moss.controller.plist
cp moss.env.example "$HOME/Library/Application Support/Moss/moss.env"   # edit it
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/io.moss.controller.plist
launchctl kickstart -k gui/$(id -u)/io.moss.controller     # restart
launchctl bootout gui/$(id -u)/io.moss.controller           # stop
```

A LaunchAgent runs while you are logged in. For a headless Mac mini use a LaunchDaemon (see the comment in the plist)
with a dedicated unprivileged user. Game servers are detached from Moss (`AbandonProcessGroup`), so restarting or
upgrading Moss never stops them; Moss re-attaches by PID and start time.

## Java

Moss finds Java in `JAVA_HOME`, `/Library/Java/JavaVirtualMachines`, and Homebrew (`brew install openjdk@21`, both
`/opt/homebrew` and `/usr/local`). It never runs `/usr/bin/java`: on a Mac without a JDK that stub opens an "install
Java" dialog. **Settings > Java > Install** downloads a checksum-verified Temurin JRE (macOS builds nest the runtime in
`Contents/Home`) into the data directory. Removing a managed runtime is refused while a running server uses it.

## What differs from Linux

| | Linux | macOS |
|---|---|---|
| Memory, CPU and process limits | cgroup v2 (needs a delegated cgroup, see `packaging/moss.service`) | **not available**: limits are saved, shown as "not enforced" and reported as an info finding |
| Out-of-memory detection | "killed by the memory limit" crash detail | not available (the OS may kill a JVM silently) |
| Server memory metric | cgroup `memory.current` | resident set size (libproc) |
| CPU metric | `/proc` | libproc per process group |
| Host CPU / memory | `/proc/stat`, `/proc/meminfo` | `ps` and `vm_stat` (a few ms each, cached) |
| Service manager | systemd | launchd, or `brew services` |
| Prebuilt downloads | `linux-x86_64` | `macos-aarch64`, `macos-x86_64` |

A Mac that sleeps pauses its servers: keep it awake (`caffeinate -s`, or Energy settings) for a server that should stay
up. Moss does not change power settings for you.
