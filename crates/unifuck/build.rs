use std::path::PathBuf;

fn embed_loader_cdylib() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolves from crates/unifuck/..");
    let loader_dll = workspace_root
        .join("target")
        .join("release")
        .join("unifuck_loader.dll");

    if !loader_dll.exists() {
        panic!(
            "missing {}\n\n\
             unifuck embeds unifuck_loader.dll at build time. Build the loader \
             crate first:\n\n    cargo build -p unifuck-loader --release\n\n\
             Then re-run this build.",
            loader_dll.display()
        );
    }

    let dest = out_dir.join("unifuck_loader.dll");
    std::fs::copy(&loader_dll, &dest).expect("copy unifuck_loader.dll into OUT_DIR");
    println!("cargo:rerun-if-changed={}", loader_dll.display());
}

fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app-window.slint", config).unwrap();

    embed_loader_cdylib();

    // Only embed admin manifest for release builds, not tests
    #[cfg(windows)]
    {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};

        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            winresource::WindowsResource::new()
                .set_icon("ui/assets/app-icon.ico")
                .compile()
                .expect("embed windows icon");

            // Only embed manifest for release builds to allow running tests without admin
            let profile = std::env::var("PROFILE").unwrap_or_default();
            if profile == "release" {
                embed_manifest(
                    new_manifest("unifuck")
                        .requested_execution_level(ExecutionLevel::RequireAdministrator),
                )
                .expect("embed manifest");
            }
        }
    }
}
