# Zig Soroban SDK Experiments

Compares Soroban contracts written in Rust with [soroban-sdk] against the same
contracts written in Zig with [zig-soroban-sdk], on contract size and on the
cost of every call. Inspired by [antlion], which does the same for Solang.

The latest results are in [results/REPORT.md](results/REPORT.md).

## What is compared

| Contract | Rust side | Zig side |
|---|---|---|
| hello | [soroban-examples/hello_world] | [contracts/hello/zig](contracts/hello/zig/contract.zig) |
| increment | [soroban-examples/increment] | [contracts/increment/zig](contracts/increment/zig/contract.zig) |
| token | [soroban-examples/token] | [contracts/token/zig](contracts/token/zig/contract.zig) |

The Rust sources are copied unchanged from stellar/soroban-examples at
`01a9a33` (Apache-2.0). Each Zig contract was written to behave exactly like
its Rust counterpart: same storage keys and value layouts, TTL extensions,
events and checks. The harness verifies this on every run: it compares return
values, events and final storage, and the report flags any difference.

Each Zig contract is built twice:

- **Zig small**: `ReleaseSmall`, no runtime safety checks. Integer overflow wraps silently.
- **Zig safe**: `ReleaseSafe`, traps on overflow. This matches the Rust
  examples, which build with `overflow-checks = true`. It is the fair comparison.

## How it is measured

The harness ([harness/](harness/src/exec.rs)) runs every transaction the way
the network does, using soroban-env-host 29 (mainnet's protocol):

1. **Simulate** in recording mode, as `stellar contract invoke --sim` does, to
   get the footprint, auth and IO sizes.
2. **Apply** in enforcing mode on a fresh host with the module cache
   (CAP-0065), as a validator does. CPU instructions and memory bytes come from
   this run's budget, using mainnet's cost models.
3. **Price** it with mainnet's fee settings: resource fee plus rent, excluding
   the inclusion-fee bid.

State carries over from one transaction to the next. Each contract is
uploaded, deployed (running its constructor), then called through a fixed list
of steps ([harness/src/scenarios.rs](harness/src/scenarios.rs)).

Mainnet's settings are saved in `harness/config/mainnet-settings.xdr`, so runs
are reproducible offline. To refresh them:

```
stellar network settings --internal --output xdr \
  --rpc-url https://mainnet.sorobanrpc.com \
  --network-passphrase "Public Global Stellar Network ; September 2015" \
  > harness/config/mainnet-settings.xdr
```

## Running

Requirements: [Zig] 0.15.x, [stellar-cli] 28.x, and Rust with the
`wasm32v1-none` target (`rustup target add wasm32v1-none`).

```
./scripts/bench.sh
```

This builds the Rust contracts into `out/rust/`, builds the Zig contracts into
`zig-out/zig-small/` and `zig-out/zig-safe/`, then runs the harness. The
harness prints the report and writes it to `results/REPORT.md`.

## Layout

```
contracts/<name>/rust/   Rust contract (soroban-sdk 28)
contracts/<name>/zig/    Zig contract (zig-soroban-sdk, pinned in build.zig.zon)
harness/                 Measurement harness (cargo run --release)
results/REPORT.md        Latest results
scripts/bench.sh         Build everything and run the harness
```

## Adding a contract

1. Copy the Rust example to `contracts/<name>/rust/` and add it to `members`
   in the root `Cargo.toml`.
2. Write `contracts/<name>/zig/contract.zig` and add `<name>` to `contracts`
   in `build.zig`.
3. Add a `Pair` with its calls to `harness/src/scenarios.rs`.

[soroban-sdk]: https://github.com/stellar/rs-soroban-sdk
[zig-soroban-sdk]: https://github.com/stellar-experimental/zig-soroban-sdk
[antlion]: https://github.com/aryanbaranwal001/antlion
[soroban-examples/hello_world]: https://github.com/stellar/soroban-examples/tree/main/hello_world
[soroban-examples/increment]: https://github.com/stellar/soroban-examples/tree/main/increment
[soroban-examples/token]: https://github.com/stellar/soroban-examples/tree/main/token
[Zig]: https://ziglang.org/download/
[stellar-cli]: https://github.com/stellar/stellar-cli
