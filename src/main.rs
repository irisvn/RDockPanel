fn main() -> std::process::ExitCode {
    // A template's generator, isolated in this process (the panel starts it:
    // `services/template_engine.rs`); never a console command.
    if std::env::args().nth(1).as_deref() == Some(rdockpanel::services::template_engine::CHILD_ARG)
    {
        return rdockpanel::services::template_engine::child_main();
    }
    // The panel's log kept in memory too (Settings → Panel → Logs), before
    // RustCMS installs its own (it then leaves this one).
    if let Ok(config) = rustcms::Config::load() {
        rdockpanel::services::panel_log::init(&config.log.filter, config.log.json);
    }
    rustcms::Console::<rdockpanel::database::migrations::Migrator>::new(
        rdockpanel::bootstrap::build,
    )
    .run_blocking()
}
