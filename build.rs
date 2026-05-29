fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("ui/app-window.slint", config).unwrap();

    #[cfg(windows)]
    {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};

        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            embed_manifest(
                new_manifest("unifuck")
                    .requested_execution_level(ExecutionLevel::RequireAdministrator),
            )
            .expect("embed manifest");
        }
    }
}
