# ArchLocker

ArchLocker is an owned Solana program for time-locking or permanently locking
legacy SPL liquidity assets. Its immutable devnet release passed the project's
production code, adversarial test, reproducible build, and live lifecycle
gates. It has not been independently audited and is not approved for mainnet
or real-value custody.

## Devnet release

- Program: `6K1jwGGQBGZMYCe6zcxDN3LV46yANcQaTh2wf3c2gfBi`
- Executable SHA-256:
  `a6a49b36e25189f2a076c18c32eb45658b86f50294633d98f62ed8d742374349`
- Upgrade authority: revoked
- Explorer:
  <https://explorer.solana.com/address/6K1jwGGQBGZMYCe6zcxDN3LV46yANcQaTh2wf3c2gfBi?cluster=devnet>

## Supported assets

Version 1 accepts legacy SPL Token mints. This covers fungible LP tokens and
legacy SPL position tokens. The program guarantees custody and release rules;
it does not determine whether a mint represents a valid position in a
particular AMM.

Token-2022, compressed positions, programmable NFTs, and AMM-specific fee
management are not supported in version 1.

The locker rejects any mint with an active freeze authority because that
authority could freeze the vault and block a timed release. Existing mint
authority remains an external dilution risk that users and integrations must
verify before locking an asset.

Creation also establishes and validates the beneficiary's canonical token
account. A beneficiary account left frozen before the mint's freeze authority
was revoked is rejected, preventing an irreversible release failure.

## Instructions

- `create_lock` creates a PDA lock, establishes a usable canonical beneficiary
  token account, and transfers an exact nonzero amount into the lock's
  canonical associated token vault. The depositor and beneficiary both
  authorize creation. A user can safely be both depositor and beneficiary while
  depositing from that same canonical token account.
- `extend_lock` lets the beneficiary move a timed unlock date forward. It can
  never shorten the lock.
- `make_permanent` lets the beneficiary irreversibly convert a timed lock into
  a permanent lock.
- `release_lock` can be called by anyone after maturity. It transfers the full
  vault balance only to the beneficiary's canonical associated token account,
  closes the empty vault and lock state, and returns rent to the depositor.

Timed locks have a minimum duration of 30 days. Permanent locks have no release
instruction. There is no admin withdrawal, emergency seizure, arbitrary
recipient, cancellation, or early-release path.

Direct token donations do not change the beneficiary or recorded principal.
Any donated balance is delivered to the same beneficiary at maturity so a
third party cannot prevent vault closure by sending tokens to it.

## Permanence requirement

Program logic alone cannot make a lock permanent while the deployed program is
upgradeable. The devnet release has no upgrade authority, so its code cannot be
replaced by an admin. Any future deployment offering permanent custody must
independently satisfy the same reviewed-build and revoked-authority rules.

## Local verification

From the Solana workspace root:

```sh
cargo fmt --all --check
cargo clippy -p arch-locker --all-targets -- -D warnings
anchor build --ignore-keys --program-name arch_locker
cargo test -p arch-locker
node scripts/verify-arch-locker-devnet-release.mjs
```

The tests cover time and amount validation, exact custody, pre-funded and
post-funded vaults, early-release rollback, permissionless mature release,
permanent locks, beneficiary authorization, Token-2022 rejection, active freeze
authority rejection, frozen accounts, duplicate nonces, rent return, and
account substitution. The live devnet harness also covers self-beneficiary and
distinct-beneficiary creation, donations, extension, permanent conversion,
rejected releases, and atomic rollback.
