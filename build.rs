fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        // UAC manifest — prinstall needs admin to install printer drivers,
        // so the exe requests elevation at launch.
        embed_manifest::embed_manifest(
            embed_manifest::new_manifest("prinstall")
                .requested_execution_level(
                    embed_manifest::manifest::ExecutionLevel::RequireAdministrator,
                ),
        )
        .expect("unable to embed manifest");

        // App icon — embedded as a Windows ICON resource so Explorer, the
        // taskbar, and shortcut UIs render the prinstall icon instead of
        // the generic console exe glyph.
        embed_resource::compile("assets/prinstall.rc", embed_resource::NONE)
            .manifest_required()
            .expect("unable to embed icon resource");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/prinstall.rc");
    println!("cargo:rerun-if-changed=assets/prinstall.ico");
}
