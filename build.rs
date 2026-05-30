fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/app-window.slint", config).unwrap();

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
