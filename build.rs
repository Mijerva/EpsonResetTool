//! Embeds the application manifest: common controls 6 for the current
//! Windows look, and no elevation request.
fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && target == "msvc" {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='asInvoker' uiAccess='false'");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
}
