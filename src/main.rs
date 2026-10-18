fn main() -> std::process::ExitCode {
    // A template's generator, isolated in this process (the panel starts it:
    // `services/template_engine.rs`); never a console command.
    if std::env::args().nth(1).as_deref() == Some(rdockpanel::services::template_engine::CHILD_ARG)
    {
        return rdockpanel::services::template_engine::child_main();
    }
    rustcms::Console::<rdockpanel::database::migrations::Migrator>::new(
        rdockpanel::bootstrap::build,
    )
    .run_blocking()
}
