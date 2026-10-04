//! The calls made against each contract. Rust and Zig builds run the exact
//! same list, so their per-call numbers line up row by row.
//!
//! Each call names the account that signs it. Making the account that has to
//! authorize the call also the transaction source means auth is satisfied by
//! the source account, with no signatures or nonces to fake.

use soroban_env_host::xdr::{
    AccountId, Int128Parts, PublicKey, ScAddress, ScString, ScVal, Uint256,
};

use crate::network::LEDGER_SEQ;

pub struct Pair {
    pub name: &'static str,
    /// File name of the Rust build in out/rust/.
    pub rust_wasm: &'static str,
    pub constructor_args: Vec<ScVal>,
    pub steps: Vec<Step>,
}

pub struct Step {
    pub label: &'static str,
    pub func: &'static str,
    pub args: Vec<ScVal>,
    pub source: AccountId,
}

pub fn account(n: u8) -> AccountId {
    AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([n; 32])))
}

/// The account that uploads and deploys every contract, and the token admin.
pub fn deployer() -> AccountId {
    account(1)
}

fn addr(n: u8) -> ScVal {
    ScVal::Address(ScAddress::Account(account(n)))
}

fn i128(v: i128) -> ScVal {
    ScVal::I128(Int128Parts { hi: (v >> 64) as i64, lo: v as u64 })
}

fn string(s: &str) -> ScVal {
    ScVal::String(ScString(s.try_into().unwrap()))
}

fn step(label: &'static str, func: &'static str, args: Vec<ScVal>, source: AccountId) -> Step {
    Step { label, func, args, source }
}

pub fn all() -> Vec<Pair> {
    let (admin, alice, bob, carol) = (1, 2, 3, 4);
    vec![
        Pair {
            name: "hello",
            rust_wasm: "soroban_hello_world_contract.wasm",
            constructor_args: vec![],
            steps: vec![step("hello(\"World\")", "hello", vec![string("World")], account(alice))],
        },
        Pair {
            name: "increment",
            rust_wasm: "soroban_increment_contract.wasm",
            constructor_args: vec![],
            steps: vec![
                step("increment (first)", "increment", vec![], account(alice)),
                step("increment (again)", "increment", vec![], account(alice)),
            ],
        },
        Pair {
            name: "token",
            rust_wasm: "soroban_token_contract.wasm",
            constructor_args: vec![addr(admin), ScVal::U32(7), string("Example Token"), string("EXT")],
            steps: vec![
                step("mint (new balance)", "mint", vec![addr(alice), i128(1_000_000)], account(admin)),
                step("mint (existing balance)", "mint", vec![addr(alice), i128(1_000)], account(admin)),
                step("balance", "balance", vec![addr(alice)], account(alice)),
                step("transfer (new recipient)", "transfer", vec![addr(alice), addr(bob), i128(100)], account(alice)),
                step("transfer (existing recipient)", "transfer", vec![addr(alice), addr(bob), i128(100)], account(alice)),
                step("approve", "approve", vec![addr(alice), addr(carol), i128(500), ScVal::U32(LEDGER_SEQ + 1000)], account(alice)),
                step("allowance", "allowance", vec![addr(alice), addr(carol)], account(alice)),
                step("transfer_from", "transfer_from", vec![addr(carol), addr(alice), addr(bob), i128(50)], account(carol)),
                step("burn", "burn", vec![addr(alice), i128(10)], account(alice)),
                step("burn_from", "burn_from", vec![addr(carol), addr(alice), i128(10)], account(carol)),
                step("decimals", "decimals", vec![], account(alice)),
                step("name", "name", vec![], account(alice)),
                step("symbol", "symbol", vec![], account(alice)),
                step("set_admin", "set_admin", vec![addr(alice)], account(admin)),
            ],
        },
    ]
}
