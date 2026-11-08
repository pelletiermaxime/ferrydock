#[cfg(target_os = "linux")]
mod cosmic;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    cosmic::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("FerryDock's COSMIC window helper requires Linux");
    std::process::ExitCode::FAILURE
}
