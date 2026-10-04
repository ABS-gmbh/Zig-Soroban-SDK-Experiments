// Zig port of stellar/soroban-examples/increment.
//
// The Rust version also calls `log!`, which is compiled out of release
// builds, so it has no equivalent here.

const std = @import("std");
const sdk = @import("soroban-sdk");

pub const panic = std.debug.no_panic;

const COUNTER = sdk.Symbol.fromString("COUNTER");

const IncrementContract = struct {
    /// Increment the counter in instance storage and return the new value.
    pub fn increment() sdk.U32Val {
        var count: u32 = sdk.ledger.getU32(COUNTER, sdk.StorageType.instance) orelse 0;
        count += 1;
        sdk.ledger.putU32(COUNTER, count, sdk.StorageType.instance);
        sdk.ledger.extendCurrentContractInstanceAndCodeTtl(50, 100);
        return sdk.U32Val.fromU32(count);
    }
};

comptime {
    _ = sdk.contract.exportContract(IncrementContract);
}
