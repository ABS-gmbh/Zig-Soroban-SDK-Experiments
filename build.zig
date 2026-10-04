const std = @import("std");

// Contracts to build. Each one lives at contracts/<name>/zig/contract.zig.
const contracts = [_][]const u8{ "hello", "increment", "token" };

// Every contract is built in two modes:
//   small: ReleaseSmall, no runtime safety checks (integer overflow wraps).
//   safe:  ReleaseSafe, traps on overflow like Rust's `overflow-checks = true`.
const Variant = struct { name: []const u8, optimize: std.builtin.OptimizeMode };
const variants = [_]Variant{
    .{ .name = "small", .optimize = .ReleaseSmall },
    .{ .name = "safe", .optimize = .ReleaseSafe },
};

pub fn build(b: *std.Build) void {
    const target = b.resolveTargetQuery(.{
        .cpu_arch = .wasm32,
        .os_tag = .freestanding,
        // Same feature set as zig-soroban-sdk: Soroban only accepts
        // bulk_memory, mutable_globals and sign_ext.
        .cpu_features_sub = std.Target.wasm.featureSet(&.{
            .multivalue,
            .extended_const,
            .nontrapping_fptoint,
            .call_indirect_overlong,
        }),
    });

    // The SDK's post-processor turns exported section data into real wasm
    // custom sections (contractspecv0, contractenvmetav0).
    const sdk_src = b.dependency("soroban_sdk", .{});
    const postprocess = b.addExecutable(.{
        .name = "postprocess_wasm",
        .root_module = b.createModule(.{
            .root_source_file = sdk_src.path("tools/postprocess_wasm.zig"),
            .target = b.graph.host,
        }),
    });

    for (variants) |variant| {
        const sdk = b.dependency("soroban_sdk", .{ .optimize = variant.optimize });
        for (contracts) |name| {
            const module = b.createModule(.{
                .root_source_file = b.path(b.fmt("contracts/{s}/zig/contract.zig", .{name})),
                .target = target,
                .optimize = variant.optimize,
                // Like Rust's `strip = "symbols"`: no debug info in the wasm.
                .strip = true,
            });
            module.addImport("soroban-sdk", sdk.module("soroban-sdk"));

            const exe = b.addExecutable(.{ .name = name, .root_module = module });
            exe.entry = .disabled;
            exe.rdynamic = true;

            const run = b.addRunArtifact(postprocess);
            run.addArtifactArg(exe);
            const wasm = run.addOutputFileArg(b.fmt("{s}.wasm", .{name}));
            const install = b.addInstallFileWithDir(
                wasm,
                .{ .custom = b.fmt("zig-{s}", .{variant.name}) },
                b.fmt("{s}.wasm", .{name}),
            );
            b.getInstallStep().dependOn(&install.step);
        }
    }
}
