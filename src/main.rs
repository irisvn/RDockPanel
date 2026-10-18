fn main() -> std::process::ExitCode {
    rustcms::Console::<rdockpanel::database::migrations::Migrator>::new(
        rdockpanel::bootstrap::build,
    )
    .run_blocking()
}
