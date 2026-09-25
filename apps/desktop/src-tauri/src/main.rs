#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

fn main() {
    if std::env::args().any(|argument| argument == "--pmc-build-metadata") {
        println!(
            "{{\"commit\":\"{}\",\"dirty\":\"{}\"}}",
            env!("PMC_BUILD_COMMIT"),
            env!("PMC_BUILD_DIRTY")
        );
        return;
    }
    pmc_desktop_lib::run(pmc_desktop_lib::workspace_from_args(
        std::env::args().skip(1),
    ));
}
