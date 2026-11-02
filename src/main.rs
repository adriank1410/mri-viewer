// W wersji wydawniczej na Windows nie otwieramy okna konsoli.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod cache;
mod gui;
mod imaging;
mod library;
mod mcp;
mod mpr;

use std::path::PathBuf;

const USAGE: &str = "\
Przeglądarka MRI – prosty podgląd badań DICOM (narzędzie poglądowe, nie diagnostyczne)

Użycie:
  mri-viewer [FOLDER]                        otwiera okno programu (opcjonalnie od razu z folderem)
  mri-viewer mcp [FOLDER] [--show-patient]   uruchamia serwer MCP dla Claude (stdio)
  mri-viewer list FOLDER                     wypisuje znalezione badania i serie
  mri-viewer --help | --version
";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") => {
            print!("{USAGE}");
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
                    "Badanie {} {} ({})",
                    library::fmt_date(&st.date),
                    st.description,
                    st.patient_name
                );
                for se in &st.series {
                    println!("  [{}] {}", se.id, se.title());
                }
            }
            println!("Pominięte pliki: {}", lib.skipped_files);
            Ok(())
        }
        other => {
            // macOS przy pierwszym uruchomieniu potrafi dodać argument -psn_…
            let folder = other.filter(|a| !a.starts_with("-psn")).map(PathBuf::from);
            gui::run(folder)
        }
    }
}
