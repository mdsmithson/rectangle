# Security model

Rectangle is an early-stage, self-hosted project for one trusted operator or household. Run it on a dedicated Linux machine or VM. A rootful Docker account has root-equivalent access on that machine. Everybody with a Rectangle key for the same server account can manage that account's sessions and saved profiles. Do not give such keys to untrusted users.

Browser and Agent tabs use separate containers with a shared Linux kernel. The Browser uses separate egress and defaults to no network until configured. Agent uses ordinary Docker networking; its port defaults to server loopback. A configured public bind address can make the Agent port reachable from outside the server. No display port is published.

Keep Docker, the host OS, and your images updated. Keep SSH keys, VPN profiles, and saved session data private. Revoke a lost client key in the host's `authorized_keys` file. Use a dedicated key per device.

Remote custom Docker builds and VPN changes are disabled unless the server owner enables them. Enabling custom builds allows anyone with the account's Rectangle key to submit build recipes. Only enable it for keys you trust with that capability.

For a vulnerability, use GitHub's private vulnerability reporting if the repository owner has enabled it. Otherwise ask the maintainer for a private reporting channel before sharing exploit details.
