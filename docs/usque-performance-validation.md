# Downstream performance validation with usque

`tun-rs-fast` has two different kinds of performance evidence, and they answer different questions.

1. The repository's existing `tun-benchmark2` benchmark measures the TUN library more directly.
2. The `usque-rs-fast -> quiche-fast -> tun-rs-fast` campaign measures whole-tunnel cost, including the kernel and the rest of the QUIC/MASQUE stack.

Do not compare the absolute numbers from those two benchmark families. Use the microbenchmark to understand tun-rs itself; use the downstream harness to prove that a tun-rs change actually makes the deployed tunnel cheaper.

The canonical whole-stack methodology and closed-candidate ledger are documented at:

- https://github.com/codeandsolder/usque-rs-fast/blob/main/docs/BENCHMARKING.md
- https://github.com/codeandsolder/usque-rs-fast/blob/main/docs/REJECTED_OPTIMIZATIONS.md

Before proposing an obvious copy/checksum/batching cleanup, search the rejected-optimization ledger. Several changes that were very convincing in tun-rs microbenchmarks disappeared or reversed in the current whole-tunnel stack.

## Downstream headline metric

The downstream score is:

**raw host-wide busy CPU seconds / steady inner-L3 Gbit**

The host boundary is intentional. TUN changes commonly alter syscall count, batching, copying, checksumming, wakeups, softirq, and kernel networking work. Looking only at the `usque-rs` process can therefore give the wrong answer.

Useful traffic is counted at the TUN inner-L3 boundary: RX uses TUN RX bytes; TX and UTX use TUN TX bytes. Application goodput is a quality/saturation signal, not the efficiency denominator.

## Test tiers

The campaign used two main environments:

- **low tier:** MT7621/OpenWrt router, excellent for exposing fixed syscall/per-packet cost;
- **high tier:** EPYC host, useful for higher rates and separating process/kernel behavior.

A general tun-rs optimization should ideally move the right way on both. If it only helps one architecture or operating range, document that scope.

## TUN-write batching work

The RX path showed that writing decoded packets to TUN is a meaningful whole-host cost. We explored fixed packets-per-write settings, delay sweeps, adaptive batching, multiple adaptive-policy revisions, low-tier control runs, high-tier focused runs, and direct/native write variants.

A retained adaptive candidate (`adaptive4` in the campaign artifacts) produced representative matched RX results:

| Tier / rate | Baseline host s/Gbit | adaptive4 host s/Gbit | Approx. change |
|---|---:|---:|---:|
| low RX 10 Mbit/s | 81.843 | 80.440 | -1.7% |
| high RX 50 Mbit/s | 3.571 | 3.516 | -1.5% |
| high RX 100 Mbit/s | 3.087 | 2.947 | -4.5% |

The durable conclusion is not that four packets is universally optimal. It is that bounded aggregation can reduce real host cost, and the optimal policy depends on traffic/rate/architecture enough that it should be adaptive and latency-bounded rather than an unbounded “batch more” rule.

## Checksum-path result — early positive screen, final rejection

An early high-tier RX 100 Mbit/s comparison made the checksum candidate look useful:

- baseline: 3.319 host s/Gbit,
- checksum candidate: 3.228 host s/Gbit,
- approximately **-2.73%** raw host CPU/Gbit.

An early adaptive-TUN-write + checksum combination also measured 3.100 host s/Gbit against the 3.319 baseline (about **-6.59%**) with essentially unchanged inner throughput.

Those are historical screening results, **not the current disposition**. The later current-stack confirmation ran two independent five-pair RX100 campaigns. Across all ten accepted pairs, checksum SAD had:

- **+2.17% median idle-adjusted host CPU/Gbit**,
- +2.14% raw-host median,
- only **2/10** host wins,
- +0.84% median candidate-process CPU.

The final Rust 1.99 native profiles also put the checksum function at only about 0.7–1.2% self cycles in the relevant modes.

**Current conclusion: reject the hand-written checksum/SAD path as an end-to-end tunnel optimization.** The local checksum microbenchmark improvement is real, but it does not survive the full-stack boundary.

## Direct/native write experiment

Profiler work suggested a more direct write path. The promotion evidence came from a native high-tier RX 200 Mbit/s A/B, not from Cachegrind:

- direct candidate: 2.568 host s/Gbit,
- matched comparison candidate: 2.670 host s/Gbit,
- both passed quality 3/3,
- inner throughput was virtually identical (~212.2 Mbit/s).

This is a useful template for future low-level tun-rs work: let profiling point at the code, but confirm the value with a native whole-stack run.

## Closed / superseded tun-rs candidates

### Removing the GRO packet `.to_vec()` — rejected

The obvious allocation/copy removal was functionally correct but **slower in corrected standalone CPU probes**. Do not assume that deleting the visible allocation improves this path; buffer shape, aliasing, alignment, and generated copy code matter.

### Checksum SAD / AVX2 — rejected end-to-end

The local checksum microbenchmark win was large and real (roughly 24% at 1500 bytes and around 39-41% for larger buffers), but the current whole-tunnel ten-pair result above rejected it. This is now a textbook example of why tun-rs microbenchmarks are screening evidence rather than the promotion gate.

### Fixed pseudo-header checksum specialization — deferred

This looked promising in local work but whole-GRO attribution was unstable. It never reached a convincing current whole-tunnel result. Do not make it a first-pass candidate unless a fresh profile shows checksum construction has become materially hotter.

### GRO flow-map lookup/hash variants — do not throw out the successful descendants

The rejected copy-removal and checksum candidates should not be generalized into "GRO work never helps". Replacing `contains_key` + `get_mut` with one entry lookup and switching the flow table to AHash produced real surgical wins and were retained. Start from the maintained implementation rather than replaying the old intermediate map/hash probes.

### Ready-drain policy tuning belongs above tun-rs

The nonblocking `try_recv_multiple()` primitive was useful and became part of the accepted batching mechanism. The many rejected cap/threshold/deadline/hysteresis variants were **usque policy experiments**, not evidence that the tun-rs API was a mistake. The cross-repository rejected-optimization ledger records those policy variants in detail.

## Why the existing microbenchmark still matters

The whole-tunnel harness includes quiche, usque policy, kernel networking, the generator, and real tunnel traffic. That makes it realistic but less surgical.

`tun-benchmark2` is better for isolating a raw TUN implementation change, finding per-operation overhead, comparing APIs/backends without QUIC noise, and quickly screening a candidate before a remote end-to-end campaign.

The downstream harness is better for deciding whether to promote the change into the real stack, detecting kernel cost moved by batching/syscall behavior, proving equivalent useful traffic, and spotting interactions with quiche/usque scheduling.

A good performance change should normally make sense in the microbenchmark and survive the downstream test. If the two disagree, investigate the boundary rather than averaging them together.

## Rules for downstream A/B tests

When the change under test is in tun-rs:

1. pin the usque commit;
2. pin the quiche commit;
3. pin the compiler and lockfile;
4. keep runtime batching/flush knobs fixed unless that knob is the experiment;
5. record exact candidate binary hashes;
6. use 8-second steady-state native samples for promotion-grade comparisons;
7. alternate candidate order and repeat;
8. reject saturated points from efficiency averages.

The 8-second window is calibrated. During the campaign, a 6-second prefix could still be off by as much as 8.35% relative to the full 8-second result.

## Watch process-vs-kernel tradeoffs

Do not use candidate-process CPU as the headline.

One unrelated receive-path experiment in the same stack reduced process CPU/Gbit by about 1.96% while increasing raw host CPU/Gbit by about 5.21%. TUN changes are particularly capable of producing this kind of tradeoff because syscall and kernel work sit directly across the library boundary.

Always inspect raw host CPU/Gbit first, then inner throughput/quality, kernel CPU/Gbit, candidate CPU/Gbit, softirq/softnet counters, and drops/retransmits.

## Saturation is not efficiency

The router and high-tier host reach their knees at very different rates. If a candidate cannot sustain the requested traffic, classify the point as saturation/capacity evidence and compare achieved throughput/quality. Do not call a lower CPU/Gbit number an efficiency win when the candidate moved less useful traffic.

Matched-rate efficiency and maximum throughput are separate claims.

## Profilers

Cachegrind, perf, hardware counters, and cache/L1-focused passes were used during this campaign. They are hypothesis generators. Instrumentation changes the timing enough that absolute Cachegrind CPU/Gbit is not comparable to native results.

Recommended loop:

1. microbenchmark or native whole-stack result identifies a cost;
2. profiler identifies likely code/instruction/syscall source;
3. make one change;
4. microbenchmark again if relevant;
5. native pinned-stack A/B;
6. expand across tiers/rates before making a broad claim.

## Correctness remains part of the gate

A faster raw-fd/framing/batching path is useless if it changes packet semantics.

Performance work in this repository still needs the normal platform/backend correctness tests. The downstream harness then adds the “same useful traffic at lower host cost” check.

Do not work around a correctness regression to preserve a benchmark number; fix the contract first, then remeasure.
