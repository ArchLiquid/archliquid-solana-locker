# Dependency audit

The release uses exact Anchor 1.0.2 constraints and a committed Cargo lockfile.
The on-chain dependency graph supports the legacy SPL Token program only.

The 2026-08-16 RustSec scan found no known vulnerabilities. It reported six
allowed warnings in pinned transitive dependencies:

- `RUSTSEC-2021-0139` for `ansi_term`
- `RUSTSEC-2025-0141` for `bincode 1.3.3`
- `RUSTSEC-2024-0388` for `derivative`
- `RUSTSEC-2025-0161` for `libsecp256k1`
- `RUSTSEC-2024-0436` for `paste`
- `RUSTSEC-2026-0097` for `rand 0.7.3`

These are unmaintained or informational warnings, not published vulnerability
advisories. The `rand 0.7.3` path reaches this repository through the LiteSVM
development test graph and is not linked into the deployed program. The other
warnings are pinned through the Solana and Anchor dependency graph and are
tracked until compatible upstream replacements exist.

CI fails on any advisory or warning outside this explicit list.
