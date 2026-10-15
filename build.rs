fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        for file in ["ferrydock.rc", "ferrydock.manifest", "FerryDock.ico"] {
            println!("cargo:rerun-if-changed=packaging/windows/{file}");
        }
        embed_resource::compile_for(
            "packaging/windows/ferrydock.rc",
            ["ferrydock"],
            embed_resource::ParamsIncludeDirs(["packaging/windows"]),
        )
        .manifest_required()
        .expect("could not embed FerryDock's Windows icon and manifest");
    }
}
