//! Na macOS wszywa Info.plist w sam plik wykonywalny (sekcja __TEXT,__info_plist).
//! Dzięki temu także program uruchomiony bez paczki .app (np. z Terminala) korzysta
//! ze zintegrowanej karty graficznej zamiast włączać dedykowaną, która grzeje komputer.

fn main() {
    println!("cargo:rerun-if-changed=packaging/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let plist = std::fs::read_to_string("packaging/Info.plist")
        .expect("brak packaging/Info.plist")
        .replace("VERSION", &version);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("Info.plist");
    std::fs::write(&out, plist).expect("zapis Info.plist");
    println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", out.display());
}
