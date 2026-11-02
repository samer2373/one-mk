# Wi-Fi and network profile observations

## Native follow-up (Linux implementation host; native runtime NOT RUN)

The native watcher publishes a canonical snapshot of SSIDs, local non-loopback
IPv4/IPv6 addresses and default-router MAC addresses. Address/gateway-only
changes publish even if the SSID stays the same, so DHCP after association and
route/VPN changes re-evaluate rules. Sorted/deduplicated equal snapshots do not
repeatedly select a profile or undo a manual layout change.

- macOS: CoreWLAN remains the SSID listener. A SCDynamicStore run-loop source
  watches Global IPv4/IPv6 and service/interface IP state. The Router value
  selects ARP/NDP neighbors. IPv4 cache misses trigger a bounded one-shot ping
  (at most one second) and a second ARP read on the Wi-Fi worker, never an input
  thread. IPv6 gateway identity currently requires an existing NDP neighbor;
  unresolved/non-Ethernet neighbors do not match a MAC rule. Subnet rules use
  both IPv4 and IPv6 addresses regardless of neighbor availability.
- Windows: WlanAPI remains the SSID listener. NotifyIpInterfaceChange,
  NotifyRouteChange2 and NotifyUnicastIpAddressChange wake the same worker;
  all registrations are cancelled before their callback context is freed.
  GetIpForwardTable2 selects default routes; GetIpNetEntry2/ResolveIpNetEntry2
  obtain router MACs. Neighbor resolution happens only on a refresh.
- Windows watches both wiFiControl and location capability permission changes.
  Focusing the Profiles window also requests a fresh status, so a permission
  grant can be detected even if a capability notification was unavailable.
- Linux's NetworkManager observer is unchanged. Without NetworkManager this
  automatic network-profile watcher is unavailable; manual profile use works.

There is no new periodic timer, polling loop or idle neighbor probe. Zero idle
wakeups/CPU is an owner measurement, not proven by compilation. A burst of OS
notifications may cause multiple refreshes, but only changed snapshots publish.
SSID permission denial clears SSIDs without discarding usable route/address
observations. oneMK does not subscribe to location coordinates.

The Profiles window offers SSID rules only on Linux/macOS/Windows. It does not
expose a gateway/subnet rule editor; those existing rules use `profile auto
--network` or configuration. No unsupported rule kinds were added to the window.

## Owner hardware checklist — NOT RUN

- [ ] On each native OS associate before DHCP finishes. Confirm the SSID first,
  then a subnet rule switches after the address notification, with no reassociation.
- [ ] Renew/change the DHCP address on the same SSID, add/remove an IPv4/IPv6
  address and switch a default route/VPN. Confirm exactly changed snapshots
  cause rule evaluation and a manual layout persists while the network is idle.
- [ ] Add a router-MAC rule, clear the ARP/neighbor cache in a disposable network,
  reconnect and confirm router identity appears. Test warm IPv6 NDP separately;
  a missing IPv6 neighbor is a documented non-match, not successful detection.
- [ ] Deny Location, grant it while oneMK is running, then focus Profiles.
  Confirm readable Wi-Fi names and switching return without a daemon restart,
  particularly on Windows 11 24H2. Revoke again and check the permission reason.
- [ ] Disable Wi-Fi, use wired-only networking and stop WLAN services where
  possible. IP/route changes should still be observed; manual profiles work.
- [ ] Leave the system idle and measure watcher CPU/wakeups and spawned probes;
  no periodic work should occur. Repeat reconnect bursts and shutdown to check
  callbacks never use freed context. Native builds here are compile-checked
  only; no hardware result has been recorded.
