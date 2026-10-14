fn main() {
    // Embeds the icon, version info ("Puente Digital") and DPI manifest into the .exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
