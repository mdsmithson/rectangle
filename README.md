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

In **Settings → Startup → Website theme**, choose **Light**, **Dark**, or **Firefox default** and save. Websites that use `prefers-color-scheme` can follow the selected theme. The setting applies to Browser containers when they start, including reopened saved profiles. Save and reopen an existing Browser tab to apply the change. Parked tabs keep their current theme until you save and reopen them.

Light and Dark require the 0.1.4 or newer server launcher and Browser image. To upgrade, run the server installer with your existing install mode before selecting either theme.

- **Browser:** The Firefox toolbar is hidden. Use Back, Forward, and Refresh on each tab. Click in the page or on its tab to link the address bar to that browser. Click away to return to new-tab mode. Use **Settings → Startup** for new Browser tabs, or edit a saved Browser's startup page from Home. **Smooth video** limits the remote display to 1080p and uses compression.
- **Agent:** Pick a port when opening the tab. Bind your in-container service to `0.0.0.0` and use the provided `PORT` environment variable. The host port binds to loopback by default. Right-click **Open preview in Brave** to access it over SSH. To publish it on a particular LAN interface, write that server's IPv4 address to `~/.config/rectangle/agent-bind-ip` and start a new container. Agent uses regular Docker networking, outside the Browser VPN.
- **Custom Agent images:** The server owner can enable builds with `touch ~/.config/rectangle/allow-agent-builds`. Only enable this for trusted clients. Edit the built-in recipe to create a named copy; running a custom image does not alter the built-in recipe.
- **Agent resolution:** Right-click an Agent tab and open **Resolution**. Select **4K UHD (3840 × 2160)**, **2K (2048 × 1080)**, **1K (1024 × 576)**, **Match Tile**, or **Match Host**. Match Tile follows the space available to the remote display. Match Host follows the monitor showing Rectangle, within the supported display limits. Each Agent tab starts in Match Tile.
- **Agent views:** Right-click an Agent tab to open **File Manager** or **SH Terminal** in the same container. From File Manager you can open **Freebuff Terminal**. Close a linked view to keep the Agent running. Parking or stopping the Agent applies to the whole container. Choose **File Manager** under **+ New Tab** to start a new Agent container in the file explorer.
- **File Manager:** Select its tab to show the host, container, and folder in the Rectangle address bar. Enter an absolute folder path or a copied folder address for that Agent. Use Back and Forward for folder history, the arrow to toggle name sort, and the view menu for icon, list, or detail layouts. Dates are in UTC; a dash means the filesystem did not provide a creation date.
- **Agent files:** Drop local files in File Manager to upload with progress and cancellation. In icon view, drag an item onto a folder in the same Agent to move it. Right-click to copy its full Rectangle address, rename, delete with confirmation, download with Windows Save As, or open the file in Vim. Changes and transfers are restricted to `/agent/home`. These controls require the 0.1.9 server launcher; Vim requires the updated Agent image.
- **Creating Agent work:** New Agent and File Manager tabs require a name. The chosen server suggests its first available TCP port starting at 4000; you can enter a different port or refresh the suggestion. Port availability is checked again at launch. Linked views show their parent Agent's name. In File Manager, **New Folder** and **New File** accept names compatible with Linux and Windows. **New File** opens the newly created file in Vim. These options require the 0.1.11 client and server.
- **Host usage:** The RAM graph opens `btop`. The Disk graph shows usage and capacity for the primary server’s root filesystem and opens `ncdu /` in read-only mode. Install `ncdu` on the host and enable PTY allocation for your Rectangle SSH key as in step 2. Agent views and disk monitoring require the 0.1.8 server launcher. Starting directly in File Manager also requires the updated Agent image.
- **Agent memory:** Right-click a running Agent tab and choose **Memory limit…** to see usage and apply a new RAM limit without restarting the container. A decrease must stay above current usage. Parked Agents retain the live limit, and saved Agents remember it when reopened. New Agents and Browser tabs still start with 3 GiB. Requires the 0.1.7 server launcher or newer.
- **Always on:** Right-click a running Agent tab and check **Always on**. It skips the idle timeout and stays running on the server after Rectangle exits or disconnects. Reconnect from Home or restart Rectangle to resume it. Uncheck it to restore the normal timeouts, or use **Save and close**, **Delete**, or **Stop** to end the running session. Requires the 0.1.6 server launcher or newer. A server or Docker restart can still stop the container.
- **Sessions:** Save and close to keep cookies or an Agent workspace on the server. Park to leave the container running, then resume from Home or restart Rectangle. Drag tabs to tile them; empty tiles show Home. Hover over a tab for RAM use. Per-tab volume and window placement are stored on the Windows desktop.

## Optional VPN setup

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

The Windows icon and loading animation are generated from the square spiral in `src/mark.rs`. Firefox and the other software used in containers are downloaded during image builds.

See [SECURITY.md](SECURITY.md) for the trust model and [NOTICE](NOTICE) for third-party attribution. Rectangle source is available under [Apache License 2.0](LICENSE).
