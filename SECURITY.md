# Security policy

## Release status

ArchLocker is deployed immutably on Solana devnet at
`6K1jwGGQBGZMYCe6zcxDN3LV46yANcQaTh2wf3c2gfBi`. Its executable SHA-256 is
`a6a49b36e25189f2a076c18c32eb45658b86f50294633d98f62ed8d742374349`.
The ProgramData upgrade authority is revoked, so an administrator cannot
replace the deployed code.

The devnet release passed the project's production code, adversarial test,
reproducible build, and live lifecycle gates. No independent audit is claimed.
The release is not approved for mainnet or real-value custody.

## Supported security boundary

Version 1 supports legacy SPL Token mints only. Token-2022, programmable or
compressed positions, and AMM-specific fee collection are not supported.

The locker provides these custody invariants:

- Each lock has an immutable mint, beneficiary, depositor, vault, and mode.
- The beneficiary authorizes creation.
- The vault is the canonical associated token account owned by the lock PDA.
- Creation transfers an exact nonzero amount.
- Active mint freeze authorities, frozen sources, and frozen beneficiary
  destinations are rejected.
- Timed locks cannot be shortened and cannot release before chain time reaches
  the recorded unlock time.
- Permanent locks have no release path.
- Mature release can only send the full vault balance to the beneficiary's
  canonical associated token account.
- There is no admin withdrawal, early cancellation, arbitrary destination, or
  emergency custody exception.

Mint authority is outside the locker. A mint authority can dilute a fungible
asset and must be evaluated by users and integrations before deposit.

## Reporting a vulnerability

Do not publish an unresolved vulnerability in a public issue. Use GitHub's
private vulnerability reporting for this repository. Include the affected
instruction or account constraint, an executable reproduction when possible,
the impact, and any proposed mitigation.

Reports should use valueless localnet or devnet assets. Do not test against
third-party funds or attempt to move assets that you do not own.
