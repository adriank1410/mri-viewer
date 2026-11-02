// W wersji wydawniczej na Windows nie otwieramy okna konsoli.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod cache;
mod gui;
mod imaging;
mod library;
mod mcp;
mod mpr;

use std::path::PathBuf;

/// Pomoc wiersza poleceń w bieżącym języku.
fn usage() -> String {
    let lines = [
        ("mri-viewer [FOLDER]", t!("cli.usage.gui")),
        ("mri-viewer mcp [FOLDER] [--show-patient]", t!("cli.usage.mcp")),
        ("mri-viewer list FOLDER", t!("cli.usage.list")),
        ("mri-viewer --help | --version", ""),
    ];
    let mut out = format!("{}\n\n{}\n", t!("cli.usage.title"), t!("cli.usage.heading"));
    for (cmd, desc) in lines {
        let line = format!("  {cmd:<43}{desc}");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn main() -> anyhow::Result<()> {
    // wiersz poleceń działa przed oknem programu – mówi w języku systemu (okno ustawia potem język z ustawień)
    i18n::set_language(None);
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            print!("{}", usage());
            Ok(())
        }
        Some("-V") | Some("--version") => {
            println!("mri-viewer {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("mcp") => {
            let show_patient = args.iter().any(|a| a == "--show-patient");
            let folder = args[1..]
                .iter()
                .find(|a| !a.starts_with("--"))
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("MRI_DIR").map(PathBuf::from));
            mcp::run(folder, !show_patient)
        }
        Some("list") => {
            let folder = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let lib = library::scan(&folder, None);
            for st in &lib.studies {
                println!(
                    "{}",
                    t!(
                        "cli.list.study",
                        date = library::fmt_date(&st.date),
                        description = st.description,
                        patient = st.patient_name
                    )
                );
                for se in &st.series {
                    println!("  [{}] {}", se.id, se.title());
                }
            }
            println!("{}", t!("cli.list.skipped", count = lib.skipped_files));
            Ok(())
        }
        other => {
            // macOS przy pierwszym uruchomieniu potrafi dodać argument -psn_…
            let folder = other.filter(|a| !a.starts_with("-psn")).map(PathBuf::from);
            gui::run(folder)
        }
    }
}
