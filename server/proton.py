"""Restricted Proton WireGuard gateway. Never execute imported config directives."""
import base64
import configparser
import ipaddress
import os
from pathlib import Path
import subprocess
import stat
import sys
import time

MARK = 0x5243
PORT = 51821
SOCKET = Path("/run/rectangle/egress.sock")


def prepare_tun():
    # /dev is container-local. Opening this device can autoload the host TUN
    # driver without requiring a host /dev/net/tun mount at Docker startup.
    device = Path("/dev/net/tun")
    device.parent.mkdir(exist_ok=True)
    try:
        os.mknod(device, stat.S_IFCHR | 0o600, os.makedev(10, 200))
    except FileExistsError:
        info = device.lstat()
        if not stat.S_ISCHR(info.st_mode) or info.st_rdev != os.makedev(10, 200):
            sys.exit("Rectangle Proton: unexpected TUN device")
    try:
        descriptor = os.open(device, os.O_RDWR | os.O_NOFOLLOW)
        os.close(descriptor)
    except OSError:
        sys.exit("Rectangle Proton: cannot open TUN; verify the gateway device permissions and host TUN driver")


def profile(source):
    parser = configparser.ConfigParser(interpolation=None, inline_comment_prefixes=("#",))
    parser.read_string(source)
    if set(parser.sections()) != {"Interface", "Peer"} or parser.defaults():
        raise ValueError
    interface, peer = parser["Interface"], parser["Peer"]
    if set(interface) != {"privatekey", "address", "dns"} or not (
        {"publickey", "allowedips", "endpoint"} <= set(peer)
        <= {"publickey", "allowedips", "endpoint", "persistentkeepalive", "presharedkey"}
    ):
        raise ValueError
    address, = [addr for addr in map(ipaddress.ip_interface, interface["address"].split(","))
                if addr.version == 4]
    if address.network.prefixlen != 32 or not address.ip.is_private or address.ip.is_loopback:
        raise ValueError
    dns, = [addr for addr in map(ipaddress.ip_address, interface["dns"].split(",")) if addr.version == 4]
    if dns.is_loopback or dns.is_link_local or dns.is_unspecified or dns.is_multicast:
        raise ValueError
    endpoint, _, port = peer["endpoint"].rpartition(":")
    endpoint = ipaddress.IPv4Address(endpoint)
    port = int(port)
    if not endpoint.is_global or endpoint.is_multicast or not 1 <= port <= 65535 or endpoint == dns:
        raise ValueError
    if "0.0.0.0/0" not in [item.strip() for item in peer["allowedips"].split(",")]:
        raise ValueError

    def key(value):
        raw = base64.b64decode(value, validate=True)
        if len(raw) != 32 or not any(raw):
            raise ValueError
        return base64.b64encode(raw).decode("ascii")

    rendered = (
        f"[Interface]\nPrivateKey = {key(interface['privatekey'])}\nListenPort = {PORT}\nFwMark = {MARK}\n"
        f"[Peer]\nPublicKey = {key(peer['publickey'])}\nEndpoint = {endpoint}:{port}\n"
        "AllowedIPs = 0.0.0.0/0\nPersistentKeepalive = 25\n"
    )
    if "presharedkey" in peer:
        rendered += f"PresharedKey = {key(peer['presharedkey'])}\n"
    return address, dns, endpoint, port, rendered


def firewall(endpoint, port):
    return f"""table inet rectangle {{
        chain output {{
            type filter hook output priority 0; policy drop;
            oifname "wg0" meta nfproto ipv4 accept
            oifname "eth0" ip daddr {endpoint} udp sport {PORT} udp dport {port} meta mark {MARK} accept
        }}
        chain input {{
            type filter hook input priority 0; policy drop;
            iifname "wg0" meta nfproto ipv4 accept
            iifname "eth0" ip saddr {endpoint} udp sport {port} udp dport {PORT} accept
        }}
        chain forward {{
            type filter hook forward priority 0; policy drop;
        }}
    }}"""


def main():
    source = Path("/run/proton/client.conf")
    try:
        with source.open(encoding="utf-8-sig") as handle:
            data = handle.read(16_385)
        if len(data) > 16_384:
            raise ValueError
        address, dns, endpoint, port, rendered = profile(data)
    except (OSError, ValueError, KeyError, configparser.Error):
        sys.exit("Rectangle Proton: invalid WireGuard configuration")
    expected = [["nameserver", str(dns)]]
    actual = [line.split() for line in Path("/etc/resolv.conf").read_text().splitlines()
              if line.split()[:1] == ["nameserver"]]
    if actual != expected:
        sys.exit("Rectangle Proton: unexpected DNS resolver configuration")
    os.umask(0o077)
    prepare_tun()
    SOCKET.unlink(missing_ok=True)
    run = lambda *argv, **kw: subprocess.run(argv, check=True, **kw)
    run("nft", "-f", "-", input=firewall(endpoint, port), text=True)
    routes = subprocess.check_output(["ip", "-4", "route", "show", "default", "dev", "eth0"], text=True).split()
    gateway = str(ipaddress.IPv4Address(routes[routes.index("via") + 1]))
    run("ip", "-4", "route", "replace", f"{endpoint}/32", "via", gateway, "dev", "eth0")
    wireguard = None
    if subprocess.run(["ip", "link", "add", "wg0", "type", "wireguard"], stderr=subprocess.DEVNULL).returncode:
        wireguard = subprocess.Popen(["wireguard-go", "-f", "wg0"])
        for _ in range(40):
            if Path("/run/wireguard/wg0.sock").exists():
                break
            if wireguard.poll() is not None:
                sys.exit("Rectangle Proton: WireGuard could not start")
            time.sleep(0.1)
    # Pass the private key through an anonymous pipe. Do not print wg diagnostics that may contain it.
    run("wg", "setconf", "wg0", "/dev/stdin", input=rendered, text=True, stderr=subprocess.DEVNULL)
    run("ip", "-4", "address", "add", str(address), "dev", "wg0")
    run("ip", "link", "set", "wg0", "mtu", "1420", "up")
    run("ip", "-4", "route", "replace", "default", "dev", "wg0")
    proxy = subprocess.Popen(["su-exec", "10001:10001", "/usr/bin/python3", "-I",
                              "/opt/rectangle/egress.py", str(SOCKET)])
    try:
        while proxy.poll() is None and (wireguard is None or wireguard.poll() is None):
            time.sleep(0.5)
    finally:
        proxy.terminate() if proxy.poll() is None else None
        if wireguard is not None and wireguard.poll() is None:
            wireguard.terminate()
        SOCKET.unlink(missing_ok=True)
    sys.exit("Rectangle Proton: gateway process stopped")


if __name__ == "__main__":
    if sys.argv[1:] == ["describe"]:
        try:
            data = sys.stdin.buffer.read(16_385)
            if len(data) > 16_384:
                raise ValueError
            address, dns, endpoint, port, _ = profile(data.decode("utf-8-sig"))
        except (ValueError, KeyError, configparser.Error):
            sys.exit("Select a valid Proton GNU/Linux WireGuard .conf file.")
        print(f"{endpoint}:{port}\n{dns}\n{address}")
    elif len(sys.argv) == 1:
        main()
    else:
        sys.exit(2)
