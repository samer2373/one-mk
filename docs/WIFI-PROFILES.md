# Switching layouts by network

`one-mk profile auto` picks a saved layout profile by itself when this computer joins a
Wi-Fi network, a subnet, or a router (see [ARRANGING.md](ARRANGING.md#profiles) for profiles).
This page says how each system reports network changes, what it needs, and what has and has
not been checked.

```bash
one-mk profile auto office --ssid "Office WiFi"
one-mk profile auto home --network 192.168.1.0/24          # a subnet this computer is on
one-mk profile auto home --network aa:bb:cc:dd:ee:ff       # the router's MAC address
```

The window's **Desk > Arrange** profiles card offers Wi-Fi name rules on Linux, macOS and Windows,
shows the current network, and explains and offers a settings button when permission is needed.
Subnet and router rules are command-line only.

## How each system is watched

oneMK watches for changes of the Wi-Fi name (SSID), this computer's addresses and the default
router's hardware address. Address or router changes count even when the Wi-Fi name stays the
same, so DHCP finishing after association, a renewed address, or a VPN changing the default route
all re-evaluate the rules. Identical snapshots do not re-select a profile, so a layout you changed
by hand stays until the network really changes. There is no timer and no polling: the watchers
sleep until the system reports an event, and a burst of events causes at most a few refreshes.

- **Linux:** NetworkManager reports every change. Without NetworkManager (systemd-networkd or iwd
  on their own) the automatic watcher is unavailable; `one-mk profile use NAME` still works.
- **macOS:** CoreWLAN reports the Wi-Fi name and a system-configuration listener reports IPv4/IPv6
  and service changes. The router's hardware address comes from the ARP/NDP neighbour table; a
  missing IPv4 neighbour triggers one bounded probe (at most one second) per refresh, never an
  idle poll. An IPv6 router needs an already-cached neighbour; an unresolved neighbour simply
  does not match a router rule. Subnet rules use both IPv4 and IPv6 addresses regardless.
- **Windows:** WlanAPI reports the Wi-Fi name and IP Helper notifications report interface, route
  and address changes; routes and neighbours are read from the IP Helper tables, and only when
  refreshing.

**Permission to read Wi-Fi names.** macOS 14 and later and Windows 11 24H2 require Location
Services permission before an app may read the Wi-Fi name. oneMK asks only for that: it does not
request location updates, does not read coordinates and does not track your location. Without
permission, Wi-Fi names are unavailable but subnet and router rules keep working. Grant it in the
system's privacy settings; the Profiles card notices when you return to the window. On macOS use
the installed app so the permission text and identity are available to both the window and the
background service.

## Not yet verified on real Windows or macOS

The Windows and macOS watchers compile and are unit-tested but have never run on those systems,
so network profiles there are **experimental**. Linux with NetworkManager is the only tested path.
If you have a Windows or Mac and a few minutes, these checks would tell us if it works (report
results through an issue; see [CONTRIBUTING.md](../CONTRIBUTING.md)):

1. Join a Wi-Fi network before DHCP finishes. The Wi-Fi name rule applies first, then a subnet rule
   applies once the address arrives, with no reconnect.
2. Renew or change the DHCP address on the same Wi-Fi network, add or remove an address, or switch
   a VPN on and off. Each real change re-evaluates the rules; a layout you changed by hand stays
   while the network is idle.
3. Add a router-MAC rule, clear the ARP/neighbour cache on a disposable network, reconnect, and
   confirm the router is recognised. A missing IPv6 neighbour is a documented non-match.
4. Deny Location, grant it while oneMK runs, then open the window: Wi-Fi names should return without a
   restart, and the card should explain again after you revoke it.
5. Turn Wi-Fi off and use a cable: address and route changes should still be seen.
6. Leave the computer idle and check that oneMK shows no periodic wake-ups from this feature; repeat
   a burst of reconnects and quit oneMK, to check nothing crashes.
