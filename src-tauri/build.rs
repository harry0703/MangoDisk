fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_taskbar_bridge();
    }
    // Tauri embeds native icons into the compiled application, but icon-only changes do not
    // automatically invalidate the generated context in every development workflow. Watching the
    // directory ensures `tauri dev` rebuilds the executable after a Dock or taskbar icon update.
    println!("cargo:rerun-if-changed=icons");

    // Keep the Windows GUI in the interactive user's security context. If a standard user
    // approves UAC with a different administrator account, WebView2 runs as the interactive
    // user while the elevated host resolves its data directory from the administrator's
    // LocalAppData. That ACL mismatch prevents startup. The manifest therefore uses `asInvoker`;
    // operations requiring administrator rights must elevate only at their platform capability
    // boundary.
    let windows =
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest"));
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);

    tauri_build::try_build(attributes).expect("failed to run the Tauri build script");
}

/// Embed the architecture-matched XAML adapter in the executable. No deployment
/// script or machine-installed component is required, including portable builds.
fn build_taskbar_bridge() {
    let source = std::path::PathBuf::from("windows/taskbar");
    println!("cargo:rerun-if-changed={}", source.display());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output path"));
    let compiler = cc::Build::new().cpp(true).get_compiler();
    assert!(
        compiler.is_like_msvc(),
        "Windows taskbar requires the MSVC SDK"
    );
    let result = compiler
        .to_command()
        // Current C++/WinRT headers use standard coroutines under C++20. C++17
        // selects the deprecated experimental header rejected by newer MSVC.
        .args(["/nologo", "/std:c++20", "/EHsc", "/MT", "/LD", "/O2"])
        .arg(source.join("xaml_bridge.cpp"))
        .arg(format!("/Fo{}", output.join("xaml_bridge.obj").display()))
        .arg("/link")
        .arg("/Brepro")
        .arg(format!(
            "/OUT:{}",
            output.join("taskbar_xaml.dll").display()
        ))
        .arg(format!(
            "/IMPLIB:{}",
            output.join("taskbar_xaml.lib").display()
        ))
        .arg(format!("/DEF:{}", source.join("xaml_bridge.def").display()))
        .args([
            "windowsapp.lib",
            "runtimeobject.lib",
            "user32.lib",
            "ole32.lib",
        ])
        .status()
        .expect("run Windows C++ compiler");
    assert!(
        result.success(),
        "Windows taskbar bridge compilation failed"
    );
}
