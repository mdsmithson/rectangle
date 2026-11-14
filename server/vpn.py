"""Start a tab's HTTPS proxy behind a locked-down OpenVPN tunnel."""
import ipaddress
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time

INLINE = {"ca", "cert", "key", "tls-auth", "tls-crypt", "tls-crypt-v2"}
PASS = {"auth", "cipher", "data-ciphers", "data-ciphers-fallback",
        "key-direction", "verify-x509-name", "tun-mtu", "mssfix", "reneg-sec"}
IGNORED = {
    ("client",), ("dev", "tun"), ("dev", "tun0"), ("proto", "udp"),
    ("proto", "udp4"), ("nobind",), ("resolv-retry", "infinite"),
    ("persist-key",), ("persist-tun",), ("remote-cert-tls", "server"),
    ("auth-nocache",), ("redirect-gateway", "def1"), ("verb", "3"),
}
LOCAL_PORT = 11940


def profile(source):
    if len(source) > 256 * 1024:
        raise ValueError("VPN profile is too large")
    output, found = [], set()
    section, remote = None, None
    for number, line in enumerate(source.splitlines(), 1):
        line = line.strip()
        if section is not None:
            if line == f"</{section}>":
                section = None
            elif "<" in line or ">" in line or "\0" in line:
                raise ValueError(f"Invalid inline data on line {number}")
            output.append(line)
            continue
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("<") and line.endswith(">"):
            section = line[1:-1]
            if section not in INLINE or section in found:
                raise ValueError(f"Unsupported inline block on line {number}")
            found.add(section)
            output.append(line)
            continue
        try:
            args = tuple(shlex.split(line, comments=True))
            if args in IGNORED:
                continue
            if len(args) in (3, 4) and args[0] == "remote" and remote is None:
                addr = ipaddress.IPv4Address(args[1])
                port = int(args[2])
                if (addr.is_global and not addr.is_multicast and 1 <= port <= 65535
                        and (len(args) == 3 or args[3] in {"udp", "udp4"})):
                    remote = str(addr), port
                    continue
            if len(args) >= 2 and args[0] in PASS and all(
                value.isascii() and value.isprintable() for value in args[1:]
            ):
                # Quote arguments ourselves; never copy an unparsed option line.
                escaped = [arg.replace("\\", "\\\\").replace('"', '\\"') for arg in args[1:]]
                output.append(args[0] + " " + " ".join(f'"{arg}"' for arg in escaped))
                continue
        except (ValueError, IndexError):
            pass
        raise ValueError(f"Unsupported or unsafe VPN option on line {number}")
    if section or remote is None or not {"ca", "cert", "key"} <= found:
        raise ValueError("VPN profile needs one public numeric UDP remote and inline CA, cert, and key")
    ip, port = remote
    header = [
        "client", "dev tun0", "proto udp4", f"remote {ip} {port}", f"lport {LOCAL_PORT}",
        "remote-cert-tls server", "tls-version-min 1.2", "allow-compression no",
        "disable-dco", "script-security 1", "auth-nocache",
        "redirect-gateway def1", "ping 10", "ping-restart 30",
        'pull-filter ignore "redirect-"', 'pull-filter ignore "route "',
        'pull-filter ignore "route-ipv6"', 'pull-filter ignore "dhcp-option"',
        'pull-filter ignore "dns"', 'pull-filter ignore "ifconfig-ipv6"',
        'pull-filter ignore "block-ipv6"', "verb 3",
    ]
    return (ip, port), "\n".join(header + output) + "\n"


def firewall(ip, port):
    return f"""table inet rectangle {{
        chain output {{
            type filter hook output priority 0; policy drop;
            oifname "tun0" meta nfproto ipv4 accept
            oifname "eth0" ip daddr {ip} udp sport {LOCAL_PORT} udp dport {port} meta skuid 0 accept
        }}
        chain input {{
            type filter hook input priority 0; policy drop;
            iifname "tun0" meta nfproto ipv4 accept
            iifname "eth0" ip saddr {ip} udp sport {port} udp dport {LOCAL_PORT} accept
        }}
        chain forward {{
            type filter hook forward priority 0; policy drop;
        }}
    }}
"""


def main():
    source = Path("/run/vpn/client.ovpn")
    if source.stat().st_size > 256 * 1024:
        raise ValueError("VPN profile is too large")
    (ip, port), config = profile(source.read_text())
    dns = ipaddress.IPv4Address(os.environ["RECTANGLE_VPN_DNS"])
    resolvers = [line.split()[1:] for line in Path("/etc/resolv.conf").read_text().splitlines()
                 if line.split()[:1] == ["nameserver"]]
    if (dns.is_loopback or dns.is_link_local or dns.is_multicast or dns.is_unspecified
            or resolvers != [[str(dns)]]):
        raise ValueError("VPN requires one explicit IPv4 DNS resolver")
    os.umask(0o077)
    path = Path("/tmp/client.ovpn")
    path.write_text(config)
    # No app process starts until the atomic default-deny rules are installed.
    subprocess.run(["nft", "-f", "-"], input=firewall(ip, port), text=True, check=True)
    vpn = subprocess.Popen(["openvpn", "--config", str(path)])
    proxy = None
    try:
        proxy = subprocess.Popen(["su-exec", "10001:10001", "/usr/bin/python3", "-I",
                                  "/opt/rectangle/egress.py", "/run/rectangle/egress.sock"])
        while vpn.poll() is None and proxy.poll() is None:
            time.sleep(0.25)
        raise RuntimeError("The tab VPN or proxy stopped")
    finally:
        for child in (vpn, proxy):
            if child is not None and child.poll() is None:
                child.terminate()


if __name__ == "__main__":
    try:
        main()
    except (OSError, UnicodeError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as exc:
        # Do not print profile contents or private keys.
        print(f"Rectangle VPN: {exc}", file=sys.stderr)
        sys.exit(1)
