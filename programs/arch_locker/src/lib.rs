#![allow(clippy::diverging_sub_expression)] // Anchor 1.0.2 #[program] expansion on Rust 1.94.

use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{
        self, spl_token::state::AccountState, CloseAccount, Mint, Token, TokenAccount,
        TransferChecked,
    },
};

declare_id!("2jDqQUZY7yidwa8DTQm6vyFcptFPZ4QhxJn5ePGpgd2p");

const LOCK_SEED: &[u8] = b"lock";
const CURRENT_SCHEMA_VERSION: u8 = 1;
pub const MIN_LOCK_DURATION_SECONDS: i64 = 30 * 24 * 60 * 60;

#[program]
pub mod arch_locker {
    use super::*;

    pub fn create_lock(ctx: Context<CreateLock>, args: CreateLockArgs) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        validate_create_args(args, now)?;
        require!(
            ctx.accounts.beneficiary.key() != Pubkey::default(),
            LockerError::DefaultBeneficiary
        );
        require!(
            ctx.accounts.mint.freeze_authority.is_none(),
            LockerError::ActiveFreezeAuthority
        );
        require!(
            ctx.accounts.depositor_token_account.state != AccountState::Frozen,
            LockerError::FrozenSourceAccount
        );
        require!(
            ctx.accounts.beneficiary_token_account.state != AccountState::Frozen,
            LockerError::FrozenBeneficiaryAccount
        );
        let vault_before = ctx.accounts.vault.amount;
        token::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.depositor_token_account.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.depositor.to_account_info(),
                },
            ),
            args.amount,
            ctx.accounts.mint.decimals,
        )?;
        ctx.accounts.vault.reload()?;
        let expected_vault_amount = vault_before
            .checked_add(args.amount)
            .ok_or(LockerError::MathOverflow)?;
        require!(
            ctx.accounts.vault.amount == expected_vault_amount,
            LockerError::UnexpectedVaultDelta
        );

        let lock = &mut ctx.accounts.lock;
        lock.schema_version = CURRENT_SCHEMA_VERSION;
        lock.bump = ctx.bumps.lock;
        lock.mode = args.mode;
        lock.depositor = ctx.accounts.depositor.key();
        lock.beneficiary = ctx.accounts.beneficiary.key();
        lock.mint = ctx.accounts.mint.key();
        lock.vault = ctx.accounts.vault.key();
        lock.token_program = ctx.accounts.token_program.key();
        lock.nonce = args.nonce;
        lock.principal_amount = args.amount;
        lock.created_at = now;
        lock.unlock_at = args.unlock_at;
        lock.reserved = [0; 64];

        emit!(LockCreated {
            lock: lock.key(),
            depositor: lock.depositor,
            beneficiary: lock.beneficiary,
            mint: lock.mint,
            vault: lock.vault,
            nonce: lock.nonce,
            principal_amount: lock.principal_amount,
            created_at: lock.created_at,
            unlock_at: lock.unlock_at,
            mode: lock.mode,
        });
        Ok(())
    }

    pub fn extend_lock(ctx: Context<ControlLock>, new_unlock_at: i64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let lock = &mut ctx.accounts.lock;
        require!(lock.mode == LockMode::Timed, LockerError::PermanentLock);
        require!(
            new_unlock_at > lock.unlock_at && new_unlock_at > now,
            LockerError::LockCannotBeShortened
        );
        lock.unlock_at = new_unlock_at;
        emit!(LockExtended {
            lock: lock.key(),
            beneficiary: lock.beneficiary,
            new_unlock_at,
        });
        Ok(())
    }

    pub fn make_permanent(ctx: Context<ControlLock>) -> Result<()> {
        let lock = &mut ctx.accounts.lock;
        require!(lock.mode == LockMode::Timed, LockerError::PermanentLock);
        lock.mode = LockMode::Permanent;
        lock.unlock_at = 0;
        emit!(LockMadePermanent {
            lock: lock.key(),
            beneficiary: lock.beneficiary,
        });
        Ok(())
    }

    pub fn release_lock(ctx: Context<ReleaseLock>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let lock = &ctx.accounts.lock;
        require!(lock.mode == LockMode::Timed, LockerError::PermanentLock);
        require!(now >= lock.unlock_at, LockerError::LockNotMature);

        let released_amount = ctx.accounts.vault.amount;
        require!(released_amount > 0, LockerError::EmptyVault);

        let depositor_key = lock.depositor;
        let nonce = lock.nonce.to_le_bytes();
        let bump = [lock.bump];
        let signer_seeds: &[&[u8]] = &[LOCK_SEED, depositor_key.as_ref(), &nonce, &bump];

        token::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.vault.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.beneficiary_token_account.to_account_info(),
                    authority: ctx.accounts.lock.to_account_info(),
                },
                &[signer_seeds],
            ),
            released_amount,
            ctx.accounts.mint.decimals,
        )?;
        ctx.accounts.vault.reload()?;
        require!(
            ctx.accounts.vault.amount == 0,
            LockerError::UnexpectedVaultDelta
        );

        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            CloseAccount {
                account: ctx.accounts.vault.to_account_info(),
                destination: ctx.accounts.rent_recipient.to_account_info(),
                authority: ctx.accounts.lock.to_account_info(),
            },
            &[signer_seeds],
        ))?;

        emit!(LockReleased {
            lock: lock.key(),
            releaser: ctx.accounts.releaser.key(),
            depositor: lock.depositor,
            beneficiary: lock.beneficiary,
            mint: lock.mint,
            principal_amount: lock.principal_amount,
            released_amount,
            released_at: now,
        });
        Ok(())
    }
}

fn validate_create_args(args: CreateLockArgs, now: i64) -> Result<()> {
    require!(args.amount > 0, LockerError::ZeroAmount);
    match args.mode {
        LockMode::Timed => {
            let minimum_unlock_at = now
                .checked_add(MIN_LOCK_DURATION_SECONDS)
                .ok_or(LockerError::MathOverflow)?;
            require!(
                args.unlock_at >= minimum_unlock_at,
                LockerError::LockDurationTooShort
            );
        }
        LockMode::Permanent => {
            require!(args.unlock_at == 0, LockerError::InvalidPermanentUnlockTime);
        }
    }
    Ok(())
}

#[derive(Accounts)]
#[instruction(args: CreateLockArgs)]
pub struct CreateLock<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,
    pub beneficiary: Signer<'info>,
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        token::mint = mint,
        token::authority = depositor,
    )]
    pub depositor_token_account: Account<'info, TokenAccount>,
    #[account(
        dup,
        init_if_needed,
        payer = depositor,
        associated_token::mint = mint,
        associated_token::authority = beneficiary,
    )]
    pub beneficiary_token_account: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = depositor,
        space = 8 + Lock::INIT_SPACE,
        seeds = [LOCK_SEED, depositor.key().as_ref(), &args.nonce.to_le_bytes()],
        bump,
    )]
    pub lock: Account<'info, Lock>,
    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = mint,
        associated_token::authority = lock,
    )]
    pub vault: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ControlLock<'info> {
    pub beneficiary: Signer<'info>,
    #[account(
        mut,
        seeds = [LOCK_SEED, lock.depositor.as_ref(), &lock.nonce.to_le_bytes()],
        bump = lock.bump,
        has_one = beneficiary @ LockerError::WrongBeneficiary,
    )]
    pub lock: Account<'info, Lock>,
}

#[derive(Accounts)]
pub struct ReleaseLock<'info> {
    #[account(mut)]
    pub releaser: Signer<'info>,
    /// CHECK: This account is constrained to the immutable beneficiary.
    #[account(address = lock.beneficiary @ LockerError::WrongBeneficiary)]
    pub beneficiary: UncheckedAccount<'info>,
    /// CHECK: This account only receives rent and is constrained to the depositor.
    #[account(
        mut,
        address = lock.depositor @ LockerError::WrongRentRecipient,
    )]
    pub rent_recipient: UncheckedAccount<'info>,
    pub mint: Account<'info, Mint>,
    #[account(
        mut,
        close = rent_recipient,
        seeds = [LOCK_SEED, lock.depositor.as_ref(), &lock.nonce.to_le_bytes()],
        bump = lock.bump,
        has_one = mint @ LockerError::WrongMint,
        has_one = vault @ LockerError::WrongVault,
        constraint = lock.token_program == token_program.key() @ LockerError::WrongTokenProgram,
    )]
    pub lock: Account<'info, Lock>,
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = lock,
    )]
    pub vault: Account<'info, TokenAccount>,
    #[account(
        init_if_needed,
        payer = releaser,
        associated_token::mint = mint,
        associated_token::authority = beneficiary,
    )]
    pub beneficiary_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace, Debug)]
pub struct Lock {
    pub schema_version: u8,
    pub bump: u8,
    pub mode: LockMode,
    pub depositor: Pubkey,
    pub beneficiary: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub token_program: Pubkey,
    pub nonce: u64,
    pub principal_amount: u64,
    pub created_at: i64,
    pub unlock_at: i64,
    pub reserved: [u8; 64],
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum LockMode {
    Timed,
    Permanent,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateLockArgs {
    pub nonce: u64,
    pub amount: u64,
    pub mode: LockMode,
    pub unlock_at: i64,
}

#[error_code]
pub enum LockerError {
    #[msg("lock amount must be nonzero")]
    ZeroAmount,
    #[msg("beneficiary cannot be the default public key")]
    DefaultBeneficiary,
    #[msg("timed lock duration is below the 30-day minimum")]
    LockDurationTooShort,
    #[msg("permanent locks must use an unlock timestamp of zero")]
    InvalidPermanentUnlockTime,
    #[msg("checked arithmetic overflow")]
    MathOverflow,
    #[msg("legacy SPL token transfer produced an unexpected vault delta")]
    UnexpectedVaultDelta,
    #[msg("mint freeze authority must be revoked before locking")]
    ActiveFreezeAuthority,
    #[msg("depositor token account is frozen")]
    FrozenSourceAccount,
    #[msg("beneficiary token account is frozen")]
    FrozenBeneficiaryAccount,
    #[msg("beneficiary does not match the immutable lock beneficiary")]
    WrongBeneficiary,
    #[msg("rent recipient does not match the original depositor")]
    WrongRentRecipient,
    #[msg("mint does not match the locked asset")]
    WrongMint,
    #[msg("vault does not match the lock")]
    WrongVault,
    #[msg("token program does not match the lock")]
    WrongTokenProgram,
    #[msg("permanent locks cannot be extended or released")]
    PermanentLock,
    #[msg("a timed lock cannot be shortened")]
    LockCannotBeShortened,
    #[msg("lock has not reached its unlock timestamp")]
    LockNotMature,
    #[msg("lock vault is empty")]
    EmptyVault,
}

#[event]
pub struct LockCreated {
    pub lock: Pubkey,
    pub depositor: Pubkey,
    pub beneficiary: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub nonce: u64,
    pub principal_amount: u64,
    pub created_at: i64,
    pub unlock_at: i64,
    pub mode: LockMode,
}

#[event]
pub struct LockExtended {
    pub lock: Pubkey,
    pub beneficiary: Pubkey,
    pub new_unlock_at: i64,
}

#[event]
pub struct LockMadePermanent {
    pub lock: Pubkey,
    pub beneficiary: Pubkey,
}

#[event]
pub struct LockReleased {
    pub lock: Pubkey,
    pub releaser: Pubkey,
    pub depositor: Pubkey,
    pub beneficiary: Pubkey,
    pub mint: Pubkey,
    pub principal_amount: u64,
    pub released_amount: u64,
    pub released_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(mode: LockMode, amount: u64, unlock_at: i64) -> CreateLockArgs {
        CreateLockArgs {
            nonce: 7,
            amount,
            mode,
            unlock_at,
        }
    }

    #[test]
    fn timed_lock_accepts_exact_minimum_duration() {
        let now = 1_000;
        assert!(validate_create_args(
            args(LockMode::Timed, 1, now + MIN_LOCK_DURATION_SECONDS),
            now
        )
        .is_ok());
    }

    #[test]
    fn timed_lock_rejects_short_duration() {
        let now = 1_000;
        assert!(validate_create_args(
            args(LockMode::Timed, 1, now + MIN_LOCK_DURATION_SECONDS - 1),
            now
        )
        .is_err());
    }

    #[test]
    fn permanent_lock_requires_zero_unlock_time() {
        assert!(validate_create_args(args(LockMode::Permanent, 1, 0), 1_000).is_ok());
        assert!(validate_create_args(args(LockMode::Permanent, 1, 1), 1_000).is_err());
    }

    #[test]
    fn zero_amount_is_rejected_for_every_mode() {
        assert!(validate_create_args(args(LockMode::Permanent, 0, 0), 1_000).is_err());
        assert!(validate_create_args(
            args(LockMode::Timed, 0, 1_000 + MIN_LOCK_DURATION_SECONDS),
            1_000
        )
        .is_err());
    }

    #[test]
    fn timestamp_overflow_is_rejected() {
        assert!(validate_create_args(args(LockMode::Timed, 1, i64::MAX), i64::MAX).is_err());
    }
}
