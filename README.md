# Rectangle

Rectangle is a Windows desktop client for Browser and Agent sessions on a Linux Docker host you control. Browser tabs run Firefox with uBlock Origin. Agent tabs run Freebuff in an Alpine terminal. Each tab has its own container. The client sends input and receives the remote display over SSH.

## Install

### 1. Prepare the server

Use a dedicated Linux machine or VM reachable over SSH. A Linux x86-64 host with Docker Engine, Python 3, OpenSSH Server, Bash, GNU coreutils, and util-linux (`flock`) is the reference setup. Start with at least 4 GiB of RAM for one Browser tab. Install [Docker Engine](https://docs.docker.com/engine/install/), then verify that your chosen Linux account can run `docker info` without sudo. On a rootful Docker host, this access is [root-equivalent](https://docs.docker.com/engine/install/linux-postinstall/#manage-docker-as-a-non-root-user).

Clone or extract this repository on the server. In its root, run one of the following as your chosen Linux account:

```sh
# Browser traffic goes out through the server's HTTPS proxy, without a VPN.
sh server/install.sh --direct-https

# Or: Browser tabs have no internet until you configure an optional VPN.
sh server/install.sh --vpn-only
```

The installer builds the images and sets up the launcher in `~/.local/bin/`. Use the same account for installation and Rectangle connections. The HTTPS proxy blocks private and special-use destinations and permits public HTTPS on port 443. No Browser display ports need to be opened. If using a VPS, restrict inbound SSH to your client network where practical.

### 2. Authorize the Windows client

In PowerShell, make a dedicated client key (choose a unique filename if it already exists):

```powershell
New-Item -ItemType Directory -Force "$HOME/.ssh" | Out-Null
ssh-keygen -t ed25519 -f "$HOME/.ssh/rectangle_ed25519"
Get-Content "$HOME/.ssh/rectangle_ed25519.pub"
```

On the Linux server, create `~/.ssh` with mode 700, then add this line to the same account's `~/.ssh/authorized_keys` (mode 600). Replace the placeholder with the **entire public-key line** from PowerShell:

```text
restrict,command="exec ~/.local/bin/rectangle-session" PASTE_PUBLIC_KEY_LINE_HERE
```

Optional: install `btop` on the server and add `pty` after `restrict` on that line to enable the interactive host monitor. Keep the private key on Windows. If it has a passphrase, load it into the Windows OpenSSH agent with `ssh-add`. Verify the server's SSH host-key fingerprint against the server console before accepting it; Rectangle uses strict host-key checking.

Replace `user@server.example` below with your SSH login to confirm the launcher is reachable:

```powershell
ssh -T -i "$HOME/.ssh/rectangle_ed25519" user@server.example 'exec ~/.local/bin/rectangle-session host-memory'
```

A successful setup returns a line starting with `RECTHOSTMEM1`.

### 3. Build and open the Windows client

Install [Rust](https://rustup.rs/) and [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) with **Desktop development with C++** and a Windows SDK. Use the stable MSVC Rust toolchain. Windows OpenSSH Client must be installed. From the repository root in PowerShell:

```powershell
cargo +stable build --release --locked
.\target\release\rectangle.exe
```

In **Settings → Connections**, enter your own `user@host` and the path to your private key. Click **Save settings**, then **+ New Tab → Browser**. If you have another Linux server, configure it under **Secondary**. The Home library lists sessions by server. A native Mac build is not included.

## Use Rectangle

Right-click an Agent tab and choose **Open Browser** to open Firefox in a linked tab inside that same container. Its browser profile stays in the Agent’s persistent home directory. Open this Browser tab before clicking **Continue to sign in** in Codex. The app’s web links are then directed to that Browser tab, and localhost login callbacks reach Codex inside the same container. Return to the Codex tab when sign-in finishes.

Open Browser requires the 0.1.21 launcher and updated Freebuff or Codex image. Save and reopen existing Agents after **Sync servers** to start them with the updated image. Custom Agent images need Firefox and the browser helper from the current Agent Dockerfile.

Codex tabs default to **QHD (2560 × 1440)**. Right-click the tab and open **Resolution** to select QHD, Full HD, 4K, Match Tile, or Match Host.

To authorize the container’s Codex engine from another computer, open **SH Terminal** in that container and run `codex login --device-auth`. Open the displayed link on the other computer, enter the code, and approve sign-in. Keep the command running until it confirms success. Enable device-code login in your ChatGPT security settings or workspace permissions first. Afterward, return to the Codex app and check the account. If needed, save and reopen the tab to reload the cached credentials. The app’s browser sign-in flow is also available inside its container.

When creating an **Agent** or **File Manager** tab, choose **Freebuff** or **ChatGPT Codex** under **Agent app**. Codex uses the official Linux desktop app on Debian 13, with its main windows maximized and its dialogs visible. Sign in inside the container and select Codex in the app. Its workspace, app settings, and credentials stay in that Agent’s persistent home directory.

Codex containers include Python, Node.js/npm, Git, and an OpenSSH client. Their selected TCP port follows the existing Agent publishing settings. Services must listen on `0.0.0.0` and use `PORT`. Outbound SSH uses the container’s own `~/.ssh` keys and host verification. Install your own keys through File Manager or SH Terminal. The server and Windows login keys are separate.

The Codex image uses the official x64 Linux package version 26.1117.41037 with a pinned SHA-256. A changed upstream package fails verification until the recipe’s URL and checksum are updated. ARM64 Codex containers require a corresponding verified ARM64 package recipe. Update the server to 0.1.19 before creating a Codex container. Existing Freebuff containers keep their own image and workspace.

In **Settings → Startup → Website theme**, choose **Light**, **Dark**, or **Firefox default** and save. Websites that use `prefers-color-scheme` can follow the selected theme. The setting applies to Browser containers when they start, including reopened saved profiles. Save and reopen an existing Browser tab to apply the change. Parked tabs keep their current theme until you save and reopen them.

Light and Dark require the 0.1.4 or newer server launcher and Browser image. To upgrade, run the server installer with your existing install mode before selecting either theme.

- **Browser:** The Firefox toolbar is hidden. Use Back, Forward, and Refresh on each tab. Click in the page or on its tab to link the address bar to that browser. Click away to return to new-tab mode. Use **Settings → Startup** for new Browser tabs, or edit a saved Browser's startup page from Home. **Smooth video** limits the remote display to 1080p and uses compression.
- **Agent:** Pick a port when opening the tab. Bind your in-container service to `0.0.0.0` and use the provided `PORT` environment variable. The host port binds to loopback by default. Right-click **Open preview in Brave** to access it over SSH. To publish it on a particular LAN interface, write that server's IPv4 address to `~/.config/rectangle/agent-bind-ip` and start a new container. Agent uses regular Docker networking, outside the Browser VPN.
- **Custom Agent images:** The server owner can enable builds with `touch ~/.config/rectangle/allow-agent-builds`. Only enable this for trusted clients. Edit the built-in recipe to create a named copy; running a custom image does not alter the built-in recipe.
- **Agent resolution:** Right-click an Agent tab and open **Resolution**. Select **4K UHD (3840 × 2160)**, **2K (2048 × 1080)**, **1K (1024 × 576)**, **Match Tile**, or **Match Host**. Match Tile follows the space available to the remote display. Match Host follows the monitor showing Rectangle, within the supported display limits. New Freebuff Agents default to Match Tile and Codex Agents to QHD; a saved resolution preference takes precedence.
- **Agent views:** Right-click an Agent tab to open **File Manager** or **SH Terminal** in the same container. From File Manager you can open **Freebuff Terminal**. Close a linked view to keep the Agent running. Parking or stopping the Agent applies to the whole container. Choose **File Manager** under **+ New Tab** to start a new Agent container in the file explorer.
- **Temporary storage:** Agents have a separate 512 MiB temporary filesystem. Keep project files in `/agent/home/workspace` and remove temporary build or screenshot profiles when finished. If temporary storage fills, linked views report the problem instead of an X display lock error.
- **Clone a tab:** Right-click a Browser or Agent tab and choose **Clone tab**. Give it a new name. Rectangle pauses the source during the copy, saves the clone in Home, and opens it in a separate container. The clone gets the Browser profile or Agent home files, including files stored in a dedicated SMB folder. Agent clones get a separate port and no SMB shares. Running processes and temporary files are not copied.
- **Open a shell from your desktop:** Focus a Freebuff, SH Terminal, or File Manager tab to see the SSH command for opening a shell at its current folder. Copy the command into your local terminal. It uses your normal SSH login with Docker access on the host. In File Manager, replace the command with an absolute folder path and press Enter to navigate there.
- **File Manager:** Select its tab to show the host, container, and folder in the Rectangle address bar. Enter an absolute folder path or a copied folder address for that Agent. Use Back and Forward for folder history, the arrow to toggle name sort, and the view menu for icon, list, or detail layouts. Dates are in UTC; a dash means the filesystem did not provide a creation date.
- **Agent files:** Drop local files in File Manager to upload with progress and cancellation. In icon view, drag an item onto a folder in the same Agent to move it. Right-click to copy its full Rectangle address, rename, delete with confirmation, download with Windows Save As, or open the file in Vim. Changes and transfers are restricted to `/agent/home`. These controls require the 0.1.9 server launcher; Vim requires the updated Agent image.
- **Creating Agent work:** New Agent and File Manager tabs require a name. The chosen server suggests its first available TCP port starting at 4000; you can enter a different port or refresh the suggestion. Port availability is checked again at launch. Linked views show their parent Agent's name. In File Manager, **New Folder** and **New File** accept names compatible with Linux and Windows. **New File** opens the newly created file in Vim. These options require the 0.1.11 client and server.
- **Host usage:** The RAM graph opens `btop`. The Disk graph shows usage and capacity for the primary server’s root filesystem and opens `ncdu /` in read-only mode. Install `ncdu` on the host and enable PTY allocation for your Rectangle SSH key as in step 2. Agent views and disk monitoring require the 0.1.8 server launcher. Starting directly in File Manager also requires the updated Agent image.
- **Agent memory:** Right-click a running Agent tab and choose **Memory limit…** to see usage and apply a new RAM limit without restarting the container. A decrease must stay above current usage. Parked Agents retain the live limit, and saved Agents remember it when reopened. New Agents and Browser tabs still start with 3 GiB. Requires the 0.1.7 server launcher or newer.
- **Always on:** Right-click a running Agent tab and check **Always on**. It skips the idle timeout and stays running on the server after Rectangle exits or disconnects. Reconnect from Home, or reopen Rectangle to restore tabs that were visible when it closed. Uncheck it to restore the normal timeouts, or use **Save and close**, **Delete**, or **Stop** to end the running session. Requires the 0.1.6 server launcher or newer. A server or Docker restart can still stop the container.
- **Sessions:** Save and close to keep cookies or an Agent workspace on the server. Park to leave the container running, then resume from Home. Drag tabs to tile them; empty tiles show Home. Hover over a tab for RAM use. Per-tab volume and window placement are stored on the Windows desktop.

Starting with **0.1.27**, Rectangle remembers this computer's tab order, selected tab (or Home), linked Agent views, and tile arrangement in `%LOCALAPPDATA%\Rectangle\workspace.ron`. Closing the window captures that workspace before parking its tabs. Reopening restores available parked tabs or reopens their saved profiles, using the local tile arrangement. Tabs on an unavailable server remain remembered for a later launch. An explicit tab selection takes precedence over late startup replies.

The **−** button beside **×** minimizes and parks a tab. Resume it from **Home → Parked sessions**; if a saved session's parking period has expired, open its saved entry instead. Minimizing an Agent parks its linked views with it, and restores those views when resumed. Minimized tabs stay parked when Rectangle reopens. Existing parking lifetimes still apply: normal parking lasts 30 minutes, while **Always on** Agents stay running. The existing **×** save-and-close behavior is unchanged.

Starting with **0.1.26**, Agent resolution, Browser **Smooth video**, and File Manager's last successfully opened folder are saved automatically with the server profile. Saved and parked tabs restore these preferences when reopened, including from another updated computer using the same server account. Linked File Manager views share the Agent's folder preference. Match Tile and Match Host keep following the current computer's layout and monitor. If the previous folder was removed or is inaccessible, File Manager returns to `/agent/home/workspace`. Preferences live in `~/.config/rectangle/tab-preferences/`; cloning copies them and deleting a saved profile removes them. Both client and server must be on 0.1.26 or newer. Settings → Updates installs the matching server support.

## Optional VPN setup

In **Settings → Proton VPN**, use **Restart VPN** to recreate the shared gateway with its installed profile. Rectangle confirms success after a VPN-routed HTTPS check. This requires the 0.1.22 server release. Restart once after upgrading an older gateway to apply the new boot-safe TUN configuration. The gateway creates its TUN device inside the container, so Docker can start it after a host reboot even when the host’s `/dev/net/tun` has not been created yet. Offline mode and an absent profile are reported as errors. A restart briefly interrupts VPN browsing on that server.

### Proton VPN Free

The top-right **Proton VPN** badge checks for a VPN route and an HTTPS handshake from inside the gateway. Open **Settings → Proton VPN** to view the endpoint, tunnel address, DNS and routing without exposing private keys. To change the config, enter the path to a downloaded Proton GNU/Linux WireGuard `.conf` file and select **Apply new config**. Rectangle sends it over SSH, validates it before restarting the shared gateway, and restores the previous config if the replacement does not connect. To allow changes from the client, the server owner must create `~/.config/rectangle/allow-vpn-changes`.

Download a fresh GNU/Linux [WireGuard profile from Proton](https://protonvpn.com/support/wireguard-configurations) for a free server. Transfer it securely to the Docker host. Install it under `$HOME/.config/rectangle/proton/client.conf` with mode 600 in a directory with mode 700. Create a second file in that directory named `resolv.conf` (mode 644 so the proxy can read it) containing one line, `nameserver <the IPv4 DNS address from the Proton profile>`. Run `$HOME/.local/bin/rectangle-proton start`.

All new tabs share the one Proton tunnel while retaining separate browser containers. Only HTTPS and DNS travel through the gateway; IPv6 is blocked. The gateway publishes no ports and does not change the host's routes. If it is stopped or loses the tunnel, new internet traffic fails closed. To switch to OpenVPN, stop the gateway with `rectangle-proton stop`, move the Proton profile to private storage outside its active path, then open new tabs after configuring OpenVPN.

### Private OpenVPN server

To use OpenVPN, you need a private OpenVPN server on a VPS with IPv4 forwarding and internet egress (usually NAT), and a compatible client profile. Copy the profile directly to the Docker host and install it with private permissions:

```sh
install -d -m 700 "$HOME/.config/rectangle/vpn"
install -m 600 /path/to/client.ovpn "$HOME/.config/rectangle/vpn/client.ovpn"
```

Rectangle accepts a certificate-based TUN profile with one numeric public IPv4 UDP endpoint and inline `<ca>`, `<cert>`, and `<key>` blocks. Inline TLS authentication or TLS crypt keys are supported. Use a private CA for the VPS, and optionally pin its certificate name with `verify-x509-name`. Do not put the profile or its private key in the project, client, or source archive. The VPS must accept simultaneous connections with the configured client identity for multiple tabs. The VPN uses `9.9.9.9` for DNS by default; to use your VPN's own resolver, write its IPv4 address alone in `$HOME/.config/rectangle/vpn/dns`. The resolver must be reachable through the tunnel.

Each online tab starts its own Alpine OpenVPN container. The browser still has Docker networking disabled and reaches only its tab's proxy over a Unix socket. The proxy accepts public HTTPS on port 443, resolves and checks destinations, and connects to the checked address. It blocks private, loopback, link-local, and other special-use addresses, including mixed DNS answers. Firefox verifies site certificates. Plain HTTP upgrades to HTTPS; HTTP-only sites and nonstandard HTTPS ports are unsupported. The VPN container routes IPv4 through OpenVPN, drops IPv6, and permits only the OpenVPN client process to send UDP to the configured endpoint outside the tunnel. If the tunnel drops, proxy and DNS traffic cannot fall back to the ordinary network. Windows and unrelated apps are outside this tunnel.

To explicitly disable the tunnel and start offline tabs:

```sh
touch "$HOME/.config/rectangle/offline"
```

Remove the flag to allow new tabs to start a tunnel if a profile is installed. Close older tabs when enabling the new deployment; running containers retain their original networking. After setup, open a fresh tab, check its public IP against the VPS, and verify that public browsing stops if the tunnel is interrupted.

## Development

### Update the Windows client

**Settings → Updates** shows the installed version. Choose **Check for updates**, then **Update** if the official repository has a higher version. Rectangle checks out the exact revision advertised during the check and builds it with Cargo. When the build succeeds, it parks your tabs, replaces the client executable, and restarts. Updates require Git, Rust/Cargo, the same Windows build tools as the initial build, and a writable client executable directory. A failed build keeps the installed version and shows the path to the build log.

Starting with 0.1.10, **Settings → Updates** builds the desktop client first and updates configured servers that are behind. Servers fetch the matching revision through the official GitHub repository, serialize concurrent requests, and return immediately when their installed version is already at or above the requested one. Changed images build before the host activates the new files. Running containers and VPN settings stay in place; new containers use the new images. If a server fails, Rectangle keeps the current desktop version and can retry without rebuilding the prepared desktop binary.

Enable server updates once, as the server's Rectangle SSH account, from the 0.1.10 source checkout: `sh server/install.sh --keep-egress --enable-server-updates`. This preserves the existing egress mode. Install Git on the host first. On a fresh server, use `--direct-https` or `--vpn-only` instead of `--keep-egress`. To opt out later, remove `~/.config/rectangle/allow-server-updates`. A desktop on 0.1.9 or earlier can install 0.1.10 using its old desktop-only updater. After that, **Check for updates → Sync servers** can bring hosts up to the desktop version even when there is no newer client. Program Files installations may prompt for Windows administrator approval to replace the executable.

Maintainers must bump the three-part version in `Cargo.toml` and update `Cargo.lock` for each update. When adding a local Docker build input, also add it to the corresponding image in `server/release.py`.

```powershell
cargo fmt --check
cargo test --locked
python -m unittest discover -s server
```

The default tests do not connect to a server. Ignored integration tests require your own test host and key; see the `RECTANGLE_SMOKE_*` variables in the test source. Existing running containers retain their old image. The server owner can still run `sh server/install.sh --keep-egress` for manual upgrades.

The client icon and marble pixels are generated by `build.rs` and embedded in the executable. The marble is compressed to keep the embedded asset small. SSH management requests create their processes on worker threads, including the startup session lookup and host meters. To measure artwork loading independently of graphics-driver initialization and server reconnection, run `cargo test --release --locked startup_artwork_timing -- --ignored --nocapture`. These timings measure artwork preparation, not total application startup.

Startup reads saved and parked sessions in parallel for each configured host, reusing those results for Home. Each host restores independently, so an unavailable secondary host does not block the ready host. Sessions arriving later open without changing the selected tab. To compare the former sequence of session-list requests with the parallel reads, set `RECTANGLE_SMOKE_HOST` and `RECTANGLE_SMOKE_IDENTITY`, then run `cargo test --release --locked startup_network_timing -- --ignored --nocapture`. This reads server metadata without opening or changing containers and measures session lookup, not first-window or desktop-stream rendering time.

On Windows, Rectangle first initializes DirectX 12 to avoid loading and enumerating other graphics backends before showing its window. If DirectX adapter or device creation fails, it uses the original automatic graphics setup. An explicit `WGPU_BACKEND` setting bypasses this preference. Other platforms keep their existing graphics setup. To diagnose launch time, set `RECTANGLE_STARTUP_TRACE` to a writable log-file path before launching. The optional log records settings, graphics, artwork and UI milestones in milliseconds from entry into `main`; `first-frame-complete` is recorded at the next native frame after the first paint. Process creation and executable loading precede these timestamps. No timing log or extra diagnostic repaint is produced unless tracing is enabled.

## Read-only Windows shares

Rectangle can share a folder from a saved Agent through one Samba service per host. It uses whole Docker volumes and works on Docker 25. The host needs an unused TCP port 445 on its LAN address. Enable the feature once from a regular SSH terminal as the Rectangle server owner:

```sh
python3 -I ~/.config/rectangle/current-release/smb.py setup <LAN-IP> <LAN-CIDR>
```

Enter a separate SMB password at the hidden prompts. The command needs the private IPv4 address and local subnet, for example `192.168.1.20 192.168.1.0/24`. Do not forward TCP 445 from the internet. SMB uses the username `rectangle`.

1. Save and close the Agent once, then reopen it from Home.
2. In File Manager, right-click a folder such as Music and check **Share over SMB**. Pick a share name that's unique on this host.
3. The first time you share a folder, save and close the Agent, then reopen it from Home. Parking won't start the copy. Rectangle needs free space for a second copy and keeps the original files for recovery. Afterward, the Agent uses the new volume at the same folder path and SMB stays available when the Agent is closed.
4. When the share is live, right-click the folder and choose **Copy Windows path**. Map it to a drive in Windows using the SMB username and password, then add that drive in Winamp.

Samba mounts only the dedicated folder volume, read-only, and requires authentication. Agent edits inside the folder appear immediately over SMB. Turning off sharing leaves files available at the same path, and sharing that folder again does not require another restart. You can edit the files inside the folder, but Rectangle protects the folder itself and its parent paths from moves and deletion in File Manager. Turn off sharing before deleting the saved Agent. Changing shares can briefly disconnect other SMB clients. The server owner can turn off a share without reopening the Agent: `python3 -I ~/.config/rectangle/current-release/smb.py remove <share-name>`.

If you enabled shares under 0.1.13, turn them all off before updating the server, then share those folders again after updating.

The Windows icon and loading animation are generated from the square spiral in `src/mark.rs`. Firefox and the other software used in containers are downloaded during image builds.

See [SECURITY.md](SECURITY.md) for the trust model and [NOTICE](NOTICE) for third-party attribution. Rectangle source is available under [Apache License 2.0](LICENSE).
