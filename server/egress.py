"""Per-tab HTTPS tunnel. The browser has no network interface beyond loopback."""

import asyncio
import ipaddress
import os
import re
import socket
import sys
from urllib.parse import urlsplit, urlunsplit

LIMIT = 40
HEADER_LIMIT = 8192
IDLE_SECONDS = 120
LABEL = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", re.ASCII)
V6_PUBLIC = ipaddress.ip_network("2000::/3")
V6_TRANSITION = tuple(map(ipaddress.ip_network, ("2002::/16", "2001::/32")))


def public_ip(text):
    address = ipaddress.ip_address(text)
    if isinstance(address, ipaddress.IPv6Address) and address.ipv4_mapped:
        return public_ip(str(address.ipv4_mapped))
    if isinstance(address, ipaddress.IPv6Address) and (
        address.scope_id or address not in V6_PUBLIC or any(address in net for net in V6_TRANSITION)
    ):
        return False
    return (address.is_global and not address.is_multicast and not address.is_reserved
            and not address.is_loopback and not address.is_link_local)


def host_name(raw):
    if not raw or len(raw) > 254 or "%" in raw:
        raise ValueError("invalid host")
    name = raw.lower().rstrip(".")
    try:
        return str(ipaddress.ip_address(name))
    except ValueError:
        labels = name.split(".")
        if (len(labels) < 2 or labels[-1].isdigit() or
            labels[-1] in {"localhost", "local", "internal", "lan", "home", "onion"} or
            not all(LABEL.fullmatch(label) for label in labels)):
            raise ValueError("invalid public hostname") from None
        return name


def request_line(header):
    line = header.split(b"\r\n", 1)[0].decode("ascii")
    method, target, version = line.split(" ")
    if version not in {"HTTP/1.0", "HTTP/1.1"} or not target or any(not 33 <= ord(c) <= 126 for c in target):
        raise ValueError("invalid request")
    if method == "CONNECT":
        authority = re.fullmatch(r"(\[[0-9a-fA-F:.]+\]|[a-zA-Z0-9.-]+):443", target)
        if authority is None:
            raise ValueError("only HTTPS port 443 is allowed")
        return method, host_name(authority[1].strip("[]"))
    if method not in {"GET", "HEAD"}:
        raise ValueError("unsupported request")
    url = urlsplit(target)
    if url.scheme != "http" or url.username is not None or url.port not in {None, 80}:
        raise ValueError("only HTTPS is supported")
    name = host_name(url.hostname)
    if ":" in name:
        name = f"[{name}]"
    return method, urlunsplit(("https", name, url.path or "/", url.query, ""))


def checked_targets(entries):
    targets = list(dict.fromkeys((family, address) for family, _, _, _, address in entries
                                if family in (socket.AF_INET, socket.AF_INET6)))
    if not targets or any(not public_ip(address[0]) for _, address in targets):
        raise PermissionError("non-public DNS result")
    return sorted(targets, key=lambda target: target[0] != socket.AF_INET)


async def upstream(name):
    loop = asyncio.get_running_loop()
    try:
        literal = ipaddress.ip_address(name)
    except ValueError:
        literal = None
    if literal is None:
        entries = await asyncio.wait_for(loop.getaddrinfo(name + ".", 443, type=socket.SOCK_STREAM), 5)
    else:
        if not public_ip(name):
            raise PermissionError("non-public address")
        family = socket.AF_INET if literal.version == 4 else socket.AF_INET6
        address = (name, 443) if literal.version == 4 else (name, 443, 0, 0)
        entries = [(family, socket.SOCK_STREAM, socket.IPPROTO_TCP, "", address)]
    # Connect to the checked numeric address, never resolve the name a second time.
    for family, address in checked_targets(entries)[:6]:
        conn = socket.socket(family, socket.SOCK_STREAM)
        conn.setblocking(False)
        try:
            await asyncio.wait_for(loop.sock_connect(conn, address), 2)
            return await asyncio.open_connection(sock=conn)
        except (OSError, TimeoutError):
            conn.close()
        except BaseException:
            conn.close()
            raise
    raise ConnectionError("upstream unavailable")


async def respond(writer, status, extra=b""):
    writer.write(b"HTTP/1.1 " + status + b"\r\nConnection: close\r\nContent-Length: 0\r\n" + extra + b"\r\n")
    await asyncio.wait_for(writer.drain(), 5)


async def pump(reader, writer, idle):
    loop = asyncio.get_running_loop()
    while data := await reader.read(32768):
        idle.reschedule(loop.time() + IDLE_SECONDS)
        writer.write(data)
        await asyncio.wait_for(writer.drain(), IDLE_SECONDS)
    if writer.can_write_eof():
        writer.write_eof()


async def relay(client_reader, client_writer, remote_reader, remote_writer):
    # An HTTPS download can receive data for minutes without sending anything back.
    # Expire the tunnel only when neither direction moves data.
    async with asyncio.timeout(IDLE_SECONDS) as idle:
        transfers = [
            asyncio.create_task(pump(client_reader, remote_writer, idle)),
            asyncio.create_task(pump(remote_reader, client_writer, idle)),
        ]
        try:
            await asyncio.gather(*transfers)
        finally:
            for task in transfers:
                task.cancel()
            await asyncio.gather(*transfers, return_exceptions=True)


class Egress:
    def __init__(self):
        self.active = 0

    async def handle(self, reader, writer):
        if self.active >= LIMIT:
            writer.close()
            return
        self.active += 1
        remote = None
        try:
            header = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 8)
            if len(header) > HEADER_LIMIT:
                raise ValueError("header too large")
            method, target = request_line(header)
            if method != "CONNECT":
                # No cleartext upstream. Upgrade plain HTTP addresses instead.
                await respond(writer, b"308 Permanent Redirect", b"Location: " + target.encode("ascii") + b"\r\n")
                return
            try:
                remote_reader, remote = await upstream(target)
            except PermissionError:
                await respond(writer, b"403 Forbidden")
                return
            except (OSError, TimeoutError):
                await respond(writer, b"502 Bad Gateway")
                return
            writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            await asyncio.wait_for(writer.drain(), 5)
            await relay(reader, writer, remote_reader, remote)
        except (ValueError, UnicodeError, asyncio.LimitOverrunError):
            try:
                await respond(writer, b"400 Bad Request")
            except (OSError, TimeoutError):
                pass
        except (OSError, TimeoutError, asyncio.IncompleteReadError):
            pass
        finally:
            if remote is not None:
                remote.close()
            writer.close()
            self.active -= 1


async def serve(path):
    os.umask(0o177)
    server = await asyncio.start_unix_server(Egress().handle, path=path, limit=HEADER_LIMIT, backlog=LIMIT)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    asyncio.run(serve(sys.argv[1]))
