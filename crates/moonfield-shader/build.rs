use std::env;
use std::path::PathBuf;

fn main() {
    // Rerun if wrapper or headers change
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=slang/include/slang.h");
    println!("cargo:rerun-if-changed=slang/include/slang-gfx.h");
    println!("cargo:rerun-if-changed=slang/include/slang-com-helper.h");

    // Get the directory where the Slang library is located
    let slang_dir = PathBuf::from("slang");
    let include_dir = slang_dir.join("include");

    // Tell cargo to look for shared libraries in the build directories
    let lib_paths = [
        slang_dir.join("build/Release/lib"),
        slang_dir.join("build/Debug/lib"),
    ];

    let mut lib_found = false;
    for path in &lib_paths {
        if path.exists() {
            println!("cargo:rustc-link-search=native={}", path.display());
            lib_found = true;
        }
    }

    // Only link if the library exists
    if lib_found {
        // Tell cargo to tell rustc to link the slang library
        println!("cargo:rustc-link-lib=dylib=slang");
    } else {
        println!(
            "cargo:warning=Slang library not found in expected locations. Build may fail at link time."
        );
        println!(
            "cargo:warning=To fix this, build Slang first: cd slang && cmake --preset default && cmake --build --preset release"
        );
    }

    // Platform-specific settings
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Configure platform-specific calling conventions
    let is_windows = target_os == "windows";
    let is_msvc = target_env == "msvc";

    if is_windows {
        // On Windows, Slang uses stdcall calling convention for COM interfaces
        println!("cargo:rustc-cfg=slang_stdcall");
    }

    // Generate bindings using bindgen
    let mut builder = bindgen::Builder::default()
        // The wrapper header that includes slang.h
        .header("wrapper.h")
        // Add include paths for clang to find headers
        .clang_arg(format!("-I{}", slang_dir.display()))
        .clang_arg(format!("-I{}", include_dir.display()))
        // Tell clang this is C++ code (slang.h is a C++ header)
        .clang_arg("-x")
        .clang_arg("c++")
        // Use C++17 standard
        .clang_arg("-std=c++17")
        // Disable exceptions for cleaner bindings
        .clang_arg("-fno-exceptions")
        // Define platform macros for the preprocessor
        .clang_arg(format!(
            "-DSLANG_PTR_IS_64={}",
            if target_arch == "x86_64" || target_arch == "aarch64" {
                1
            } else {
                0
            }
        ))
        // Allowlist patterns for Slang types and functions
        // Core types
        .allowlist_type("Slang.*")
        // Interface types (COM-style)
        .allowlist_type("ISlang.*")
        // Global functions
        .allowlist_function("slang.*")
        // Constants and macros
        .allowlist_var("SLANG_.*")
        .allowlist_var("kIROp.*")
        // Blocklist problematic types that bindgen struggles with
        .blocklist_type("std::.*")
        .blocklist_type("__gnu_cxx::.*")
        .blocklist_type("__std_.*")
        // Use core instead of std for FFI types
        .use_core()
        // Layout tests can cause issues with some types
        .layout_tests(false)
        // Generate documentation comments
        .generate_comments(true)
        // Keep C++ namespaces
        .enable_cxx_namespaces()
        // Derive common traits (but not eq/hash/ord for function pointer types)
        .derive_default(true)
        // Note: Hash and Ord are not derived because some Slang structs contain
        // function pointers which don't have meaningful hash/ord implementations
        // Parse callbacks for cargo integration
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    // Platform-specific bindgen configuration
    if is_windows && is_msvc {
        // On Windows MSVC, we need to handle __stdcall properly
        builder = builder.clang_arg("-D_MSC_VER=1930").clang_arg("-D_WIN64");
    } else if is_windows {
        // Windows with GNU toolchain
        builder = builder.clang_arg("-D__MINGW32__").clang_arg("-D_WIN64");
    } else {
        // Unix-like platforms
        builder = builder.clang_arg("-D__linux__").clang_arg("-DSLANG_LINUX=1");
    }

    // Generate the bindings
    let bindings = builder.generate().expect(
        "Unable to generate bindings - ensure Slang headers are present",
    );

    // Write the bindings to the $OUT_DIR/bindings.rs file
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Emit additional configuration for downstream crates
    println!("cargo:rustc-cfg=slang_bindings_generated");
}
