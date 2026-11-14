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

- **Browser:** The Firefox toolbar is hidden. Use Back, Forward, and Refresh on each tab. Click in the page or on its tab to link the address bar to that browser. Click away to return to new-tab mode. Use **Settings → Startup** for new Browser tabs, or edit a saved Browser's startup page from Home. **Smooth video** limits the remote display to 1080p and uses compression.
- **Agent:** Pick a port when opening the tab. Bind your in-container service to `0.0.0.0` and use the provided `PORT` environment variable. The host port binds to loopback by default. Right-click **Open preview in Brave** to access it over SSH. To publish it on a particular LAN interface, write that server's IPv4 address to `~/.config/rectangle/agent-bind-ip` and start a new container. Agent uses regular Docker networking, outside the Browser VPN.
- **Custom Agent images:** The server owner can enable builds with `touch ~/.config/rectangle/allow-agent-builds`. Only enable this for trusted clients. Edit the built-in recipe to create a named copy; running a custom image does not alter the built-in recipe.
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

The updater only changes the desktop client. Update the server separately when the release requires it. Maintainers must bump the three-part version in `Cargo.toml` and update `Cargo.lock` when publishing a client update to `main`.

```powershell
cargo fmt --check
cargo test --locked
python -m unittest discover -s server
```

The default tests do not connect to a server. Ignored integration tests require your own test host and key; see the `RECTANGLE_SMOKE_*` variables in the test source. To update the server, rerun `server/install.sh` with the same mode, then start new containers. Existing running containers retain their old image.

The Windows icon and loading animation are generated from the square spiral in `src/mark.rs`. Firefox and the other software used in containers are downloaded during image builds.

See [SECURITY.md](SECURITY.md) for the trust model and [NOTICE](NOTICE) for third-party attribution. Rectangle source is available under [Apache License 2.0](LICENSE).
