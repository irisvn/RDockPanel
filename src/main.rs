fn main() -> std::process::ExitCode {
    rustcms::Console::<rust_panel::database::migrations::Migrator>::new(
        rust_panel::bootstrap::build,
    )
    .run_blocking()
}
