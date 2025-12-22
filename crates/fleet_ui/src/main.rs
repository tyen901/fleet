fn main() -> eframe::Result<()> {
    // Must be the first thing to run; it may restart/exit the process for install/update tasks.
    velopack::VelopackApp::build().run();

    fleet_ui::run()
}
