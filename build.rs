fn main() {
    println!("cargo:rerun-if-changed=assets/brightmarmot.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/brightmarmot.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
