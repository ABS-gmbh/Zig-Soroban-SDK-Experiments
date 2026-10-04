//! Runs host functions the way the network does, one transaction at a time.
//!
//! Each transaction goes through two steps, mirroring `stellar` + a validator:
//!   1. Simulate in recording mode, like `stellar contract invoke --sim`, to
//!      learn the footprint, auth entries and IO sizes.
//!   2. Apply it in enforcing mode on a fresh host with the network's cost
//!      models and the parsed-module cache (CAP-0065), as a validator does.
//!      CPU and memory are read from that run's budget.
//!
//! Ledger changes from step 2 are written back to `Ledger`, so the next
//! transaction sees the new state.

use std::{collections::BTreeMap, rc::Rc};

use soroban_env_host::{
    budget::Budget,
    e2e_invoke::{
        LedgerEntryChange, RecordingInvocationAuthMode, RecordingInvocationAuthParams,
        TtlLedgerEntryMeta, entry_size_for_rent, extract_rent_changes, invoke_host_function,
        invoke_host_function_in_recording_mode,
    },
    fees::{TransactionResources, compute_rent_fee, compute_transaction_resource_fee},
    storage::{EntryWithLiveUntil, SnapshotSource},
    vm::ModuleCache,
    xdr::{
        AccountId, DecoratedSignature, Hash, HostFunction, InvokeHostFunctionOp, LedgerEntry,
        LedgerKey, Limits, Memo, MuxedAccount, Operation, OperationBody,
        Preconditions, ReadXdr, ScVal, SequenceNumber, Signature, SignatureHint,
        SorobanResources, SorobanTransactionData, SorobanTransactionDataExt, Transaction,
        TransactionExt, TransactionV1Envelope, TransactionEnvelope, Uint256, WriteXdr,
        ContractEvent, PublicKey,
    },
    Host, HostError,
};

use crate::network::Network;

/// The ledger: every live entry and its live-until ledger.
#[derive(Default, Clone)]
pub struct Ledger {
    entries: BTreeMap<LedgerKey, (LedgerEntry, Option<u32>)>,
}

impl SnapshotSource for Ledger {
    fn get(&self, key: &Rc<LedgerKey>) -> Result<Option<EntryWithLiveUntil>, HostError> {
        Ok(self
            .entries
            .get(key.as_ref())
            .map(|(e, ttl)| (Rc::new(e.clone()), *ttl)))
    }
}

impl Ledger {
    pub fn iter(&self) -> impl Iterator<Item = (&LedgerKey, &(LedgerEntry, Option<u32>))> {
        self.entries.iter()
    }

    fn apply(&mut self, changes: &[LedgerEntryChange]) {
        for c in changes {
            let key = LedgerKey::from_xdr(&c.encoded_key, Limits::none()).unwrap();
            let new_ttl = c.ttl_change.as_ref().map(|t| t.new_live_until_ledger);
            if c.read_only {
                if let (Some(ttl), Some((_, old))) = (new_ttl, self.entries.get_mut(&key)) {
                    *old = Some(ttl);
                }
            } else if let Some(v) = &c.encoded_new_value {
                let entry = LedgerEntry::from_xdr(v, Limits::none()).unwrap();
                self.entries.insert(key, (entry, new_ttl));
            } else {
                self.entries.remove(&key);
            }
        }
    }
}

/// Everything measured for one transaction.
pub struct TxReport {
    pub result: ScVal,
    pub events: Vec<ContractEvent>,
    pub cpu_insns: u64,
    pub mem_bytes: u64,
    /// Resource fee in stroops (1 XLM = 10^7 stroops), excluding the
    /// inclusion fee bid: non-refundable + refundable parts + rent.
    pub fee_non_refundable: i64,
    pub fee_refundable: i64,
    pub fee_rent: i64,
}

impl TxReport {
    pub fn fee_total(&self) -> i64 {
        self.fee_non_refundable + self.fee_refundable + self.fee_rent
    }
}

pub struct Chain<'a> {
    pub net: &'a Network,
    pub ledger: Ledger,
    cache: ModuleCache,
    cache_host: Host,
    seed: u8,
}

impl<'a> Chain<'a> {
    pub fn new(net: &'a Network) -> Self {
        let cache_host = Host::with_storage_and_budget(
            soroban_env_host::storage::Storage::default(),
            net.unlimited_budget(),
        );
        let cache = ModuleCache::new(&cache_host).unwrap();
        Self { net, ledger: Ledger::default(), cache, cache_host, seed: 0 }
    }

    /// Parse a wasm into the module cache. On the network this happens once
    /// when the upload's ledger closes, not inside any transaction.
    pub fn cache_wasm(&self, wasm: &[u8]) {
        self.cache
            .parse_and_cache_module_simple(&self.cache_host, self.net.protocol, wasm)
            .unwrap();
    }

    /// Simulate, then apply, one transaction. Panics if it fails.
    pub fn run(&mut self, host_fn: HostFunction, source: &AccountId) -> TxReport {
        self.seed = self.seed.wrapping_add(1);
        let seed = [self.seed; 32];

        // 1. Simulate.
        let mut diag = vec![];
        let sim = invoke_host_function_in_recording_mode(
            &self.net.unlimited_budget(),
            false,
            &host_fn,
            source,
            RecordingInvocationAuthMode::Recording(RecordingInvocationAuthParams::new(
                true, false,
            )),
            self.net.ledger.clone(),
            Rc::new(self.ledger.clone()),
            seed,
            &mut diag,
        )
        .expect("[err] simulation failed");
        if let Err(e) = &sim.invoke_result {
            panic!("[err] simulated call failed: {e:?}");
        }
        let resources = sim.resources.clone();

        // 2. Apply, with a fresh budget using the network's cost models.
        let budget: Budget = self.net.budget();
        let footprint = &resources.footprint;
        let keys: Vec<&LedgerKey> = footprint
            .read_only
            .iter()
            .chain(footprint.read_write.iter())
            .collect();
        let entries: Vec<(Option<Vec<u8>>, Option<TtlLedgerEntryMeta>)> = keys
            .iter()
            .map(|k| match self.ledger.entries.get(*k) {
                None => (None, None),
                Some((e, ttl)) => {
                    let xdr = e.to_xdr(Limits::none()).unwrap();
                    let meta = ttl.map(|live_until_ledger| TtlLedgerEntryMeta {
                        live_until_ledger,
                        entry_size_for_rent: entry_size_for_rent(&budget, e, xdr.len() as u32)
                            .unwrap(),
                    });
                    (Some(xdr), meta)
                }
            })
            .collect();
        let auth: Vec<Vec<u8>> = sim.auth.iter().map(|a| a.to_xdr(Limits::none()).unwrap()).collect();

        let res = invoke_host_function(
            &budget,
            false,
            host_fn.to_xdr(Limits::none()).unwrap(),
            resources.to_xdr(Limits::none()).unwrap(),
            &sim.restored_rw_entry_indices,
            source.to_xdr(Limits::none()).unwrap(),
            auth.into_iter(),
            self.net.ledger.clone(),
            entries.into_iter(),
            seed.to_vec(),
            &mut diag,
            None,
            Some(self.cache.clone()),
        )
        .expect("[err] apply failed");
        let encoded = res.encoded_invoke_result.expect("[err] applied call failed");
        let result = ScVal::from_xdr(encoded, Limits::none()).unwrap();
        let cpu_insns = budget.get_cpu_insns_consumed().unwrap();
        let mem_bytes = budget.get_mem_bytes_consumed().unwrap();

        // 3. Price it.
        let tx_size_bytes = tx_size(&host_fn, &sim.auth, &resources, source);
        let read_entries = keys.iter().filter(|k| !is_soroban_key(k)).count() as u32;
        let write_entries = footprint.read_write.len() as u32;
        let (fee_non_refundable, fee_refundable) = compute_transaction_resource_fee(
            &TransactionResources {
                instructions: cpu_insns as u32,
                disk_read_entries: read_entries,
                write_entries,
                disk_read_bytes: resources.disk_read_bytes,
                write_bytes: resources.write_bytes,
                contract_events_size_bytes: sim.contract_events_and_return_value_size,
                transaction_size_bytes: tx_size_bytes,
            },
            &self.net.fees,
        );
        let fee_rent = compute_rent_fee(
            &extract_rent_changes(&res.ledger_changes),
            &self.net.rent,
            self.net.ledger.sequence_number,
        );

        self.ledger.apply(&res.ledger_changes);

        TxReport {
            result,
            events: sim.contract_events,
            cpu_insns,
            mem_bytes,
            fee_non_refundable,
            fee_refundable,
            fee_rent,
        }
    }
}

/// Contract data and code live in memory since protocol 23, so only other
/// entries (accounts, trustlines) count as disk reads.
fn is_soroban_key(k: &LedgerKey) -> bool {
    matches!(k, LedgerKey::ContractData(_) | LedgerKey::ContractCode(_))
}

/// Size of the signed transaction envelope that would carry this call.
fn tx_size(
    host_fn: &HostFunction,
    auth: &[soroban_env_host::xdr::SorobanAuthorizationEntry],
    resources: &SorobanResources,
    source: &AccountId,
) -> u32 {
    let PublicKey::PublicKeyTypeEd25519(Uint256(pk)) = &source.0;
    let tx = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256(*pk)),
        fee: 0,
        seq_num: SequenceNumber(0),
        cond: Preconditions::None,
        memo: Memo::None,
        operations: vec![Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: host_fn.clone(),
                auth: auth.to_vec().try_into().unwrap(),
            }),
        }]
        .try_into()
        .unwrap(),
        ext: TransactionExt::V1(SorobanTransactionData {
            ext: SorobanTransactionDataExt::V0,
            resources: resources.clone(),
            resource_fee: 0,
        }),
    };
    let env = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: vec![DecoratedSignature {
            hint: SignatureHint([0; 4]),
            signature: Signature([0u8; 64].try_into().unwrap()),
        }]
        .try_into()
        .unwrap(),
    });
    env.to_xdr(Limits::none()).unwrap().len() as u32
}

pub fn wasm_hash(wasm: &[u8]) -> Hash {
    Hash(crate::network::sha256(wasm))
}
