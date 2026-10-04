//! Mainnet network settings, loaded from `config/mainnet-settings.xdr`.
//!
//! The file is the `ConfigUpgradeSet` printed by
//! `stellar network settings --internal --output xdr` against a mainnet RPC.
//! Using the real settings means CPU and memory are metered with the
//! network's cost models and fees are priced at the network's rates.

use soroban_env_host::{
    budget::Budget,
    fees::{
        FeeConfiguration, RentFeeConfiguration, RentWriteFeeConfiguration,
        compute_rent_write_fee_per_1kb,
    },
    xdr::{ConfigSettingEntry, ConfigUpgradeSet, ContractCostParams, Limits, ReadXdr},
    LedgerInfo,
};

const SETTINGS_XDR: &str = include_str!("../config/mainnet-settings.xdr");

pub const PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";

/// Ledger sequence the scenarios run at. Only matters for TTL arithmetic.
pub const LEDGER_SEQ: u32 = 1_000_000;

pub struct Network {
    pub protocol: u32,
    pub ledger: LedgerInfo,
    pub fees: FeeConfiguration,
    pub rent: RentFeeConfiguration,
    cpu_params: ContractCostParams,
    mem_params: ContractCostParams,
    tx_max_instructions: u64,
    tx_memory_limit: u64,
}

impl Network {
    pub fn mainnet(protocol: u32) -> Self {
        let set = ConfigUpgradeSet::from_xdr_base64(SETTINGS_XDR.trim(), Limits::none())
            .expect("[err] cannot decode config/mainnet-settings.xdr");

        let mut fees = FeeConfiguration::default();
        let mut rent = RentFeeConfiguration::default();
        let mut rent_write = RentWriteFeeConfiguration::default();
        let mut state_size_window: Vec<u64> = vec![];
        let (mut cpu_params, mut mem_params) = (None, None);
        let (mut tx_max_instructions, mut tx_memory_limit) = (0, 0);
        let (mut min_temp, mut min_persistent, mut max_ttl) = (0, 0, 0);

        for entry in set.updated_entry.iter() {
            match entry {
                ConfigSettingEntry::ContractComputeV0(c) => {
                    fees.fee_per_instruction_increment = c.fee_rate_per_instructions_increment;
                    tx_max_instructions = c.tx_max_instructions as u64;
                    tx_memory_limit = c.tx_memory_limit as u64;
                }
                ConfigSettingEntry::ContractLedgerCostV0(c) => {
                    fees.fee_per_disk_read_entry = c.fee_disk_read_ledger_entry;
                    fees.fee_per_write_entry = c.fee_write_ledger_entry;
                    fees.fee_per_disk_read_1kb = c.fee_disk_read1_kb;
                    rent.fee_per_write_entry = c.fee_write_ledger_entry;
                    rent_write = RentWriteFeeConfiguration {
                        state_target_size_bytes: c.soroban_state_target_size_bytes,
                        rent_fee_1kb_state_size_low: c.rent_fee1_kb_soroban_state_size_low,
                        rent_fee_1kb_state_size_high: c.rent_fee1_kb_soroban_state_size_high,
                        state_size_rent_fee_growth_factor: c.soroban_state_rent_fee_growth_factor,
                    };
                }
                ConfigSettingEntry::ContractLedgerCostExtV0(c) => {
                    fees.fee_per_write_1kb = c.fee_write1_kb;
                    rent.fee_per_write_1kb = c.fee_write1_kb;
                }
                ConfigSettingEntry::ContractHistoricalDataV0(c) => {
                    fees.fee_per_historical_1kb = c.fee_historical1_kb;
                }
                ConfigSettingEntry::ContractEventsV0(c) => {
                    fees.fee_per_contract_event_1kb = c.fee_contract_events1_kb;
                }
                ConfigSettingEntry::ContractBandwidthV0(c) => {
                    fees.fee_per_transaction_size_1kb = c.fee_tx_size1_kb;
                }
                ConfigSettingEntry::StateArchival(c) => {
                    rent.persistent_rent_rate_denominator = c.persistent_rent_rate_denominator;
                    rent.temporary_rent_rate_denominator = c.temp_rent_rate_denominator;
                    min_temp = c.min_temporary_ttl;
                    min_persistent = c.min_persistent_ttl;
                    max_ttl = c.max_entry_ttl;
                }
                ConfigSettingEntry::LiveSorobanStateSizeWindow(w) => {
                    state_size_window = w.iter().copied().collect();
                }
                ConfigSettingEntry::ContractCostParamsCpuInstructions(p) => {
                    cpu_params = Some(p.clone())
                }
                ConfigSettingEntry::ContractCostParamsMemoryBytes(p) => {
                    mem_params = Some(p.clone())
                }
                _ => {}
            }
        }

        // The rent rate depends on the average Soroban state size, which the
        // network tracks over a sliding window.
        let avg_state_size =
            state_size_window.iter().sum::<u64>() / state_size_window.len().max(1) as u64;
        rent.fee_per_rent_1kb = compute_rent_write_fee_per_1kb(avg_state_size as i64, &rent_write);

        let ledger = LedgerInfo {
            protocol_version: protocol,
            sequence_number: LEDGER_SEQ,
            timestamp: 1_790_000_000,
            network_id: sha256(PASSPHRASE.as_bytes()),
            base_reserve: 5_000_000,
            min_temp_entry_ttl: min_temp,
            min_persistent_entry_ttl: min_persistent,
            max_entry_ttl: max_ttl,
        };

        Self {
            protocol,
            ledger,
            fees,
            rent,
            cpu_params: cpu_params.expect("[err] settings lack CPU cost params"),
            mem_params: mem_params.expect("[err] settings lack memory cost params"),
            tx_max_instructions,
            tx_memory_limit,
        }
    }

    /// A fresh budget with the network's per-transaction limits and cost models.
    pub fn budget(&self) -> Budget {
        Budget::try_from_configs(
            self.tx_max_instructions,
            self.tx_memory_limit,
            self.cpu_params.clone(),
            self.mem_params.clone(),
        )
        .expect("[err] cannot build budget from network settings")
    }

    /// A budget with no limits, for setup work that is not being measured.
    pub fn unlimited_budget(&self) -> Budget {
        let b = self.budget();
        b.reset_unlimited().unwrap();
        b
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}
