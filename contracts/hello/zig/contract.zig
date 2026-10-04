// Zig port of stellar/soroban-examples/hello_world.

const std = @import("std");
const sdk = @import("soroban-sdk");

// Trap on panic (including integer overflow in ReleaseSafe), like Rust's
// `panic = "abort"`.
pub const panic = std.debug.no_panic;

const HelloContract = struct {
    pub const hello_params = [_][]const u8{"to"};
    pub fn hello(to: sdk.String) sdk.Vec {
        return sdk.Vec.from(.{ sdk.String.fromSlice("Hello"), to });
    }
};

comptime {
    _ = sdk.contract.exportContract(HelloContract);
}
