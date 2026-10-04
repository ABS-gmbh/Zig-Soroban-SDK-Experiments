// Zig port of stellar/soroban-examples/token.
//
// Matches the Rust contract's observable behavior so their costs can be
// compared: the same storage keys and value layouts, TTL extensions,
// events, and checks. Where the Rust version panics, this one traps.
//
// Storage layout (as produced by Rust's #[contracttype]):
//   ["Admin"]                                -> Address        (instance)
//   "METADATA"                               -> {decimal, name, symbol} (instance)
//   ["Balance", addr]                        -> i128           (persistent)
//   ["Allowance", {from, spender}]           -> {amount, live_until_ledger} (temporary)
//
// The SDK has no struct support yet, so struct values are built and read
// with the map_*_linear_memory host functions directly, the same calls
// the Rust SDK makes for a #[contracttype] struct.

const std = @import("std");
const sdk = @import("soroban-sdk");

pub const panic = std.debug.no_panic;

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_LIFETIME_THRESHOLD: u32 = INSTANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;
const BALANCE_BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const BALANCE_LIFETIME_THRESHOLD: u32 = BALANCE_BUMP_AMOUNT - DAY_IN_LEDGERS;

const ADMIN = sdk.Symbol.fromString("Admin");
const BALANCE = sdk.Symbol.fromString("Balance");
const ALLOWANCE = sdk.Symbol.fromString("Allowance");
const METADATA = sdk.Symbol.fromString("METADATA");

const METADATA_FIELDS = [_][]const u8{ "decimal", "name", "symbol" };
const ALLOWANCE_KEY_FIELDS = [_][]const u8{ "from", "spender" };
const ALLOWANCE_VALUE_FIELDS = [_][]const u8{ "amount", "live_until_ledger" };

const TokenContract = struct {
    pub const @"__constructor_params" = [_][]const u8{ "admin", "decimal", "name", "symbol" };
    pub fn @"__constructor"(admin: sdk.Address, decimal: sdk.U32Val, name_: sdk.String, symbol_: sdk.String) sdk.Void {
        if (decimal.toU32() > 18) @trap();
        writeAdministrator(admin);
        const metadata = mapNew(&METADATA_FIELDS, .{ decimal.toVal(), name_.toVal(), symbol_.toVal() });
        sdk.ledger.putContractData(METADATA.toVal(), metadata.toVal(), sdk.StorageType.instance);
        return sdk.Void.VOID;
    }

    pub const mint_params = [_][]const u8{ "to", "amount" };
    pub fn mint(to: sdk.Address, amount: sdk.I128Val) sdk.Void {
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        readAdministrator().requireAuth();
        extendInstance();
        receiveBalance(to, amt);
        sdk.emit(.{ sdk.Symbol.fromString("mint"), to }, amount);
        return sdk.Void.VOID;
    }

    pub const set_admin_params = [_][]const u8{"new_admin"};
    pub fn set_admin(new_admin: sdk.Address) sdk.Void {
        const admin = readAdministrator();
        admin.requireAuth();
        extendInstance();
        writeAdministrator(new_admin);
        sdk.emit(.{ sdk.Symbol.fromString("set_admin"), admin }, new_admin);
        return sdk.Void.VOID;
    }

    pub const allowance_params = [_][]const u8{ "from", "spender" };
    pub fn allowance(from: sdk.Address, spender: sdk.Address) sdk.I128Val {
        extendInstance();
        return sdk.int.i128ToVal(readAllowance(from, spender).amount);
    }

    pub const approve_params = [_][]const u8{ "from", "spender", "amount", "live_until_ledger" };
    pub fn approve(from: sdk.Address, spender: sdk.Address, amount: sdk.I128Val, live_until_ledger: sdk.U32Val) sdk.Void {
        from.requireAuth();
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        extendInstance();
        writeAllowance(from, spender, amt, live_until_ledger.toU32());
        sdk.emit(
            .{ sdk.Symbol.fromString("approve"), from, spender },
            sdk.Vec.from(.{ amount, live_until_ledger }),
        );
        return sdk.Void.VOID;
    }

    pub const balance_params = [_][]const u8{"id"};
    pub fn balance(id: sdk.Address) sdk.I128Val {
        extendInstance();
        return sdk.int.i128ToVal(readBalance(id));
    }

    // `to_muxed` is a MuxedAddress in Rust: either a plain Address or a
    // MuxedAddressObject carrying an extra u64 id.
    pub const transfer_params = [_][]const u8{ "from", "to_muxed", "amount" };
    pub fn transfer(from: sdk.Address, to_muxed: sdk.Val, amount: sdk.I128Val) sdk.Void {
        from.requireAuth();
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        extendInstance();
        spendBalance(from, amt);

        var to = sdk.Address.fromVal(to_muxed);
        var to_muxed_id: ?sdk.U64Val = null;
        if (to_muxed.getTag() == sdk.Tag.muxed_address_object) {
            const muxed = sdk.MuxedAddressObject{ .payload = to_muxed.payload };
            to = sdk.address.getAddressFromMuxedAddress(muxed);
            to_muxed_id = sdk.address.getIdFromMuxedAddress(muxed);
        }
        receiveBalance(to, amt);
        emitTransfer(from, to, to_muxed_id, amount);
        return sdk.Void.VOID;
    }

    pub const transfer_from_params = [_][]const u8{ "spender", "from", "to", "amount" };
    pub fn transfer_from(spender: sdk.Address, from: sdk.Address, to: sdk.Address, amount: sdk.I128Val) sdk.Void {
        spender.requireAuth();
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        extendInstance();
        spendAllowance(from, spender, amt);
        spendBalance(from, amt);
        receiveBalance(to, amt);
        emitTransfer(from, to, null, amount);
        return sdk.Void.VOID;
    }

    pub const burn_params = [_][]const u8{ "from", "amount" };
    pub fn burn(from: sdk.Address, amount: sdk.I128Val) sdk.Void {
        from.requireAuth();
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        extendInstance();
        spendBalance(from, amt);
        sdk.emit(.{ sdk.Symbol.fromString("burn"), from }, amount);
        return sdk.Void.VOID;
    }

    pub const burn_from_params = [_][]const u8{ "spender", "from", "amount" };
    pub fn burn_from(spender: sdk.Address, from: sdk.Address, amount: sdk.I128Val) sdk.Void {
        spender.requireAuth();
        const amt = sdk.int.i128FromVal(amount);
        checkNonnegativeAmount(amt);
        extendInstance();
        spendAllowance(from, spender, amt);
        spendBalance(from, amt);
        sdk.emit(.{ sdk.Symbol.fromString("burn"), from }, amount);
        return sdk.Void.VOID;
    }

    pub fn decimals() sdk.U32Val {
        return sdk.U32Val.fromVal(.{ .payload = readMetadata()[0] });
    }

    pub fn name() sdk.String {
        return sdk.String.fromVal(.{ .payload = readMetadata()[1] });
    }

    pub fn symbol() sdk.String {
        return sdk.String.fromVal(.{ .payload = readMetadata()[2] });
    }
};

comptime {
    _ = sdk.contract.exportContract(TokenContract);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn checkNonnegativeAmount(amount: i128) void {
    if (amount < 0) @trap();
}

fn extendInstance() void {
    sdk.ledger.extendCurrentContractInstanceAndCodeTtl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn readAdministrator() sdk.Address {
    const key = sdk.Vec.from(.{ADMIN}).toVal();
    const v = sdk.ledger.getVal(key, sdk.StorageType.instance) orelse @trap();
    return sdk.Address.fromVal(v);
}

fn writeAdministrator(admin: sdk.Address) void {
    const key = sdk.Vec.from(.{ADMIN}).toVal();
    sdk.ledger.putContractData(key, admin.toVal(), sdk.StorageType.instance);
}

fn readMetadata() [METADATA_FIELDS.len]u64 {
    const v = sdk.ledger.getVal(METADATA, sdk.StorageType.instance) orelse @trap();
    return mapUnpack(&METADATA_FIELDS, sdk.Map.fromVal(v));
}

// -- Balances --

fn balanceKey(addr: sdk.Address) sdk.Val {
    return sdk.Vec.from(.{ BALANCE, addr }).toVal();
}

fn readBalance(addr: sdk.Address) i128 {
    const key = balanceKey(addr);
    const v = sdk.ledger.getVal(key, sdk.StorageType.persistent) orelse return 0;
    sdk.ledger.extendContractDataTtl(key, sdk.StorageType.persistent, BALANCE_LIFETIME_THRESHOLD, BALANCE_BUMP_AMOUNT);
    return sdk.int.i128FromVal(sdk.I128Val.fromVal(v));
}

fn writeBalance(addr: sdk.Address, amount: i128) void {
    const key = balanceKey(addr);
    sdk.ledger.putContractData(key, sdk.int.i128ToVal(amount).toVal(), sdk.StorageType.persistent);
    sdk.ledger.extendContractDataTtl(key, sdk.StorageType.persistent, BALANCE_LIFETIME_THRESHOLD, BALANCE_BUMP_AMOUNT);
}

fn receiveBalance(addr: sdk.Address, amount: i128) void {
    writeBalance(addr, readBalance(addr) + amount);
}

fn spendBalance(addr: sdk.Address, amount: i128) void {
    const bal = readBalance(addr);
    if (bal < amount) @trap();
    writeBalance(addr, bal - amount);
}

// -- Allowances --

const AllowanceValue = struct { amount: i128, live_until_ledger: u32 };

fn allowanceKey(from: sdk.Address, spender: sdk.Address) sdk.Val {
    const inner = mapNew(&ALLOWANCE_KEY_FIELDS, .{ from.toVal(), spender.toVal() });
    return sdk.Vec.from(.{ ALLOWANCE, inner }).toVal();
}

fn readAllowance(from: sdk.Address, spender: sdk.Address) AllowanceValue {
    const key = allowanceKey(from, spender);
    const v = sdk.ledger.getVal(key, sdk.StorageType.temporary) orelse
        return .{ .amount = 0, .live_until_ledger = 0 };
    const fields = mapUnpack(&ALLOWANCE_VALUE_FIELDS, sdk.Map.fromVal(v));
    const live_until_ledger = sdk.U32Val.fromVal(.{ .payload = fields[1] }).toU32();
    if (live_until_ledger < sdk.ledger.getLedgerSequence()) {
        return .{ .amount = 0, .live_until_ledger = live_until_ledger };
    }
    return .{
        .amount = sdk.int.i128FromVal(sdk.I128Val.fromVal(.{ .payload = fields[0] })),
        .live_until_ledger = live_until_ledger,
    };
}

fn writeAllowance(from: sdk.Address, spender: sdk.Address, amount: i128, live_until_ledger: u32) void {
    const seq = sdk.ledger.getLedgerSequence();
    if (amount > 0 and live_until_ledger < seq) @trap();

    const key = allowanceKey(from, spender);
    const value = mapNew(&ALLOWANCE_VALUE_FIELDS, .{
        sdk.int.i128ToVal(amount).toVal(),
        sdk.U32Val.fromU32(live_until_ledger).toVal(),
    });
    sdk.ledger.putContractData(key, value.toVal(), sdk.StorageType.temporary);

    if (amount > 0) {
        const live_for = std.math.sub(u32, live_until_ledger, seq) catch @trap();
        sdk.ledger.extendContractDataTtl(key, sdk.StorageType.temporary, live_for, live_for);
    }
}

fn spendAllowance(from: sdk.Address, spender: sdk.Address, amount: i128) void {
    const a = readAllowance(from, spender);
    if (a.amount < amount) @trap();
    if (amount > 0) writeAllowance(from, spender, a.amount - amount, a.live_until_ledger);
}

// -- Events --

// Rust's `Transfer` event: data is a map that leaves out `to_muxed_id`
// when it is None.
fn emitTransfer(from: sdk.Address, to: sdk.Address, to_muxed_id: ?sdk.U64Val, amount: sdk.I128Val) void {
    const topics = .{ sdk.Symbol.fromString("transfer"), from, to };
    if (to_muxed_id) |id| {
        sdk.emit(topics, mapNew(&[_][]const u8{ "amount", "to_muxed_id" }, .{ amount.toVal(), id.toVal() }));
    } else {
        sdk.emit(topics, mapNew(&[_][]const u8{"amount"}, .{amount.toVal()}));
    }
}

// -- Struct <-> Map via linear memory --

const SliceRef = extern struct { ptr: u32, len: u32 };

fn u32v(x: usize) sdk.U32Val {
    return sdk.U32Val.fromU32(@intCast(x));
}

fn keyRefs(comptime keys: []const []const u8) [keys.len]SliceRef {
    var refs: [keys.len]SliceRef = undefined;
    inline for (keys, 0..) |k, i| {
        refs[i] = .{ .ptr = @intCast(@intFromPtr(k.ptr)), .len = k.len };
    }
    return refs;
}

/// Build a map from field names (sorted, as Rust requires) and values.
fn mapNew(comptime keys: []const []const u8, vals: [keys.len]sdk.Val) sdk.Map {
    const refs = keyRefs(keys);
    var payloads: [keys.len]u64 = undefined;
    for (vals, 0..) |v, i| payloads[i] = v.payload;
    return sdk.env.map.map_new_from_linear_memory(u32v(@intFromPtr(&refs)), u32v(@intFromPtr(&payloads)), u32v(keys.len));
}

/// Read a map's values for the given field names, in that order.
fn mapUnpack(comptime keys: []const []const u8, m: sdk.Map) [keys.len]u64 {
    const refs = keyRefs(keys);
    var payloads: [keys.len]u64 = undefined;
    _ = sdk.env.map.map_unpack_to_linear_memory(m, u32v(@intFromPtr(&refs)), u32v(@intFromPtr(&payloads)), u32v(keys.len));
    return payloads;
}
