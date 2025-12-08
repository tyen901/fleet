#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    velopack::VelopackApp::build().run();

    if let Err(err) = fleet_ui::run() {
        eprintln!("Fleet failed: {err}");
        std::process::exit(1);
    }
}
