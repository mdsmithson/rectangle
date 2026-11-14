import unittest

from vpn import profile


PROFILE = """client
dev tun
proto udp
remote 93.184.216.34 1194
<ca>
CA
</ca>
<cert>
CERT
</cert>
<key>
KEY
</key>
"""


class VPNProfileTest(unittest.TestCase):
    def test_enforces_server_verification_and_ipv4_default_route(self):
        endpoint, config = profile(PROFILE + 'verify-x509-name "Rectangle VPN" name\n')
        self.assertEqual(endpoint, ("93.184.216.34", 1194))
        for directive in ("dev tun0", "remote-cert-tls server", "redirect-gateway def1",
                          "allow-compression no", 'verify-x509-name "Rectangle VPN" "name"'):
            self.assertIn(directive, config.splitlines())

    def test_rejects_hostnames_and_fallback_endpoints(self):
        for remote in ("remote vpn.example.net 1194", "remote 192.168.0.1 1194",
                       "remote ::1 1194", "remote 93.184.216.34 1194 tcp",
                       "remote 93.184.216.34 1194\nremote 93.184.216.35 1194"):
            with self.subTest(remote=remote), self.assertRaises(ValueError):
                profile(PROFILE.replace("remote 93.184.216.34 1194", remote))

    def test_rejects_scripts_includes_and_external_credentials(self):
        for option in ("up /tmp/a.sh", "script-security 3", "config /etc/openvpn/client.conf",
                       "auth-user-pass /tmp/pass", "key /tmp/key", "route 0.0.0.0 0.0.0.0"):
            with self.subTest(option=option), self.assertRaises(ValueError):
                profile(PROFILE + option + "\n")
