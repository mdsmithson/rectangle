import asyncio
import socket
import unittest
from unittest.mock import patch

import egress


class EgressPolicy(unittest.TestCase):
    def test_rejects_local_and_special_use_addresses(self):
        for address in ('127.0.0.1', '10.1.2.3', '192.168.50.10', '172.17.0.1',
                        '100.64.0.1', '169.254.169.254', '224.0.0.1', '0.0.0.0',
                        '::1', '::ffff:192.168.50.10', 'fd00::1', 'fe80::1',
                        '64:ff9b::a00:1', '2002:a00:100::1', '2001:db8::1'):
            with self.subTest(address=address):
                self.assertFalse(egress.public_ip(address))
        self.assertTrue(egress.public_ip('1.1.1.1'))
        self.assertTrue(egress.public_ip('2606:4700:4700::1111'))

    def test_parses_https_and_upgrades_http(self):
        self.assertEqual(egress.request_line(b'CONNECT Example.COM:443 HTTP/1.1\r\n\r\n'),
                         ('CONNECT', 'example.com'))
        self.assertEqual(egress.request_line(b'GET http://example.com/a?q=1 HTTP/1.1\r\n\r\n'),
                         ('GET', 'https://example.com/a?q=1'))
        for target in (b'example.com:22', b'example.com:8443', b'localhost:443',
                       b'a.local:443', b'example.com@127.0.0.1:443', b'[fe80::1%eth0]:443'):
            with self.subTest(target=target), self.assertRaises(ValueError):
                egress.request_line(b'CONNECT ' + target + b' HTTP/1.1\r\n\r\n')


class ResolutionPolicy(unittest.TestCase):
    def test_rejects_mixed_public_and_private_dns_answers_before_connecting(self):
        answers = [(socket.AF_INET, socket.SOCK_STREAM, socket.IPPROTO_TCP, '', (ip, 443))
                   for ip in ('1.1.1.1', '192.168.50.10')]
        self.assertEqual(egress.checked_targets(answers[:1]), [(socket.AF_INET, ('1.1.1.1', 443))])
        with self.assertRaises(PermissionError):
            egress.checked_targets(answers)


class RelayPolicy(unittest.IsolatedAsyncioTestCase):
    async def test_one_way_traffic_and_half_close(self):
        class Sink:
            data = b""
            eof = False

            def write(self, data):
                self.data += data

            async def drain(self):
                pass

            def can_write_eof(self):
                return True

            def write_eof(self):
                self.eof = True

        client, remote = asyncio.StreamReader(), asyncio.StreamReader()
        client_sink, remote_sink = Sink(), Sink()
        async def stream():
            for _ in range(5):
                remote.feed_data(b"x")
                await asyncio.sleep(0.025)
            client.feed_eof()
            remote.feed_data(b"tail")
            remote.feed_eof()

        with patch.object(egress, "IDLE_SECONDS", 0.08):
            await asyncio.gather(egress.relay(client, client_sink, remote, remote_sink), stream())
        self.assertEqual(client_sink.data, b"xxxxxtail")
        self.assertTrue(client_sink.eof and remote_sink.eof)

    async def test_completely_idle_tunnel_times_out(self):
        with patch.object(egress, "IDLE_SECONDS", 0.01), self.assertRaises(TimeoutError):
            await egress.relay(asyncio.StreamReader(), None, asyncio.StreamReader(), None)
