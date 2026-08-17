use anchor_lang::{
    prelude::{Clock, Pubkey},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::associated_token::{
    get_associated_token_address, ID as ASSOCIATED_TOKEN_PROGRAM_ID,
};
use arch_locker::{
    accounts, instruction, CreateLockArgs, Lock, LockMode, ID, MIN_LOCK_DURATION_SECONDS,
};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token_interface::state::{Account as SplTokenAccount, AccountState, Mint as SplMint};

const LOCK_SEED: &[u8] = b"lock";
const DECIMALS: u8 = 6;

fn key(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

fn lock_pda(depositor: Pubkey, nonce: u64) -> Pubkey {
    Pubkey::find_program_address(&[LOCK_SEED, depositor.as_ref(), &nonce.to_le_bytes()], &ID).0
}

fn install_mint(svm: &mut LiteSVM, mint_address: Pubkey, decimals: u8) {
    install_mint_with_freeze_authority(svm, mint_address, decimals, COption::None);
}

fn install_mint_with_freeze_authority(
    svm: &mut LiteSVM,
    mint_address: Pubkey,
    decimals: u8,
    freeze_authority: COption<Pubkey>,
) {
    install_mint_with_authorities(svm, mint_address, decimals, COption::None, freeze_authority);
}

fn install_mint_with_authorities(
    svm: &mut LiteSVM,
    mint_address: Pubkey,
    decimals: u8,
    mint_authority: COption<Pubkey>,
    freeze_authority: COption<Pubkey>,
) {
    let mint = SplMint {
        mint_authority,
        supply: 10_000_000_000,
        decimals,
        is_initialized: true,
        freeze_authority,
    };
    let mut data = vec![0; SplMint::LEN];
    SplMint::pack(mint, &mut data).unwrap();
    svm.set_account(
        mint_address,
        Account {
            lamports: svm.minimum_balance_for_rent_exemption(SplMint::LEN),
            data,
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

#[test]
fn arch_swap_style_lp_tokens_can_be_locked_and_released() {
    let mut svm = new_vm();
    let provider = Keypair::new();
    let releaser = Keypair::new();
    svm.airdrop(&provider.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&releaser.pubkey(), 10_000_000_000).unwrap();

    let pool = key(28);
    let lp_mint = key(29);
    let provider_lp = key(30);
    let amount = 9_000_000_000;
    let nonce = 8;
    install_mint_with_authorities(&mut svm, lp_mint, 9, COption::Some(pool), COption::None);
    install_token_account(&mut svm, provider_lp, lp_mint, provider.pubkey(), amount);

    let now = 10_000;
    let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
    set_time(&mut svm, now);
    send(
        &mut svm,
        &provider,
        &[&provider],
        create_lock_instruction(
            provider.pubkey(),
            provider.pubkey(),
            lp_mint,
            provider_lp,
            CreateLockArgs {
                nonce,
                amount,
                mode: LockMode::Timed,
                unlock_at,
            },
        ),
    )
    .unwrap();
    let lock = lock_pda(provider.pubkey(), nonce);
    let vault = get_associated_token_address(&lock, &lp_mint);
    assert_eq!(read_lock(&svm, lock).principal_amount, amount);
    assert_eq!(token_amount(&svm, vault), amount);

    set_time(&mut svm, unlock_at);
    svm.expire_blockhash();
    send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            provider.pubkey(),
            provider.pubkey(),
            lp_mint,
            nonce,
        ),
    )
    .unwrap();
    assert_eq!(
        token_amount(
            &svm,
            get_associated_token_address(&provider.pubkey(), &lp_mint)
        ),
        amount
    );
}

fn install_token_account(
    svm: &mut LiteSVM,
    address: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    install_token_account_with_state(svm, address, mint, owner, amount, AccountState::Initialized);
}

fn install_token_account_with_state(
    svm: &mut LiteSVM,
    address: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
    state: AccountState,
) {
    let token_account = SplTokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0; SplTokenAccount::LEN];
    SplTokenAccount::pack(token_account, &mut data).unwrap();
    svm.set_account(
        address,
        Account {
            lamports: svm.minimum_balance_for_rent_exemption(SplTokenAccount::LEN),
            data,
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn new_vm() -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(
        ID,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/deploy/arch_locker.so"
        )),
    )
    .unwrap();
    svm
}

fn send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    signers: &[&Keypair],
    ix: anchor_lang::solana_program::instruction::Instruction,
) -> Result<(), String> {
    let transaction = Transaction::new(
        signers,
        Message::new(&[ix], Some(&payer.pubkey())),
        svm.latest_blockhash(),
    );
    svm.send_transaction(transaction)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

fn create_lock_instruction(
    depositor: Pubkey,
    beneficiary: Pubkey,
    mint: Pubkey,
    source: Pubkey,
    args: CreateLockArgs,
) -> anchor_lang::solana_program::instruction::Instruction {
    let lock = lock_pda(depositor, args.nonce);
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ID,
        accounts: accounts::CreateLock {
            depositor,
            beneficiary,
            mint,
            depositor_token_account: source,
            beneficiary_token_account: get_associated_token_address(&beneficiary, &mint),
            lock,
            vault: get_associated_token_address(&lock, &mint),
            token_program: spl_token_interface::ID,
            associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::CreateLock { args }.data(),
    }
}

fn release_lock_instruction(
    releaser: Pubkey,
    depositor: Pubkey,
    beneficiary: Pubkey,
    mint: Pubkey,
    nonce: u64,
) -> anchor_lang::solana_program::instruction::Instruction {
    let lock = lock_pda(depositor, nonce);
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ID,
        accounts: accounts::ReleaseLock {
            releaser,
            beneficiary,
            rent_recipient: depositor,
            mint,
            lock,
            vault: get_associated_token_address(&lock, &mint),
            beneficiary_token_account: get_associated_token_address(&beneficiary, &mint),
            token_program: spl_token_interface::ID,
            associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::ReleaseLock {}.data(),
    }
}

fn control_lock_instruction(
    beneficiary: Pubkey,
    depositor: Pubkey,
    nonce: u64,
    new_unlock_at: i64,
) -> anchor_lang::solana_program::instruction::Instruction {
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ID,
        accounts: accounts::ControlLock {
            beneficiary,
            lock: lock_pda(depositor, nonce),
        }
        .to_account_metas(None),
        data: instruction::ExtendLock { new_unlock_at }.data(),
    }
}

fn make_permanent_instruction(
    beneficiary: Pubkey,
    depositor: Pubkey,
    nonce: u64,
) -> anchor_lang::solana_program::instruction::Instruction {
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ID,
        accounts: accounts::ControlLock {
            beneficiary,
            lock: lock_pda(depositor, nonce),
        }
        .to_account_metas(None),
        data: instruction::MakePermanent {}.data(),
    }
}

fn transfer_tokens(
    svm: &mut LiteSVM,
    owner: &Keypair,
    source: Pubkey,
    mint: Pubkey,
    destination: Pubkey,
    amount: u64,
) -> Result<(), String> {
    let ix = spl_token_interface::instruction::transfer_checked(
        &spl_token_interface::ID,
        &source,
        &mint,
        &destination,
        &owner.pubkey(),
        &[],
        amount,
        DECIMALS,
    )
    .unwrap();
    send(svm, owner, &[owner], ix)
}

fn read_lock(svm: &LiteSVM, address: Pubkey) -> Lock {
    let account = svm.get_account(&address).unwrap();
    Lock::try_deserialize(&mut account.data.as_slice()).unwrap()
}

fn token_amount(svm: &LiteSVM, address: Pubkey) -> u64 {
    let account = svm.get_account(&address).unwrap();
    SplTokenAccount::unpack(&account.data).unwrap().amount
}

fn set_time(svm: &mut LiteSVM, unix_timestamp: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = unix_timestamp;
    svm.set_sysvar(&clock);
}

#[test]
fn timed_lock_releases_principal_and_donations_only_to_beneficiary() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    let releaser = Keypair::new();
    let donor = Keypair::new();
    for wallet in [&depositor, &beneficiary, &releaser, &donor] {
        svm.airdrop(&wallet.pubkey(), 10_000_000_000).unwrap();
    }

    let mint = key(31);
    let source = key(32);
    let donor_source = key(33);
    let nonce = 9;
    let principal = 5_000_000;
    let donation = 77;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), principal);
    install_token_account(&mut svm, donor_source, mint, donor.pubkey(), donation);

    let now = 10_000;
    let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
    set_time(&mut svm, now);
    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: principal,
                mode: LockMode::Timed,
                unlock_at,
            },
        ),
    )
    .unwrap();

    let lock = lock_pda(depositor.pubkey(), nonce);
    let vault = get_associated_token_address(&lock, &mint);
    let stored = read_lock(&svm, lock);
    assert_eq!(stored.depositor, depositor.pubkey());
    assert_eq!(stored.beneficiary, beneficiary.pubkey());
    assert_eq!(stored.principal_amount, principal);
    assert_eq!(stored.mode, LockMode::Timed);
    assert_eq!(token_amount(&svm, source), 0);
    assert_eq!(token_amount(&svm, vault), principal);

    let lock_before = svm.get_account(&lock).unwrap();
    assert!(send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            nonce,
        ),
    )
    .is_err());
    assert_eq!(svm.get_account(&lock).unwrap().data, lock_before.data);
    assert_eq!(token_amount(&svm, vault), principal);

    svm.expire_blockhash();
    transfer_tokens(&mut svm, &donor, donor_source, mint, vault, donation).unwrap();
    assert_eq!(token_amount(&svm, vault), principal + donation);

    set_time(&mut svm, unlock_at);
    svm.expire_blockhash();
    let depositor_lamports_before_release = svm.get_balance(&depositor.pubkey()).unwrap();
    send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            nonce,
        ),
    )
    .unwrap();

    let beneficiary_account = get_associated_token_address(&beneficiary.pubkey(), &mint);
    assert_eq!(
        token_amount(&svm, beneficiary_account),
        principal + donation
    );
    assert!(svm.get_account(&vault).is_none());
    assert!(svm.get_account(&lock).is_none());
    assert!(svm.get_balance(&depositor.pubkey()).unwrap() > depositor_lamports_before_release);
}

#[test]
fn self_beneficiary_can_lock_from_the_canonical_source() {
    let mut svm = new_vm();
    let owner = Keypair::new();
    let releaser = Keypair::new();
    svm.airdrop(&owner.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&releaser.pubkey(), 10_000_000_000).unwrap();

    let mint = key(34);
    let source = get_associated_token_address(&owner.pubkey(), &mint);
    let nonce = 21;
    let principal = 5_000_000;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, owner.pubkey(), principal);

    let now = 15_000;
    let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
    set_time(&mut svm, now);
    send(
        &mut svm,
        &owner,
        &[&owner],
        create_lock_instruction(
            owner.pubkey(),
            owner.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: principal,
                mode: LockMode::Timed,
                unlock_at,
            },
        ),
    )
    .unwrap();

    let lock = lock_pda(owner.pubkey(), nonce);
    let vault = get_associated_token_address(&lock, &mint);
    assert_eq!(token_amount(&svm, source), 0);
    assert_eq!(token_amount(&svm, vault), principal);

    set_time(&mut svm, unlock_at);
    svm.expire_blockhash();
    send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            owner.pubkey(),
            owner.pubkey(),
            mint,
            nonce,
        ),
    )
    .unwrap();
    assert_eq!(token_amount(&svm, source), principal);
    assert!(svm.get_account(&vault).is_none());
    assert!(svm.get_account(&lock).is_none());
}

#[test]
fn only_beneficiary_can_extend_and_time_cannot_move_backward() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    let attacker = Keypair::new();
    for wallet in [&depositor, &beneficiary, &attacker] {
        svm.airdrop(&wallet.pubkey(), 10_000_000_000).unwrap();
    }
    let mint = key(41);
    let source = key(42);
    let nonce = 10;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    let now = 20_000;
    let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
    set_time(&mut svm, now);
    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at,
            },
        ),
    )
    .unwrap();

    assert!(send(
        &mut svm,
        &attacker,
        &[&attacker],
        control_lock_instruction(
            attacker.pubkey(),
            depositor.pubkey(),
            nonce,
            unlock_at + 100,
        ),
    )
    .is_err());
    svm.expire_blockhash();
    let mut missing_signature = control_lock_instruction(
        beneficiary.pubkey(),
        depositor.pubkey(),
        nonce,
        unlock_at + 100,
    );
    missing_signature.accounts[0].is_signer = false;
    assert!(send(&mut svm, &attacker, &[&attacker], missing_signature,).is_err());
    svm.expire_blockhash();
    assert!(send(
        &mut svm,
        &beneficiary,
        &[&beneficiary],
        control_lock_instruction(
            beneficiary.pubkey(),
            depositor.pubkey(),
            nonce,
            unlock_at - 1,
        ),
    )
    .is_err());
    svm.expire_blockhash();
    send(
        &mut svm,
        &beneficiary,
        &[&beneficiary],
        control_lock_instruction(
            beneficiary.pubkey(),
            depositor.pubkey(),
            nonce,
            unlock_at + 100,
        ),
    )
    .unwrap();
    assert_eq!(
        read_lock(&svm, lock_pda(depositor.pubkey(), nonce)).unlock_at,
        unlock_at + 100
    );
}

#[test]
fn permanent_lock_has_no_release_path() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    let releaser = Keypair::new();
    for wallet in [&depositor, &beneficiary, &releaser] {
        svm.airdrop(&wallet.pubkey(), 10_000_000_000).unwrap();
    }
    let mint = key(51);
    let source = key(52);
    let nonce = 11;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Permanent,
                unlock_at: 0,
            },
        ),
    )
    .unwrap();

    set_time(&mut svm, i64::MAX / 2);
    svm.expire_blockhash();
    assert!(send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            nonce,
        ),
    )
    .is_err());
    let lock = lock_pda(depositor.pubkey(), nonce);
    assert_eq!(read_lock(&svm, lock).mode, LockMode::Permanent);
    assert_eq!(
        token_amount(&svm, get_associated_token_address(&lock, &mint)),
        100
    );
}

#[test]
fn prefunded_vault_cannot_block_creation_or_redirect_assets() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    let releaser = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
    svm.airdrop(&releaser.pubkey(), 10_000_000_000).unwrap();
    let mint = key(61);
    let source = key(62);
    let nonce = 12;
    let lock = lock_pda(depositor.pubkey(), nonce);
    let vault = get_associated_token_address(&lock, &mint);
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    install_token_account(&mut svm, vault, mint, lock, 1);
    let beneficiary_token_account = get_associated_token_address(&beneficiary.pubkey(), &mint);
    install_token_account(
        &mut svm,
        beneficiary_token_account,
        mint,
        beneficiary.pubkey(),
        50,
    );
    let now = 30_000;
    let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
    set_time(&mut svm, now);

    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at,
            },
        ),
    )
    .unwrap();
    assert_eq!(token_amount(&svm, source), 0);
    assert_eq!(token_amount(&svm, vault), 101);
    assert_eq!(read_lock(&svm, lock).principal_amount, 100);

    set_time(&mut svm, unlock_at);
    svm.expire_blockhash();
    send(
        &mut svm,
        &releaser,
        &[&releaser],
        release_lock_instruction(
            releaser.pubkey(),
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            nonce,
        ),
    )
    .unwrap();
    assert_eq!(token_amount(&svm, beneficiary_token_account), 151);
    assert!(svm.get_account(&vault).is_none());
    assert!(svm.get_account(&lock).is_none());
}

#[test]
fn timed_lock_can_be_made_permanent_only_by_beneficiary() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    let attacker = Keypair::new();
    for wallet in [&depositor, &beneficiary, &attacker] {
        svm.airdrop(&wallet.pubkey(), 10_000_000_000).unwrap();
    }
    let mint = key(71);
    let source = key(72);
    let nonce = 13;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    let now = 40_000;
    set_time(&mut svm, now);
    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at: now + MIN_LOCK_DURATION_SECONDS,
            },
        ),
    )
    .unwrap();

    assert!(send(
        &mut svm,
        &attacker,
        &[&attacker],
        make_permanent_instruction(attacker.pubkey(), depositor.pubkey(), nonce),
    )
    .is_err());
    svm.expire_blockhash();
    let mut missing_signature =
        make_permanent_instruction(beneficiary.pubkey(), depositor.pubkey(), nonce);
    missing_signature.accounts[0].is_signer = false;
    assert!(send(&mut svm, &attacker, &[&attacker], missing_signature,).is_err());
    svm.expire_blockhash();
    send(
        &mut svm,
        &beneficiary,
        &[&beneficiary],
        make_permanent_instruction(beneficiary.pubkey(), depositor.pubkey(), nonce),
    )
    .unwrap();
    let stored = read_lock(&svm, lock_pda(depositor.pubkey(), nonce));
    assert_eq!(stored.mode, LockMode::Permanent);
    assert_eq!(stored.unlock_at, 0);
}

#[test]
fn duplicate_nonce_fails_atomically() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
    let mint = key(81);
    let first_source = key(82);
    let second_source = key(83);
    let nonce = 14;
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, first_source, mint, depositor.pubkey(), 100);
    install_token_account(&mut svm, second_source, mint, depositor.pubkey(), 200);
    let now = 50_000;
    let args = CreateLockArgs {
        nonce,
        amount: 100,
        mode: LockMode::Timed,
        unlock_at: now + MIN_LOCK_DURATION_SECONDS,
    };
    set_time(&mut svm, now);
    send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            first_source,
            args,
        ),
    )
    .unwrap();

    svm.expire_blockhash();
    assert!(send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            second_source,
            CreateLockArgs {
                amount: 200,
                ..args
            },
        ),
    )
    .is_err());
    assert_eq!(token_amount(&svm, second_source), 200);
}

#[test]
fn frozen_source_and_token_2022_program_are_rejected() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
    let mint = key(91);
    let frozen_source = key(92);
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, frozen_source, mint, depositor.pubkey(), 100);
    let mut frozen = svm.get_account(&frozen_source).unwrap();
    let mut frozen_state = SplTokenAccount::unpack(&frozen.data).unwrap();
    frozen_state.state = AccountState::Frozen;
    SplTokenAccount::pack(frozen_state, &mut frozen.data).unwrap();
    svm.set_account(frozen_source, frozen).unwrap();
    let now = 60_000;
    let args = CreateLockArgs {
        nonce: 16,
        amount: 100,
        mode: LockMode::Timed,
        unlock_at: now + MIN_LOCK_DURATION_SECONDS,
    };
    set_time(&mut svm, now);
    let valid = create_lock_instruction(
        depositor.pubkey(),
        beneficiary.pubkey(),
        mint,
        frozen_source,
        args,
    );
    assert!(send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        valid.clone()
    )
    .is_err());
    assert_eq!(token_amount(&svm, frozen_source), 100);

    svm.expire_blockhash();
    let mut token_2022 = valid;
    let token_program_meta = token_2022
        .accounts
        .iter_mut()
        .find(|meta| meta.pubkey == spl_token_interface::ID)
        .unwrap();
    token_program_meta.pubkey = anchor_spl::token_2022::ID;
    assert!(send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        token_2022
    )
    .is_err());
    assert_eq!(token_amount(&svm, frozen_source), 100);
}

#[test]
fn active_mint_freeze_authority_is_rejected_atomically() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
    let mint = key(93);
    let source = key(94);
    let freeze_authority = key(95);
    install_mint_with_freeze_authority(&mut svm, mint, DECIMALS, COption::Some(freeze_authority));
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    let now = 65_000;
    set_time(&mut svm, now);
    let result = send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce: 19,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at: now + MIN_LOCK_DURATION_SECONDS,
            },
        ),
    );
    assert!(result.is_err());
    assert_eq!(token_amount(&svm, source), 100);
    assert!(svm.get_account(&lock_pda(depositor.pubkey(), 19)).is_none());
}

#[test]
fn frozen_beneficiary_account_is_rejected_atomically() {
    let mut svm = new_vm();
    let depositor = Keypair::new();
    let beneficiary = Keypair::new();
    svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
    let mint = key(96);
    let source = key(97);
    let beneficiary_token_account = get_associated_token_address(&beneficiary.pubkey(), &mint);
    install_mint(&mut svm, mint, DECIMALS);
    install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
    install_token_account_with_state(
        &mut svm,
        beneficiary_token_account,
        mint,
        beneficiary.pubkey(),
        0,
        AccountState::Frozen,
    );
    let now = 67_000;
    let nonce = 20;
    set_time(&mut svm, now);
    let result = send(
        &mut svm,
        &depositor,
        &[&depositor, &beneficiary],
        create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at: now + MIN_LOCK_DURATION_SECONDS,
            },
        ),
    );
    assert!(result.is_err());
    assert_eq!(token_amount(&svm, source), 100);
    assert!(svm
        .get_account(&lock_pda(depositor.pubkey(), nonce))
        .is_none());
    assert!(svm
        .get_account(&get_associated_token_address(
            &lock_pda(depositor.pubkey(), nonce),
            &mint,
        ))
        .is_none());
}

#[test]
fn substituted_create_accounts_never_move_principal() {
    let cases = [
        ("missing depositor signature", 0usize, None, Some(false)),
        ("missing beneficiary signature", 1, None, Some(false)),
        ("wrong mint", 2, Some(key(101)), None),
        ("wrong source", 3, Some(key(102)), None),
        ("wrong beneficiary token account", 4, Some(key(107)), None),
        ("wrong lock", 5, Some(key(103)), None),
        ("wrong vault", 6, Some(key(104)), None),
        (
            "Token-2022 program",
            7,
            Some(anchor_spl::token_2022::ID),
            None,
        ),
        ("wrong associated token program", 8, Some(key(105)), None),
        ("wrong system program", 9, Some(key(106)), None),
    ];
    for (label, index, replacement, signer) in cases {
        let mut svm = new_vm();
        let depositor = Keypair::new();
        let beneficiary = Keypair::new();
        let alternate_payer = Keypair::new();
        svm.airdrop(&depositor.pubkey(), 10_000_000_000).unwrap();
        svm.airdrop(&beneficiary.pubkey(), 1_000_000).unwrap();
        svm.airdrop(&alternate_payer.pubkey(), 10_000_000_000)
            .unwrap();
        let mint = key(111);
        let source = key(112);
        let nonce = 17;
        install_mint(&mut svm, mint, DECIMALS);
        install_mint(&mut svm, key(101), DECIMALS);
        install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
        install_token_account(&mut svm, key(102), mint, key(113), 100);
        let now = 70_000;
        set_time(&mut svm, now);
        let mut ix = create_lock_instruction(
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            source,
            CreateLockArgs {
                nonce,
                amount: 100,
                mode: LockMode::Timed,
                unlock_at: now + MIN_LOCK_DURATION_SECONDS,
            },
        );
        if let Some(pubkey) = replacement {
            ix.accounts[index].pubkey = pubkey;
        }
        if let Some(is_signer) = signer {
            ix.accounts[index].is_signer = is_signer;
        }
        let result = if label == "missing depositor signature" {
            send(
                &mut svm,
                &alternate_payer,
                &[&alternate_payer, &beneficiary],
                ix,
            )
        } else if label == "missing beneficiary signature" {
            send(&mut svm, &depositor, &[&depositor], ix)
        } else {
            send(&mut svm, &depositor, &[&depositor, &beneficiary], ix)
        };
        assert!(result.is_err(), "accepted {label}");
        assert_eq!(token_amount(&svm, source), 100, "moved funds for {label}");
        assert!(
            svm.get_account(&lock_pda(depositor.pubkey(), nonce))
                .is_none(),
            "created lock for {label}"
        );
    }
}

#[test]
fn substituted_release_accounts_preserve_lock_and_vault() {
    let cases = [
        ("missing releaser signature", 0usize, None, Some(false)),
        ("wrong beneficiary", 1, Some(key(121)), None),
        ("wrong rent recipient", 2, Some(key(122)), None),
        ("wrong mint", 3, Some(key(123)), None),
        ("wrong lock", 4, Some(key(124)), None),
        ("wrong vault", 5, Some(key(125)), None),
        ("wrong beneficiary token account", 6, Some(key(126)), None),
        (
            "Token-2022 program",
            7,
            Some(anchor_spl::token_2022::ID),
            None,
        ),
        ("wrong associated token program", 8, Some(key(127)), None),
        ("wrong system program", 9, Some(key(128)), None),
    ];
    for (label, index, replacement, signer) in cases {
        let mut svm = new_vm();
        let depositor = Keypair::new();
        let beneficiary = Keypair::new();
        let releaser = Keypair::new();
        let alternate_payer = Keypair::new();
        for wallet in [&depositor, &beneficiary, &releaser, &alternate_payer] {
            svm.airdrop(&wallet.pubkey(), 10_000_000_000).unwrap();
        }
        let mint = key(131);
        let source = key(132);
        let nonce = 18;
        install_mint(&mut svm, mint, DECIMALS);
        install_mint(&mut svm, key(123), DECIMALS);
        install_token_account(&mut svm, source, mint, depositor.pubkey(), 100);
        let now = 80_000;
        let unlock_at = now + MIN_LOCK_DURATION_SECONDS;
        set_time(&mut svm, now);
        send(
            &mut svm,
            &depositor,
            &[&depositor, &beneficiary],
            create_lock_instruction(
                depositor.pubkey(),
                beneficiary.pubkey(),
                mint,
                source,
                CreateLockArgs {
                    nonce,
                    amount: 100,
                    mode: LockMode::Timed,
                    unlock_at,
                },
            ),
        )
        .unwrap();
        set_time(&mut svm, unlock_at);
        svm.expire_blockhash();
        let lock = lock_pda(depositor.pubkey(), nonce);
        let vault = get_associated_token_address(&lock, &mint);
        let lock_before = svm.get_account(&lock).unwrap();
        let vault_before = svm.get_account(&vault).unwrap();
        let mut ix = release_lock_instruction(
            releaser.pubkey(),
            depositor.pubkey(),
            beneficiary.pubkey(),
            mint,
            nonce,
        );
        if let Some(pubkey) = replacement {
            ix.accounts[index].pubkey = pubkey;
        }
        if let Some(is_signer) = signer {
            ix.accounts[index].is_signer = is_signer;
        }
        let result = if label == "missing releaser signature" {
            send(&mut svm, &alternate_payer, &[&alternate_payer], ix)
        } else {
            send(&mut svm, &releaser, &[&releaser], ix)
        };
        assert!(result.is_err(), "accepted {label}");
        assert_eq!(
            svm.get_account(&lock).unwrap(),
            lock_before,
            "changed lock for {label}"
        );
        assert_eq!(
            svm.get_account(&vault).unwrap(),
            vault_before,
            "changed vault for {label}"
        );
    }
}

#[test]
fn every_public_handler_is_covered_by_the_security_manifest() {
    let source = include_str!("../src/lib.rs");
    let module_start = source.find("pub mod arch_locker {").unwrap();
    let handlers: Vec<&str> = source[module_start..]
        .lines()
        .skip(1)
        .take_while(|line| *line != "}")
        .filter_map(|line| {
            line.trim_start()
                .strip_prefix("pub fn ")
                .and_then(|rest| rest.split('(').next())
        })
        .collect();
    assert_eq!(
        handlers,
        [
            "create_lock",
            "extend_lock",
            "make_permanent",
            "release_lock"
        ]
    );
}
