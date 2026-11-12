# Running your own relay

A relay carries traffic between two of your machines when they can't reach each other directly: one is behind a strict NAT, say, or a firewall drops UDP between them. XenoNet uses [iroh](https://www.iroh.computer)'s relay server, `iroh-relay`, and has no relay of its own and no default one. A network uses the relays its chain lists ([network.md](network.md)), and a machine may add relays for itself alone.

The connection between two machines is QUIC, end to end, keyed by each machine's `host.key`. A relay passes the QUIC packets on and can't open them: it sees which machines use it, when, from which addresses, and how much they send. It never sees what they send.

## What it needs

- **A machine every one of yours can reach.** A small VPS with a public IPv4 address (and IPv6, if you have it) is enough. A machine on your LAN works only for machines on that LAN.
- **A domain name** pointing at it (an `A` record, and `AAAA` for IPv6), such as `relay.example.net`. Let's Encrypt needs one. With a certificate from your own CA you can use an IP address instead.
- **These ports open** on the machine and in its provider's firewall:

  | Port | What for |
  |---|---|
  | TCP 443 | The relay itself, over HTTPS (`/relay`), and its probes (`/ping`, `/healthz`) |
  | TCP 80 | Let's Encrypt's HTTP challenge, and the captive portal probe (`/generate_204`) |
  | UDP 7842 | QUIC address discovery: tells your machines their public address, which helps them find a direct path. Optional |

  The metrics port (9090 by default) is for you alone: bind it to `127.0.0.1` or turn it off.
- **A TLS certificate**: from Let's Encrypt, which `iroh-relay` gets and renews by itself, or one you have, from a public CA or your own.
- **`iroh-relay` at the iroh version XenoNet uses**, 1.4.1:

  ```bash
  cargo install --locked --features server iroh-relay --version 1.4.1
  ```

  This needs Rust 1.91 or later. The binary lands in `~/.cargo/bin`; copy it to `/usr/local/bin` for the service below.

Don't run it with `--dev`: that serves plain HTTP on port 3340, for testing on one machine.

## The config file

`/etc/iroh-relay/relay.toml`, with a certificate from Let's Encrypt:

```toml
enable_relay = true
http_bind_addr = "[::]:80"
enable_quic_addr_discovery = true
enable_metrics = false

[tls]
cert_mode = "LetsEncrypt"
hostname = "relay.example.net"
contact = "you@example.net"
cert_dir = "/var/lib/iroh-relay"
prod_tls = true
```

With a certificate you have (from a public CA, or your own), in place of the `[tls]` above:

```toml
[tls]
https_bind_addr = "[::]:443"
cert_mode = "Manual"
manual_cert_path = "/etc/iroh-relay/relay.pem"
manual_key_path = "/etc/iroh-relay/relay.key"
```

`relay.pem` holds the relay's certificate, then any intermediates. The certificate has to name what the URL names: `relay.example.net`, or the IP address for `https://203.0.113.7`.

Leave `access` out, which lets everyone relay. XenoNet doesn't send a relay token yet, so a relay that asks for one turns your machines away. What it carries is sealed either way.

## A systemd unit

`/etc/systemd/system/iroh-relay.service`:

```ini
[Unit]
Description=iroh relay
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/iroh-relay --config-path /etc/iroh-relay/relay.toml
DynamicUser=yes
StateDirectory=iroh-relay
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`StateDirectory` gives the service `/var/lib/iroh-relay` for Let's Encrypt's certificates. With your own certificate, make its key readable by the service (`chmod 640` and a group the service runs in, or drop `DynamicUser` for a user of its own). Then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now iroh-relay
curl https://relay.example.net/healthz
```

The last line answers `{"status":"ok","version":"1.4.1",…}` once the relay has its certificate.

## Adding it to a network

A network's relays are in its chain, so every machine in it uses the same ones. Adding a relay widens the network: it takes the founder, or two window hosts, each of whom XenoNet asks.

- **When you start the network**: name it in **Start a network** in the window, or

  ```bash
  xenonet net create --name home --host studio --relay https://relay.example.net
  ```

  When your terminal can't ask you, the network starts without the relay and says so; add it from the window then.
- **Later**: in **Network settings**, under **Relays**, type the URL and choose **Add to** the network. Adding asks you, so it's the window's: `xenonet net relay add https://relay.example.net` in a terminal says so. On a window host other than the founder, it waits for the founder or another window host to approve it in their window.
- **Taking one out** is done at once, from either: `xenonet net relay remove https://relay.example.net`.

`xenonet net relay` lists the relays as they stand, and `n0` in place of a URL names n0's public relays, run by n0 and rate-limited.

## A relay for one machine

A relay can also be one machine's own, in no network's chain:

```bash
xenonet net relay add https://relay.example.net --mine
```

or **Add as this machine's own** under **Relays**. That machine uses it for its own dials, and may make it its home relay, the one others reach it through. Its address records carry that home relay only into networks that list it, so another network's machines never route through a relay they didn't choose, and a machine never dials a relay its network doesn't list, whatever an address names. A machine in several networks makes its home a relay every one of them lists; with none such, one of its own.

## A relay with a certificate from your own CA

Each machine has to trust the CA. Put the CA's certificate, as PEM, in `relay-ca.pem` in XenoNet's state folder, under `network/` (on a Mac, `~/Library/Application Support/XenoNet/network/relay-ca.pem`; on Linux, `~/.local/state/xenonet/network/relay-ca.pem`). The file may hold several CAs, one after another. Then start the service again, since the endpoint reads the file when it starts:

```bash
xenonet service stop && xenonet service start
```

The file is a setting of that machine alone and never goes into a chain. Machines without it can't reach the relay, and their doctor says why. The public CAs stay trusted beside yours.

## Checking it

`xenonet doctor` has a row for each relay the machine has, the networks' and its own, under **Network**: `The relay https://relay.example.net answers`. It connects as the endpoint would and says how far away the relay is, whether its certificate checks and whose CA vouched for it (a public CA, or one in `relay-ca.pem`), and the relay's version. A relay that doesn't answer is a warning that names it; one whose certificate doesn't check (expired, for another name, from a CA this machine doesn't trust) is a failure, with what to do. A `relay-ca.pem` that doesn't parse is a failure of its own.

When a relay goes down, connections that have a direct path keep it. A machine whose home relay is gone moves to another relay its networks list within seconds, and the address record it sends from then on names the new one. With no other relay, machines behind a NAT that blocks direct paths can't reach each other until it's back; the LAN and direct paths still work.

The VM lab runs a relay like this one, with a lab CA, and checks each of these ([scripts/lab/README.md](../../scripts/lab/README.md)).
