# Test specification map

This suite does **not** treat the current implementation as the specification. Each behavior is classified by its source of truth:

- **External** — required by an RFC, POSIX, Rust's documented OS-resource contract, or an operating-system ABI/kernel implementation.
- **Crate contract** — an intentional `tun-rs` API policy documented by this crate. These rules may be stricter than the OS but must not contradict it.
- **Heuristic** — a performance or sizing choice, not a correctness rule. Tests may protect its mechanics, but it must not be described as an external requirement.

When an external source and the implementation disagree, the implementation is the bug unless the source is inapplicable to the target/version. Tests should encode the externally required behavior rather than preserve the bug.

## Authoritative behavior map

| Behavior | Authority | What the suite must establish |
| --- | --- | --- |
| IPv4 prefix lengths are 0–32 and masks are left-contiguous | **External:** RFC 4632 §§3.1, 5.1 | All valid prefixes round-trip; non-contiguous masks and >32 are rejected; `/0` is accepted. |
| IPv6 prefixes are a count of contiguous high-order bits | **External:** RFC 4291 §2.3 | Prefixes 0–128 round-trip; non-contiguous masks and >128 are rejected. |
| POSIX `readv`/`writev` scatter/gather ordering and descriptor ownership boundaries | **External:** POSIX + Rust `AsFd`/`BorrowedFd`/`FromRawFd` contracts | Vectored data order is preserved; owned raw-fd conversion transfers ownership; borrowed wrappers never close the fd. |
| `poll()` timeout finer than its resolution is rounded **up** | **External:** POSIX `poll()` | Positive sub-millisecond durations become at least 1 ms; large Rust durations are not silently shortened to `c_int::MAX` ms overall. |
| Interruptible read cancellation remains observable across readiness races | **Crate contract**, implemented on POSIX readiness primitives | A signalled interrupt wins if device and interrupt fds are simultaneously ready; `WouldBlock` retries do not reset the caller's total timeout. |
| IPv6 socket conversion preserves flowinfo and scope ID | **External:** Rust `SocketAddrV6`, POSIX/RFC `sockaddr_in6`; Rust std's platform conversion is the reference mapping | `sin6_flowinfo` and `sin6_scope_id` survive Rust↔C round-trip, including nonzero values. |
| BSD/macOS TUN family header is four-byte network-order address family | **External:** Apple XNU `if_utun.c`, FreeBSD `if_tuntap.c`, OpenBSD/NetBSD `if_tun.c` | Target-gated `packet_information_tests` assert exact big/network-order `AF_INET`/`AF_INET6` bytes. |
| Linux legacy TUN vnet header represented here is 10 bytes | **External:** Linux `include/uapi/linux/virtio_net.h`; TUN defaults `vnet_hdr_sz` to `sizeof(struct virtio_net_hdr)` | ABI-size assertion is 10 bytes and short buffers fail. Larger/newer vnet-header layouts are not represented by this type. |
| Linux TUN legacy vnet-header byte order | **External:** Linux `drivers/net/tun_vnet.h` | Native/legacy endianness is correct for descriptors created by this crate, because it does not opt into `TUNSETVNETLE`/`TUNSETVNETBE`. Helpers must not claim universal Virtio endianness. |
| `VIRTIO_NET_HDR_GSO_ECN` is a flag on TCP GSO types | **External:** Linux UAPI + `include/linux/virtio_net.h` | TCPv4/v6 with ECN is accepted; base GSO type is classified after masking ECN; ECN on UDP and unsupported GSO types are rejected. |
| TCP GSO segmentation sequence/length/checksum/flag semantics | **External:** Linux `net/ipv4/tcp_offload.c`, IP/TCP checksum rules | Payload reconstructs exactly; sequence advances by MSS; FIN/PSH occur only on the final segment; legacy CWR remains only on the first segment; IPv4/IPv6 lengths and checksums are correct. |
| UDP GSO (`UDP_L4`) segmentation | **External:** Linux virtio-net/TUN support | Per-segment UDP/IP lengths, payload and checksums are correct; legacy UFO (`GSO_UDP = 3`) is not silently treated as UDP_L4. |
| Linux TUN/TAP mode and multi-queue semantics | **External:** Linux TUN/TAP documentation/UAPI | Real privileged tests create TUN/TAP; a multi-queue clone refers to the same interface/index. |
| Linux persistent-device offload state is correctly reset on reattach | **External:** Linux TUN offload ioctls + crate lifecycle contract | Dedicated privileged regression explicitly runs under CI rather than hiding behind `#[ignore]`. |
| NetBSD TUN MTU is 576–1500 inclusive | **External:** NetBSD `sys/net/if_tun.c` (`SIOCSIFMTU`) and `if_tun.h` (`TUNMTU=1500`) | Native NetBSD privileged test accepts both boundaries and rejects 575/1501. |
| Windows interface LUID/MTU/address configuration and error reporting use each API family's documented status model | **External:** Microsoft NetIO/IP Helper, Registry, Configuration Manager, and Win32 synchronization docs | Native Windows tests exercise live adapter identity/configuration; `netio_status_conversion_uses_the_returned_error_code` locks direct status-code translation rather than stale `GetLastError()` state. |
| Windows finite waits preserve finite-timeout semantics | **External:** Win32 wait APIs reserve `INFINITE` as a sentinel and accept millisecond `DWORD` timeouts | `finite_wait_timeout_rounds_up_without_aliasing_infinite` checks zero/sub-ms rounding and the maximum finite chunk; wait loops continue across chunks for longer Rust `Duration`s. |
| Windows TAP creation requires the external TAP-Windows driver | **External prerequisite:** project-supported TAP backend uses hardware ID `tap0901`; README documents TAP-Windows installation | Native `create_tap` validates a real TAP when the driver exists; when absent, only the backend's explicit `NotFound: No driver found` capability error is accepted. |

## Intentional tun-rs contracts

These are not claims about an RFC or kernel ABI. They are project API decisions and should be documented/tested as such:

| Contract | Evidence |
| --- | --- |
| `DeviceBuilder::new()` and `DeviceBuilder::default()` both use the documented enabled-by-default policy; `.inherit_enable_state()` explicitly opts out | `builder::tests::new_and_default_enable_device_and_inherit_clears_override` |
| Repeated IPv4 builder configuration is last-write-wins; IPv6 entries accumulate in call order | `builder::tests` |
| Tokio and async-io features are mutually exclusive | CI's `Async runtime exclusivity contract` compile-fail step |
| `BytesCodec` consumes a complete packet without copying its allocation | `async_device::async_framed::tests` |
| `Decoder::decode_eof` rejects leftover undecoded bytes | `async_device::async_framed::tests` |
| The framed read buffer cannot be configured below its packet-safe initial minimum | `read_buffer_size_setter_respects_packet_minimum_and_allows_safe_resize` |
| The initial framed buffer reserves Ethernet + two VLAN tags above MTU | `framed_buffer_reserves_ethernet_and_two_vlan_tags` — **project safety/headroom policy**, not an MTU/RFC requirement |
| Non-GSO framed write-buffer sizing only grows through its public setter | `write_buffer_size_setter_only_grows_without_gso` |
| Interrupt event trigger is level-like/idempotent until reset and preserves the first trigger value | `platform::unix::interrupt::tests` |

## Coverage tiers

| Area | Executable evidence | CI tier |
| --- | --- | --- |
| Address/netmask + builder policy | `builder::tests` | Every Linux PR; native release platforms |
| Public examples/signatures | doctests, default/Tokio/async-io | Every Linux PR |
| Framing/codec/buffer contracts | `async_device::async_framed::tests` | Tokio + async-io matrices |
| POSIX fd ownership, vectored I/O, deterministic interrupt/timeout semantics | `platform::unix::{fd,interrupt}::tests` using pipes/event fds rather than assuming a live TUN stays idle | Unix feature matrices |
| Sockaddr conversion/provenance | unit tests + targeted Miri | Every Linux PR |
| Linux virtio/GRO/GSO algorithms | `platform::linux::offload::tests` | Every Linux PR |
| Sync/Tokio/async-io IPv4+IPv6 packet reception | privileged `tests/test_dev.rs` | Every Linux PR |
| Raw-fd ownership, nonblocking, multiqueue, interrupt wrapper forwarding | `tests/test_dev.rs` | Every Linux PR / applicable native release targets |
| Persistent offload reset | dedicated privileged ignored-test invocation | Every Linux PR |
| Windows/macOS native behavior | `build_n_test` matrix | Release tags |
| FreeBSD pure library behavior | native FreeBSD VM unit matrices | Release tags |
| NetBSD library + privileged TUN behavior | native NetBSD 10.1 VM, all three feature matrices | Release tags |
| OpenBSD/Android/iOS/tvOS/OpenHarmony compatibility | source audit and/or cross-target compilation where Rust artifacts exist | Release tags |

## Primary sources

- RFC 4632 — CIDR / IPv4 prefix semantics: https://www.rfc-editor.org/rfc/rfc4632.html
- RFC 4291 — IPv6 addressing/prefix semantics: https://www.rfc-editor.org/rfc/rfc4291.html
- POSIX `poll()`: https://pubs.opengroup.org/onlinepubs/9799919799/functions/poll.html
- POSIX `readv()`/`writev()`: https://pubs.opengroup.org/onlinepubs/9799919799/functions/readv.html and https://pubs.opengroup.org/onlinepubs/9799919799/functions/writev.html
- Rust raw/borrowed fd contracts: https://doc.rust-lang.org/std/os/fd/ and `FromRawFd`
- Rust `SocketAddrV6`: https://doc.rust-lang.org/std/net/struct.SocketAddrV6.html
- Linux virtio-net UAPI: https://github.com/torvalds/linux/blob/master/include/uapi/linux/virtio_net.h
- Linux virtio-net conversion rules: https://github.com/torvalds/linux/blob/master/include/linux/virtio_net.h
- Linux TCP GSO/GRO implementation: https://github.com/torvalds/linux/blob/master/net/ipv4/tcp_offload.c
- Linux TUN/TAP implementation/endian controls: https://github.com/torvalds/linux/blob/master/drivers/net/tun.c and https://github.com/torvalds/linux/blob/master/drivers/net/tun_vnet.h
- Linux TUN/TAP documentation: https://docs.kernel.org/networking/tuntap.html
- Apple UTUN ABI: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/net/if_utun.c
- FreeBSD TUN ABI: https://github.com/freebsd/freebsd-src/blob/main/sys/net/if_tuntap.c
- NetBSD TUN ABI/MTU: https://github.com/NetBSD/src/blob/trunk/sys/net/if_tun.c and https://github.com/NetBSD/src/blob/trunk/sys/net/if_tun.h
- OpenBSD TUN ABI: https://github.com/openbsd/src/blob/master/sys/net/if_tun.c
- Microsoft IP Helper APIs: https://learn.microsoft.com/windows/win32/api/netioapi/
- Microsoft `ConvertInterface*` status contracts: https://learn.microsoft.com/windows/win32/api/netioapi/nf-netioapi-convertinterfaceluidtoalias
- Microsoft Configuration Manager status mapping: https://learn.microsoft.com/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_mapcrtowin32err
- Microsoft `RegNotifyChangeKeyValue`: https://learn.microsoft.com/windows/win32/api/winreg/nf-winreg-regnotifychangekeyvalue
- Microsoft `CreateEventW` / wait semantics: https://learn.microsoft.com/windows/win32/api/synchapi/nf-synchapi-createeventw

Performance claims are **not** correctness assertions. Batch size 128 and extra framing headroom are heuristics/project policy. Throughput ratios, CPU savings, and coalescing ratios require representative benchmarks and must not be presented as protocol guarantees.
