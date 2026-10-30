use std::{env, fs, path::PathBuf, process::Command};

pub fn build() {
    println!("cargo:rerun-if-changed=windows/explorer");
    for name in [
        "MANGODISK_EXPLORER_PUBLISHER",
        "MANGODISK_EXPLORER_SIGN_COMMAND",
        "WindowsSdkDir",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output path"));
    let source = PathBuf::from("windows/explorer");
    let compiler = cc::Build::new().cpp(true).get_compiler();
    assert!(
        compiler.is_like_msvc(),
        "Explorer integration requires MSVC"
    );
    let dll = output.join("MangoDiskExplorer.dll");
    let result = compiler
        .to_command()
        .args([
            "/nologo",
            "/utf-8",
            "/std:c++20",
            "/EHsc",
            "/MT",
            "/LD",
            "/O2",
            "/DUNICODE",
            "/D_UNICODE",
        ])
        .arg(source.join("command.cpp"))
        .arg(format!(
            "/Fo{}",
            output.join("explorer_command.obj").display()
        ))
        .args(["/link", "/Brepro"])
        .arg(format!("/OUT:{}", dll.display()))
        .arg(format!(
            "/IMPLIB:{}",
            output.join("explorer_command.lib").display()
        ))
        .arg(format!("/DEF:{}", source.join("command.def").display()))
        .args([
            "shell32.lib",
            "shlwapi.lib",
            "ole32.lib",
            "advapi32.lib",
            "uuid.lib",
            "user32.lib",
        ])
        .status()
        .expect("run Explorer extension compiler");
    assert!(result.success(), "Explorer extension compilation failed");
    let publisher = env::var("MANGODISK_EXPLORER_PUBLISHER")
        .unwrap_or_else(|_| "CN=MangoDisk Development".into());
    // The signed manifest has a fixed relative DLL name. A content-keyed external
    // directory supplies version isolation without rewriting signed metadata.
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x64",
        _ => panic!("Explorer integration supports Windows x64 and ARM64"),
    };
    let version = format!("{}.0", env::var("CARGO_PKG_VERSION").unwrap());
    let manifest = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
 xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
 xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
 xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
 xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
 xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
 xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5"
 IgnorableNamespaces="uap uap10 rescap com desktop4 desktop5">
 <Identity Name="MangoDisk.ExplorerIntegration" Publisher="{}" Version="{version}" ProcessorArchitecture="{architecture}"/>
 <Properties><DisplayName>MangoDisk</DisplayName><PublisherDisplayName>MangoDisk</PublisherDisplayName><Logo>logo.png</Logo><uap10:AllowExternalContent>true</uap10:AllowExternalContent></Properties>
 <Resources><Resource Language="en-us"/></Resources>
 <Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0"/></Dependencies>
 <Capabilities><rescap:Capability Name="runFullTrust"/><rescap:Capability Name="unvirtualizedResources"/></Capabilities>
 <Applications><Application Id="ExplorerIntegration" Executable="MangoDisk.exe" uap10:RuntimeBehavior="win32App" uap10:TrustLevel="mediumIL">
 <uap:VisualElements AppListEntry="none" DisplayName="MangoDisk" Description="MangoDisk" BackgroundColor="transparent" Square150x150Logo="logo.png" Square44x44Logo="logo.png"/>
 <Extensions>
 <com:Extension Category="windows.comServer"><com:ComServer><com:SurrogateServer DisplayName="MangoDisk"><com:Class Id="93E49F90-CEBD-4BA0-93BA-733A3C732214" Path="MangoDiskExplorer.dll" ThreadingModel="STA"/></com:SurrogateServer></com:ComServer></com:Extension>
 <desktop4:Extension Category="windows.fileExplorerContextMenus"><desktop4:FileExplorerContextMenus>
 <desktop5:ItemType Type="Directory"><desktop5:Verb Id="AnalyzeDirectory" Clsid="93E49F90-CEBD-4BA0-93BA-733A3C732214"/></desktop5:ItemType>
 </desktop4:FileExplorerContextMenus></desktop4:Extension>
 </Extensions></Application></Applications>
</Package>"#,
        xml(&publisher)
    );
    let package_source = output.join("explorer-package");
    fs::create_dir_all(&package_source).unwrap();
    fs::write(package_source.join("AppxManifest.xml"), manifest).unwrap();
    fs::copy("icons/128x128.png", output.join("logo.png")).unwrap();
    let makeappx = sdk_tool("makeappx.exe");
    let package = output.join("explorer_identity.msix");
    let packed = Command::new(makeappx)
        .args(["pack", "/o", "/nv", "/d"])
        .arg(package_source)
        .arg("/p")
        .arg(&package)
        .status()
        .expect("run MakeAppx");
    assert!(packed.success(), "Explorer identity packaging failed");
    if let Ok(command) = env::var("MANGODISK_EXPLORER_SIGN_COMMAND") {
        let arguments: Vec<String> =
            serde_json::from_str(&command).expect("sign command must be a JSON argv array");
        assert!(
            arguments.len() > 1 && arguments[1..].iter().any(|v| v == "{file}"),
            "sign command requires a {{file}} argument"
        );
        for file in [&dll, &package] {
            let result = Command::new(&arguments[0])
                .args(arguments[1..].iter().map(|v| {
                    if v == "{file}" {
                        file.as_os_str().to_owned()
                    } else {
                        v.into()
                    }
                }))
                .status()
                .expect("run Explorer signing command");
            assert!(result.success(), "Explorer signing failed");
        }
    } else {
        assert!(env::var("PROFILE").as_deref() != Ok("release"), "Release Explorer integration requires MANGODISK_EXPLORER_SIGN_COMMAND and a trusted signing certificate");
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

fn sdk_tool(name: &str) -> PathBuf {
    // Developer shells may select an SDK installed outside Program Files.
    // Honor that selection instead of assuming the default installation drive.
    let root = env::var_os("WindowsSdkDir")
        .map(|directory| PathBuf::from(directory).join("bin"))
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK location"))
                .join("Windows Kits/10/bin")
        });
    let mut versions: Vec<_> = fs::read_dir(&root)
        .expect("installed Windows SDK")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("x64").join(name).is_file())
        .collect();
    versions.sort();
    versions
        .pop()
        .expect("Windows SDK MakeAppx is required")
        .join("x64")
        .join(name)
}
