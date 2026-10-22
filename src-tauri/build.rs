fn main() {
    // Native C++ wrappers catch allocation failures before returning across Rust FFI.
    // Enable standard exception unwinding for all MSVC C++ translation units.
    println!("cargo:rerun-if-changed=native-genesis");
    println!("cargo:rerun-if-changed=native-fx");
    println!("cargo:rerun-if-changed=native-noise");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_noise.h");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_fx.h");
    println!("cargo:rerun-if-changed=src/native_genesis_core.cpp");
    println!("cargo:rerun-if-changed=native-megacd");
    println!("cargo:rerun-if-changed=src/native_genesis_rf5c164.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_rf5c164.h");
    println!("cargo:rerun-if-changed=native-livefx");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_livefx.h");
    println!("cargo:rerun-if-changed=native-ym2608");
    println!("cargo:rerun-if-changed=src/native_genesis_ym2608.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_ym2608.h");
    // GNU ld scans archives in order; the PC-98 wrapper references the shared
    // ymfm provider below, which must be linked after this wrapper.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-genesis")
        .file("src/native_genesis_ym2608.cpp")
        .warnings(false)
        .compile("tetorica_native_ym2608");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-genesis")
        .file("src/native_genesis_core.cpp")
        .file("src/native_genesis_fx.cpp")
        .file("src/native_genesis_noise.cpp")
        .file("native-genesis/ym2612_wasm.cpp")
        .file("native-genesis/segapsg_wasm.cpp")
        .file("native-genesis/ymfm_opn.cpp")
        .file("native-genesis/ymfm_adpcm.cpp")
        .file("native-genesis/ymfm_ssg.cpp")
        .file("native-genesis/segapsg.cpp")
        .warnings(false)
        .compile("tetorica_native_genesis");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-megacd")
        .file("src/native_genesis_rf5c164.cpp")
        .file("native-megacd/rf5c164.cpp")
        .file("native-megacd/rf5c164_wasm.cpp")
        .warnings(false)
        .compile("tetorica_native_megacd");
    println!("cargo:rerun-if-changed=native-dx7");
    println!("cargo:rerun-if-changed=src/native_genesis_dx7.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_dx7.h");
    let mut dx7 = cc::Build::new();
    dx7.cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .define("MSFA_FORCE_SCALAR", None)
        // MSVC exposes M_PI in math.h only when this is defined first.
        .define("_USE_MATH_DEFINES", None)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("native-dx7")
        .file("src/native_genesis_dx7.cpp");
    for source in [
        "dx7note",
        "env",
        "exp2",
        "fm_core",
        "fm_op_kernel",
        "freqlut",
        "lfo",
        "patch",
        "pitchenv",
        "resofilter",
        "ringbuffer",
        "sin",
        "synth_unit",
    ] {
        dx7.file(format!("native-dx7/{source}.cc"));
    }
    dx7.warnings(false).compile("tetorica_native_dx7");
    println!("cargo:rerun-if-changed=native-nes");
    println!("cargo:rerun-if-changed=src/native_genesis_nes.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_nes.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .file("src/native_genesis_nes.cpp")
        .warnings(false)
        .compile("tetorica_native_nes");
    for name in ["gameboy", "snes", "huc6280"] {
        println!("cargo:rerun-if-changed=native-{name}");
        println!("cargo:rerun-if-changed=src/native_genesis_{name}.cpp");
        println!("cargo:rerun-if-changed=src/native_genesis_{name}.h");
    }
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("native-gameboy/snapshot/third_party/mame-gameboy")
        .file("src/native_genesis_gameboy.cpp")
        .file("native-gameboy/snapshot/third_party/mame-gameboy/gameboy_apu.cpp")
        .warnings(false)
        .compile("tetorica_native_gameboy");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .file("src/native_genesis_huc6280.cpp")
        .warnings(false)
        .compile("tetorica_native_huc6280");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-fwrapv")
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-ffp-contract=off")
        .include("native-snes/snapshot/third_party/snes-spc/snes_spc")
        .file("src/native_genesis_snes.cpp")
        .file("native-snes/snapshot/third_party/snes-spc/snes_spc/SPC_DSP.cpp")
        .warnings(false)
        .compile("tetorica_native_snes");
    println!("cargo:rerun-if-changed=native-pwm");
    println!("cargo:rerun-if-changed=src/native_genesis_pwm.cpp");
    println!("cargo:rerun-if-changed=src/native_genesis_pwm.h");
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++17")
        .opt_level(2)
        .flag_if_supported("-ffp-contract=off")
        .include("native-pwm/snapshot/wasm")
        .file("src/native_genesis_pwm.cpp")
        .warnings(false)
        .compile("tetorica_native_pwm");
    // LiveFX executes only on its generation/probe owner thread, never the
    // output callback. The host creates a JSRuntime only on first register.
    // GNU ld scans static archives in order: the wrapper must precede QuickJS.
    cc::Build::new()
        .cpp(true)
        .flag_if_supported("/EHsc")
        .std("c++14")
        .opt_level(2)
        .include("native-livefx/quickjs-ng")
        .file("src/native_genesis_livefx.cpp")
        .warnings(false)
        .compile("tetorica_native_livefx_probe");
    cc::Build::new()
        .std("c11")
        // Match QuickJS-NG's MSVC build: C11 alone does not enable <stdatomic.h>.
        .flag_if_supported("/experimental:c11atomics")
        .opt_level(2)
        .define("QUICKJS_NG_BUILD", None)
        // Match QuickJS-NG's build and expose clock_gettime/tm_gmtoff on glibc.
        .define("_GNU_SOURCE", None)
        .include("native-livefx/quickjs-ng")
        .file("native-livefx/quickjs-ng/quickjs-amalgam.c")
        .warnings(false)
        .compile("tetorica_quickjs_ng");
    tauri_build::build()
}
